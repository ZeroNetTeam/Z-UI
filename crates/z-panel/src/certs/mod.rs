//! The certificate store's own work: getting certificates from an authority
//! over ACME (HTTP, TLS or DNS challenges), making self-signed ones,
//! renewing before they expire, and putting a renewed one in force where it
//! is used (the panel's HTTPS, the inbounds in the core) without a restart.

pub mod acme;
mod client;
mod cloudflare;
pub mod settings;
mod solve;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

use self::acme::{Method, Name};
use crate::error::ApiError;
use crate::notify::Event;
use crate::server::AppState;

/// What a certificate says about itself, once its key is checked to match.
pub struct Parsed {
    pub domains: Vec<String>,
    pub not_before: i64,
    pub not_after: i64,
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::bad_request("invalid_certificate", message)
}

/// Parse a PEM chain and key, check they belong together, and read the
/// leaf's names and validity.
pub fn inspect(cert_pem: &str, key_pem: &str) -> Result<Parsed, ApiError> {
    let chain = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid("the certificate is not PEM"))?;
    let leaf = chain
        .first()
        .ok_or_else(|| invalid("no certificate in the PEM text"))?;
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .map_err(|_| invalid("the private key is not PEM"))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    rustls::sign::CertifiedKey::from_der(chain.clone(), key, &provider)
        .map_err(|_| invalid("the private key does not belong to this certificate"))?;

    let (_, certificate) = x509_parser::parse_x509_certificate(leaf.as_ref())
        .map_err(|_| invalid("the certificate cannot be read"))?;
    let mut domains: Vec<String> = Vec::new();
    if let Ok(Some(names)) = certificate.subject_alternative_name() {
        for name in &names.value.general_names {
            match name {
                x509_parser::extensions::GeneralName::DNSName(dns) => domains.push(dns.to_string()),
                x509_parser::extensions::GeneralName::IPAddress(bytes) => {
                    if let Ok(v4) = <[u8; 4]>::try_from(*bytes) {
                        domains.push(std::net::IpAddr::from(v4).to_string());
                    } else if let Ok(v6) = <[u8; 16]>::try_from(*bytes) {
                        domains.push(std::net::IpAddr::from(v6).to_string());
                    }
                }
                _ => {}
            }
        }
    }
    if domains.is_empty() {
        if let Some(cn) = certificate.subject().iter_common_name().next() {
            if let Ok(cn) = cn.as_str() {
                domains.push(cn.to_string());
            }
        }
    }
    let validity = certificate.validity();
    Ok(Parsed {
        domains,
        not_before: validity.not_before.timestamp(),
        not_after: validity.not_after.timestamp(),
    })
}

/// When a certificate should be renewed: `renew_days` before it expires, but
/// no earlier than a third of its life before (a short-lived certificate
/// lives for days, not months).
pub fn renew_at(not_before: i64, not_after: i64, renew_days: u32) -> i64 {
    let lead = (i64::from(renew_days) * 86_400).min((not_after - not_before) / 3);
    not_after - lead
}

/// How long to leave a certificate whose renewal failed before trying again,
/// so a broken setup does not wear the authority's limits down.
const RETRY_AFTER: i64 = 6 * 3600;
/// How often the renewal loop looks.
const LOOP: Duration = Duration::from_secs(3600);
/// An expiring certificate that is not renewed is said so this many days out.
pub const WARN_DAYS: i64 = 14;

/// What an admin asks for.
pub struct IssueRequest {
    pub name: String,
    pub names: Vec<String>,
    pub method: Method,
    pub auto_renew: bool,
}

/// Get a certificate and add it to the store. The work is done in a task of
/// its own: an admin who closes the page does not leave a challenge
/// half-answered.
pub async fn issue(state: &AppState, request: IssueRequest) -> Result<z_db::Certificate, ApiError> {
    let state = state.clone();
    tokio::spawn(async move { issue_now(&state, request).await })
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
}

async fn issue_now(state: &AppState, request: IssueRequest) -> Result<z_db::Certificate, ApiError> {
    let names = acme::parse_names(&request.names)?;
    let _one_at_a_time = state.cert_lock().lock().await;
    let settings = settings::load(state.db()).await;
    let got = acme::obtain(state, &settings, &names, request.method).await?;
    let parsed = inspect(&got.chain_pem, &got.key_pem).map_err(|e| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "ca_failed",
            format!("what the authority sent is unusable: {}", e.message),
        )
    })?;
    let label = match request.name.trim() {
        "" => names[0].text(),
        name => name.chars().take(128).collect(),
    };
    let certificate = state
        .db()
        .add_certificate(z_db::NewCertificate {
            name: label,
            domains: parsed.domains.join(","),
            cert_pem: got.chain_pem.trim().to_string(),
            key_pem: got.key_pem.trim().to_string(),
            source: "acme".into(),
            not_before: parsed.not_before,
            not_after: parsed.not_after,
            auto_renew: request.auto_renew,
            method: got.method.label().into(),
            ca: settings.ca_label(),
        })
        .await?;
    Ok(certificate)
}

/// Renew certificate `id` with the same names and challenge, in place: the
/// inbounds that use it, and the panel if it serves with it, follow.
pub async fn renew(state: &AppState, id: i64) -> Result<z_db::Certificate, ApiError> {
    let state = state.clone();
    tokio::spawn(async move { renew_now(&state, id).await })
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
}

async fn renew_now(state: &AppState, id: i64) -> Result<z_db::Certificate, ApiError> {
    let _one_at_a_time = state.cert_lock().lock().await;
    let found = state.db().certificate(id).await?.ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "there is no such certificate",
        )
    })?;
    if found.source != "acme" {
        return Err(ApiError::bad_request(
            "not_renewable",
            "only a certificate the panel got from an authority can be renewed here; upload a new one in its place",
        ));
    }
    let outcome = async {
        let names = acme::parse_names(std::slice::from_ref(&found.domains))?;
        let settings = settings::load(state.db()).await;
        let got = acme::obtain(state, &settings, &names, Method::from_label(&found.method)).await?;
        let parsed = inspect(&got.chain_pem, &got.key_pem).map_err(|e| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "ca_failed",
                format!("what the authority sent is unusable: {}", e.message),
            )
        })?;
        state
            .db()
            .renew_certificate(
                id,
                got.chain_pem.trim(),
                got.key_pem.trim(),
                parsed.not_before,
                parsed.not_after,
            )
            .await?;
        Ok::<_, ApiError>(())
    }
    .await;
    match outcome {
        Ok(()) => {
            apply(state, id).await;
            Ok(state.db().certificate(id).await?.unwrap_or(found))
        }
        Err(error) => {
            let _ = state.db().certificate_failed(id, &error.message).await;
            Err(error)
        }
    }
}

/// A self-signed certificate for `names`, for panel or inbound use where no
/// authority's will do (an address nobody validates, a private network).
pub fn self_signed(names: &[Name]) -> Result<(String, String), ApiError> {
    let texts: Vec<String> = names.iter().map(Name::text).collect();
    let mut params = rcgen::CertificateParams::new(texts)
        .map_err(|e| ApiError::bad_request("invalid_names", e.to_string()))?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, names[0].text());
    params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(5 * 365);
    let key = rcgen::KeyPair::generate()
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let certificate = params
        .self_signed(&key)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    Ok((certificate.pem(), key.serialize_pem()))
}

/// Put certificate `id` in force where it is used, now: the panel's HTTPS
/// takes the next handshake with it, and the core is reconfigured if an
/// inbound presents it.
pub async fn apply(state: &AppState, id: i64) {
    if state.tls() && state.config().cert.is_none() && state.panel_certificate() == Some(id) {
        if let Some(resolver) = state.tls_resolver() {
            match crate::tls::stored_certificate(state.db(), id).await {
                Ok(key) => {
                    resolver.set(key);
                    tracing::info!(id, "the panel now serves its renewed certificate");
                }
                Err(error) => {
                    tracing::warn!(%error, "the renewed certificate cannot be served by the panel")
                }
            }
        }
    }
    let used = state
        .db()
        .inbounds()
        .await
        .unwrap_or_default()
        .iter()
        .any(|inbound| {
            let stream: serde_json::Value =
                serde_json::from_str(&inbound.stream).unwrap_or_default();
            inbound.enabled && stream["tlsSettings"]["certificateId"].as_i64() == Some(id)
        });
    if used {
        match crate::core_config::sync(state).await {
            Ok(_) => tracing::info!(id, "the core now presents the renewed certificate"),
            Err(error) => {
                tracing::warn!(error = %error.message, "the core did not take the renewed certificate")
            }
        }
    }
}

/// One look at every certificate: renew what is due, and tell what needs
/// telling. Returns what was done, for a test to read.
pub async fn renew_due(state: &AppState) -> Vec<(i64, Result<(), String>)> {
    let now = z_db::now();
    let settings = settings::load(state.db()).await;
    let mut done = Vec::new();
    let certificates = state.db().certificates().await.unwrap_or_default();
    for certificate in certificates {
        let left_days = (certificate.not_after - now) / 86_400;
        let due = certificate.auto_renew
            && certificate.source == "acme"
            && now
                >= renew_at(
                    certificate.not_before,
                    certificate.not_after,
                    settings.renew_days,
                )
            && (certificate.last_error.is_empty()
                || now - certificate.last_attempt_at >= RETRY_AFTER);
        if due {
            let outcome = renew(state, certificate.id).await;
            match &outcome {
                Ok(renewed) => state.notify(Event::CertificateRenewed {
                    name: renewed.name.clone(),
                    days: (renewed.not_after - z_db::now()) / 86_400,
                }),
                Err(error) => {
                    let day = now / 86_400;
                    if state
                        .db()
                        .mark_notified(&format!("cert_failed:{}:{day}", certificate.id))
                        .await
                        .unwrap_or(false)
                    {
                        state.notify(Event::CertificateFailed {
                            name: certificate.name.clone(),
                            error: error.message.clone(),
                        });
                    }
                }
            }
            done.push((certificate.id, outcome.map(|_| ()).map_err(|e| e.message)));
            continue;
        }
        // Not renewing it itself, and not long to go: say so once.
        let renewing = certificate.auto_renew
            && certificate.source == "acme"
            && certificate.last_error.is_empty();
        if !renewing
            && left_days <= WARN_DAYS
            && state
                .db()
                .mark_notified(&format!(
                    "cert_expiring:{}:{}",
                    certificate.id, certificate.not_after
                ))
                .await
                .unwrap_or(false)
        {
            state.notify(Event::CertificateExpiring {
                name: certificate.name.clone(),
                days: left_days.max(0),
            });
        }
    }
    done
}

/// Look for certificates to renew, now and then.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        // Not at the very start: the panel and the core settle first.
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            for (id, outcome) in renew_due(&state).await {
                match outcome {
                    Ok(()) => tracing::info!(id, "a certificate was renewed"),
                    Err(error) => tracing::warn!(id, %error, "a certificate could not be renewed"),
                }
            }
            tokio::time::sleep(LOOP).await;
        }
    });
}
