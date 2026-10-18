//! The certificate store: PEM certificates and keys the panel's inbounds
//! present. Uploaded by hand for now; issued and renewed by the panel in
//! phase 10. Keys never leave the server once stored.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};

use crate::auth::CurrentAdmin;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;

#[derive(Serialize)]
pub struct CertificateView {
    id: i64,
    name: String,
    domains: Vec<String>,
    source: String,
    not_before: i64,
    not_after: i64,
    created_at: i64,
}

impl From<z_db::Certificate> for CertificateView {
    fn from(c: z_db::Certificate) -> Self {
        Self {
            id: c.id,
            name: c.name,
            domains: c
                .domains
                .split(',')
                .filter(|d| !d.is_empty())
                .map(str::to_string)
                .collect(),
            source: c.source,
            not_before: c.not_before,
            not_after: c.not_after,
            created_at: c.created_at,
        }
    }
}

#[derive(Deserialize)]
pub struct CertificateInput {
    #[serde(default)]
    name: String,
    cert_pem: String,
    key_pem: String,
}

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

pub async fn list(
    State(state): State<AppState>,
    _: CurrentAdmin,
) -> ApiResult<Json<Vec<CertificateView>>> {
    let all = state.db().certificates().await?;
    Ok(Json(all.into_iter().map(CertificateView::from).collect()))
}

pub async fn create(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Json(input): Json<CertificateInput>,
) -> ApiResult<(StatusCode, Json<CertificateView>)> {
    let parsed = inspect(&input.cert_pem, &input.key_pem)?;
    let name = match input.name.trim() {
        "" => parsed
            .domains
            .first()
            .cloned()
            .unwrap_or_else(|| "certificate".into()),
        name => name.chars().take(128).collect(),
    };
    let certificate = state
        .db()
        .create_certificate(
            &name,
            &parsed.domains.join(","),
            input.cert_pem.trim(),
            input.key_pem.trim(),
            "upload",
            parsed.not_before,
            parsed.not_after,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(certificate.into())))
}

pub async fn delete(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let _guard = state.core_lock().lock().await;
    // An inbound that presents it would no longer build.
    for inbound in state.db().inbounds().await? {
        let stream: serde_json::Value = serde_json::from_str(&inbound.stream).unwrap_or_default();
        if stream["tlsSettings"]["certificateId"].as_i64() == Some(id) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "certificate_in_use",
                format!("inbound {:?} uses this certificate", inbound.remark),
            ));
        }
    }
    if state.db().delete_certificate(id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "there is no such certificate",
        ))
    }
}
