//! The core's configuration, built from what the panel stores.
//!
//! How this works. Every change to inbounds goes through [`apply`]: the
//! inbounds as they would be after the change are built into a whole core
//! configuration, checked (ports, certificates, and the core's own compiler),
//! and handed to the node. Only when the node takes it does the caller write
//! the change to the database, so the database never holds an inbound the
//! core refused. Changes run one at a time under the state's core lock.
//!
//! What is stored is Xray's shape plus a few keys only the panel reads (the
//! certificate an inbound uses, the REALITY public key that links need);
//! those are resolved or removed here, so the core sees plain configuration.

use std::collections::{HashMap, HashSet};

use axum::http::StatusCode;
use serde_json::{json, Map, Value};
use z_common::node::Applied;
use z_db::{Certificate, Inbound};

use crate::error::ApiError;
use crate::server::AppState;

/// Protocols the panel builds inbounds for, and the settings key their
/// accounts go under.
pub const PROTOCOLS: &[(&str, &str)] = &[
    ("vless", "clients"),
    ("vmess", "clients"),
    ("trojan", "clients"),
    ("shadowsocks", "clients"),
    ("hysteria2", "users"),
    ("tuic", "users"),
    ("anytls", "users"),
    ("wireguard", "peers"),
];

/// Keys in `settings` only the panel reads.
const PANEL_SETTINGS: &[&str] = &["flow", "publicKey"];
/// Keys in `realitySettings` that describe the client side, kept for links.
const PANEL_REALITY: &[&str] = &["publicKey", "mldsa65Verify", "fingerprint", "spiderX"];

pub fn accounts_key(protocol: &str) -> Option<&'static str> {
    PROTOCOLS
        .iter()
        .find(|(name, _)| *name == protocol)
        .map(|(_, key)| *key)
}

/// Whether `inbound` listens on UDP rather than TCP.
pub fn uses_udp(protocol: &str, stream: &Value) -> bool {
    matches!(protocol, "hysteria2" | "tuic" | "wireguard")
        || (stream["network"] == "xhttp"
            && matches!(
                stream["xhttpSettings"]["httpVersion"].as_str(),
                Some("h3" | "3")
            ))
}

fn parse(text: &str, what: &str, tag: &str) -> Result<Value, ApiError> {
    serde_json::from_str(text).map_err(|error| {
        ApiError::bad_request(
            "invalid_inbound",
            format!("inbound {tag:?}: {what} is not JSON: {error}"),
        )
    })
}

/// Whether this machine can reach the IPv6 internet: a UDP socket "connects"
/// only when a route exists, and nothing is sent. Asked once.
fn has_ipv6() -> bool {
    static IPV6: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *IPV6.get_or_init(|| {
        std::net::UdpSocket::bind("[::]:0")
            .and_then(|socket| socket.connect("[2001:4860:4860::8888]:53"))
            .is_ok()
    })
}

/// The everything-but-inbounds part of the configuration. Phase 7 makes it
/// editable; for now a server that sends its clients out directly and never
/// into the server's own network.
fn template() -> Value {
    json!({
        "log": {"loglevel": "warning"},
        // The machine's own resolver, as the core's installer sets a server
        // up: the provider's DNS is the one always reachable from its
        // network. Without a resolver no domain a client asks for resolves.
        "dns": {
            "servers": ["localhost"],
            "leakPolicy": "fallback",
            "queryStrategy": if has_ipv6() { "UseIP" } else { "UseIPv4" },
        },
        "outbounds": [
            {"tag": "direct", "protocol": "freedom"},
            {"tag": "blocked", "protocol": "blackhole"},
        ],
        "routing": {"rules": [{
            "type": "field",
            "ip": [
                "0.0.0.0/8", "10.0.0.0/8", "100.64.0.0/10", "127.0.0.0/8",
                "169.254.0.0/16", "172.16.0.0/12", "192.168.0.0/16",
                "::1/128", "fc00::/7", "fe80::/10",
            ],
            "outboundTag": "blocked",
        }]},
    })
}

/// The core's view of one stored inbound: panel keys resolved or removed,
/// and an empty account list (users arrive in phase 4).
fn core_inbound(
    inbound: &Inbound,
    certificates: &HashMap<i64, Certificate>,
) -> Result<Value, ApiError> {
    let tag = inbound.tag.as_str();
    let mut settings = parse(&inbound.settings, "settings", tag)?;
    let mut stream = parse(&inbound.stream, "stream", tag)?;
    let sniffing = parse(&inbound.sniffing, "sniffing", tag)?;

    let settings_map = settings.as_object_mut().ok_or_else(|| {
        ApiError::bad_request(
            "invalid_inbound",
            format!("inbound {tag:?}: settings is not an object"),
        )
    })?;
    for key in PANEL_SETTINGS {
        settings_map.remove(*key);
    }
    if let Some(key) = accounts_key(&inbound.protocol) {
        settings_map.insert(key.into(), json!([]));
    }

    if let Some(stream) = stream.as_object_mut() {
        if let Some(reality) = stream
            .get_mut("realitySettings")
            .and_then(Value::as_object_mut)
        {
            for key in PANEL_REALITY {
                reality.remove(*key);
            }
        }
        if let Some(tls) = stream.get_mut("tlsSettings").and_then(Value::as_object_mut) {
            if let Some(id) = tls.remove("certificateId") {
                let id = id.as_i64().unwrap_or_default();
                let certificate = certificates.get(&id).ok_or_else(|| {
                    ApiError::bad_request(
                        "certificate_missing",
                        format!("inbound {tag:?} uses certificate {id}, which does not exist"),
                    )
                })?;
                tls.insert(
                    "certificates".into(),
                    json!([{"certificate": [certificate.cert_pem], "key": [certificate.key_pem]}]),
                );
            }
        }
    }

    let mut out = Map::new();
    out.insert("tag".into(), json!(inbound.tag));
    if !inbound.listen.is_empty() {
        out.insert("listen".into(), json!(inbound.listen));
    }
    out.insert("port".into(), json!(inbound.port));
    out.insert("protocol".into(), json!(inbound.protocol));
    out.insert("settings".into(), settings);
    out.insert("streamSettings".into(), stream);
    if sniffing.is_object() {
        out.insert("sniffing".into(), sniffing);
    }
    Ok(Value::Object(out))
}

/// The whole configuration for `inbounds` (the enabled ones).
pub fn build(
    inbounds: &[Inbound],
    certificates: &HashMap<i64, Certificate>,
) -> Result<Value, ApiError> {
    let mut config = template();
    let built = inbounds
        .iter()
        .filter(|inbound| inbound.enabled)
        .map(|inbound| core_inbound(inbound, certificates))
        .collect::<Result<Vec<_>, _>>()?;
    config["inbounds"] = Value::Array(built);
    Ok(config)
}

/// Refuse two enabled inbounds on one port (of the same kind), or one on
/// the panel's own port.
pub fn check_ports(inbounds: &[Inbound], panel_port: u16) -> Result<(), ApiError> {
    let mut taken = HashSet::new();
    for inbound in inbounds.iter().filter(|inbound| inbound.enabled) {
        let stream: Value = serde_json::from_str(&inbound.stream).unwrap_or_default();
        let udp = uses_udp(&inbound.protocol, &stream);
        if !udp && inbound.port == i64::from(panel_port) {
            return Err(ApiError::bad_request(
                "port_conflict",
                format!("port {} is the panel's own", inbound.port),
            ));
        }
        // Different listen addresses could share a port, but a wildcard
        // takes every address, so conflicts are judged by port alone.
        if !taken.insert((inbound.port, udp)) {
            return Err(ApiError::bad_request(
                "port_conflict",
                format!(
                    "another inbound already listens on {} port {}",
                    if udp { "UDP" } else { "TCP" },
                    inbound.port
                ),
            ));
        }
    }
    Ok(())
}

/// Build, check and apply the configuration for `inbounds`. The caller
/// holds the core lock and writes its change only on success.
pub async fn apply(state: &AppState, inbounds: &[Inbound]) -> Result<Applied, ApiError> {
    check_ports(inbounds, state.config().listen.port())?;
    let certificates: HashMap<i64, Certificate> = state
        .db()
        .certificates()
        .await?
        .into_iter()
        .map(|certificate| (certificate.id, certificate))
        .collect();
    let config = build(inbounds, &certificates)?;
    // The core's own compiler is the judge of what it can run.
    zero_config::compile_config(&config, zero_core::GenerationId(1))
        .map_err(|error| ApiError::bad_request("invalid_inbound", error))?;
    state
        .node()
        .apply(config)
        .await
        .map_err(|error| ApiError::new(StatusCode::BAD_REQUEST, "core_refused", error.to_string()))
}

/// Bring the core in line with the database, at start and after anything
/// went wrong half-way.
pub async fn sync(state: &AppState) -> Result<Applied, ApiError> {
    let _guard = state.core_lock().lock().await;
    let inbounds = state.db().inbounds().await?;
    apply(state, &inbounds).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbound(tag: &str, port: i64, protocol: &str, settings: Value, stream: Value) -> Inbound {
        Inbound {
            id: 1,
            tag: tag.into(),
            remark: tag.into(),
            enabled: true,
            listen: String::new(),
            port,
            protocol: protocol.into(),
            settings: settings.to_string(),
            stream: stream.to_string(),
            sniffing: json!({"enabled": true, "destOverride": ["http", "tls"]}).to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn panel_keys_are_resolved_or_removed() {
        let certificate = Certificate {
            id: 7,
            name: "c".into(),
            domains: "example.com".into(),
            cert_pem: "CERT PEM".into(),
            key_pem: "KEY PEM".into(),
            source: "upload".into(),
            not_before: 0,
            not_after: 0,
            created_at: 0,
        };
        let certificates = HashMap::from([(7, certificate)]);
        let reality = inbound(
            "r",
            443,
            "vless",
            json!({"decryption": "none", "flow": "xtls-rprx-vision"}),
            json!({"network": "raw", "security": "reality", "realitySettings": {
                "target": "example.com:443", "privateKey": "p", "publicKey": "P", "fingerprint": "chrome",
            }}),
        );
        let tls = inbound(
            "t",
            8443,
            "trojan",
            json!({}),
            json!({"network": "ws", "security": "tls", "tlsSettings": {"certificateId": 7}}),
        );
        let mut off = inbound("off", 9000, "vmess", json!({}), json!({}));
        off.enabled = false;
        let config = build(&[reality, tls, off], &certificates).unwrap();
        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 2, "a disabled inbound is left out");
        assert_eq!(
            inbounds[0]["settings"],
            json!({"decryption": "none", "clients": []})
        );
        let reality = &inbounds[0]["streamSettings"]["realitySettings"];
        assert!(reality.get("publicKey").is_none() && reality.get("fingerprint").is_none());
        assert_eq!(reality["privateKey"], "p");
        assert_eq!(
            inbounds[1]["streamSettings"]["tlsSettings"],
            json!({"certificates": [{"certificate": ["CERT PEM"], "key": ["KEY PEM"]}]})
        );
        assert!(inbounds[0].get("listen").is_none());
        assert_eq!(config["outbounds"][0]["protocol"], "freedom");
        assert_eq!(config["dns"]["servers"], json!(["localhost"]));
    }

    #[test]
    fn a_missing_certificate_is_named() {
        let tls = inbound(
            "t",
            8443,
            "trojan",
            json!({}),
            json!({"security": "tls", "tlsSettings": {"certificateId": 3}}),
        );
        let error = build(&[tls], &HashMap::new()).unwrap_err();
        assert_eq!(error.code, "certificate_missing");
    }

    #[test]
    fn ports_are_shared_only_across_tcp_and_udp() {
        let a = inbound("a", 443, "vless", json!({}), json!({}));
        let b = inbound("b", 443, "hysteria2", json!({}), json!({}));
        assert!(
            check_ports(&[a.clone(), b], 2053).is_ok(),
            "TCP and UDP 443"
        );
        let c = inbound("c", 443, "trojan", json!({}), json!({}));
        assert_eq!(
            check_ports(&[a.clone(), c], 2053).unwrap_err().code,
            "port_conflict"
        );
        assert_eq!(check_ports(&[a], 443).unwrap_err().code, "port_conflict");
        let mut d = inbound("d", 443, "trojan", json!({}), json!({}));
        d.enabled = false;
        let a = inbound("a", 443, "vless", json!({}), json!({}));
        assert!(
            check_ports(&[a, d], 2053).is_ok(),
            "a disabled one holds nothing"
        );
    }

    #[test]
    fn a_built_reality_inbound_compiles_in_the_core() {
        let pair = zero_runtime::keys::x25519();
        let reality = inbound(
            "r",
            443,
            "vless",
            json!({"decryption": "none"}),
            json!({"network": "raw", "security": "reality", "realitySettings": {
                "target": "www.example.com:443", "serverNames": ["www.example.com"],
                "privateKey": pair.private_key, "publicKey": pair.public_key,
                "shortIds": [zero_runtime::keys::short_id(8).unwrap()],
            }}),
        );
        let config = build(&[reality], &HashMap::new()).unwrap();
        zero_config::compile_config(&config, zero_core::GenerationId(1)).unwrap();
    }
}
