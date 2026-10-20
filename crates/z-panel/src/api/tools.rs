//! Helpers for building inbounds.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::Query;
use axum::Json;
use rustls_pki_types::ServerName;
use serde::{Deserialize, Serialize};

use crate::auth::Access;
use crate::error::{ApiError, ApiResult};

const TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Deserialize)]
pub struct RealityQuery {
    /// `example.com` or `example.com:443`.
    target: String,
}

/// Whether a site can be a REALITY target: what a good one offers.
#[derive(Debug, Serialize)]
pub struct RealityCheck {
    target: String,
    /// It answered TLS 1.3, which REALITY needs.
    tls13: bool,
    /// It chose HTTP/2 from ALPN, as a real browser's site would.
    h2: bool,
    /// The key exchange was X25519, the group REALITY's clients offer.
    x25519: bool,
    /// Its certificate is valid for the name.
    certificate_valid: bool,
    /// Time to finish the TLS handshake.
    latency_ms: Option<u64>,
    error: Option<String>,
    /// All of the above.
    suitable: bool,
}

/// Connect to `target` as a browser would and report how it answered.
pub async fn reality_check(
    _: Access,
    Query(query): Query<RealityQuery>,
) -> ApiResult<Json<RealityCheck>> {
    let target = query.target.trim().to_ascii_lowercase();
    let (host, port) = match target.rsplit_once(':') {
        Some((host, port)) => (
            host.to_string(),
            port.parse::<u16>()
                .map_err(|_| ApiError::bad_request("invalid_target", "the port is not a number"))?,
        ),
        None => (target.clone(), 443),
    };
    if host.is_empty() || host.parse::<IpAddr>().is_ok() {
        return Err(ApiError::bad_request(
            "invalid_target",
            "a REALITY target is a domain name",
        ));
    }
    let mut report = RealityCheck {
        target: format!("{host}:{port}"),
        tls13: false,
        h2: false,
        x25519: false,
        certificate_valid: false,
        latency_ms: None,
        error: None,
        suitable: false,
    };
    match probe(&host, port).await {
        Ok((tls13, h2, x25519, elapsed)) => {
            report.tls13 = tls13;
            report.h2 = h2;
            report.x25519 = x25519;
            report.certificate_valid = true;
            report.latency_ms = Some(elapsed.as_millis() as u64);
        }
        Err(error) => report.error = Some(error),
    }
    report.suitable = report.tls13 && report.h2 && report.x25519 && report.certificate_valid;
    Ok(Json(report))
}

async fn probe(host: &str, port: u16) -> Result<(bool, bool, bool, Duration), String> {
    let addresses: Vec<_> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| format!("the name does not resolve: {error}"))?
        .collect();
    // A target inside the server's own network is never a disguise, and
    // probing one from the panel would reach places it should not.
    let address = addresses
        .iter()
        .find(|address| public(address.ip()))
        .ok_or("the name resolves only to private addresses")?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| error.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let name = ServerName::try_from(host.to_string()).map_err(|_| "not a valid name")?;
    let started = Instant::now();
    let connect = async {
        let tcp = tokio::net::TcpStream::connect(address)
            .await
            .map_err(|error| format!("cannot connect: {error}"))?;
        tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, tcp)
            .await
            .map_err(|error| format!("TLS 1.3 handshake failed: {error}"))
    };
    let stream = tokio::time::timeout(TIMEOUT, connect)
        .await
        .map_err(|_| "no answer in time".to_string())??;
    let elapsed = started.elapsed();
    let (_, connection) = stream.get_ref();
    let tls13 = connection.protocol_version() == Some(rustls::ProtocolVersion::TLSv1_3);
    let h2 = connection.alpn_protocol() == Some(b"h2");
    let x25519 = connection
        .negotiated_key_exchange_group()
        .is_some_and(|group| group.name() == rustls::NamedGroup::X25519);
    Ok((tls13, h2, x25519, elapsed))
}

fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64)
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80)
        }
    }
}

#[derive(Deserialize)]
pub struct QrQuery {
    text: String,
}

#[derive(Serialize)]
pub struct Qr {
    svg: String,
}

/// A QR code of `text` (a config link), as SVG, for the dashboard to show.
pub async fn qr(_: Access, Query(query): Query<QrQuery>) -> ApiResult<Json<Qr>> {
    if query.text.is_empty() || query.text.len() > 2048 {
        return Err(ApiError::bad_request(
            "invalid_text",
            "1 to 2048 bytes of text",
        ));
    }
    Ok(Json(Qr {
        svg: crate::subscription::page::qr(&query.text),
    }))
}
