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
use std::path::Path;

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
pub(crate) fn has_ipv6() -> bool {
    static IPV6: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *IPV6.get_or_init(|| {
        std::net::UdpSocket::bind("[::]:0")
            .and_then(|socket| socket.connect("[2001:4860:4860::8888]:53"))
            .is_ok()
    })
}

/// The core's view of one stored inbound: panel keys resolved or removed,
/// and `clients` as its accounts.
fn core_inbound(
    inbound: &Inbound,
    certificates: &HashMap<i64, Certificate>,
    clients: Vec<Value>,
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
        // WireGuard peers are not made from users yet: keep those written.
        if inbound.protocol != "wireguard" || !settings_map.contains_key(key) {
            settings_map.insert(key.into(), Value::Array(clients));
        }
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

/// The whole configuration for `inbounds` (the enabled ones), with the
/// accounts of `users` their `groups` give them, and the rest from `settings`
/// ([`crate::core_settings`]).
pub fn build(
    inbounds: &[Inbound],
    certificates: &HashMap<i64, Certificate>,
    users: &[z_db::User],
    groups: &[z_db::AccessGroup],
    settings: &Value,
    data_dir: &Path,
) -> Result<Value, ApiError> {
    let built = inbounds
        .iter()
        .filter(|inbound| inbound.enabled)
        .map(|inbound| {
            let clients = crate::accounts::clients_of(inbound, users, groups);
            core_inbound(inbound, certificates, clients)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(crate::core_settings::assemble(settings, built, data_dir))
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

/// What the compiler noted about a configuration that it accepted anyway:
/// a setting it reads and does not act on, a resolver it could not parse.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Warning {
    pub path: String,
    pub message: String,
}

/// The configuration for `inbounds` and `settings`, built and compiled by the
/// core's own compiler, with what the compiler had to say about it.
pub async fn compile(
    state: &AppState,
    inbounds: &[Inbound],
    settings: &Value,
) -> Result<(Value, Vec<Warning>), ApiError> {
    check_ports(inbounds, state.config().listen.port())?;
    crate::core_settings::check(settings)
        .map_err(|message| ApiError::bad_request("invalid_core", message))?;
    let certificates: HashMap<i64, Certificate> = state
        .db()
        .certificates()
        .await?
        .into_iter()
        .map(|certificate| (certificate.id, certificate))
        .collect();
    let users = state.db().all_users().await?;
    let groups = state.db().access_groups().await?;
    let config = build(
        inbounds,
        &certificates,
        &users,
        &groups,
        settings,
        &state.config().data_dir,
    )?;
    // The core's own compiler is the judge of what it can run.
    let (generation, notes) = zero_config::compile_config(&config, zero_core::GenerationId(1))
        .map_err(|error| ApiError::bad_request("invalid_core", error))?;
    let mut warnings: Vec<Warning> = notes
        .diagnostics
        .into_iter()
        .map(|d| Warning {
            path: d.path,
            message: d.message,
        })
        .collect();
    warnings.extend(dns_warnings(settings, &generation.config));
    Ok((config, warnings))
}

/// What the core does with DNS that its compiler does not say: any outbound
/// that sends traffic through another server makes the core refuse this
/// machine's own resolver, whatever `leakPolicy` asks for, because on a client
/// that resolver would answer outside the tunnel. On a panel's server that is
/// a trap: with only `localhost` to ask, no name resolves, for direct
/// connections too.
fn dns_warnings(settings: &Value, config: &zero_config::RuntimeConfig) -> Vec<Warning> {
    use zero_config::dns::{LeakPolicy, ResolverEndpoint};
    let mut warnings = Vec::new();
    let strict = config.dns.leak_policy == LeakPolicy::Strict;
    if strict && settings["dns"]["leakPolicy"] == "fallback" {
        warnings.push(Warning {
            path: "dns.leakPolicy".into(),
            message: "ignored: an outbound sends traffic through another server, and then the \
                      core never uses this machine's own resolver"
                .into(),
        });
    }
    let only_system = config
        .dns
        .servers
        .iter()
        .all(|server| matches!(server.endpoint, ResolverEndpoint::System));
    if strict && only_system {
        warnings.push(Warning {
            path: "dns.servers".into(),
            message: "no usable resolver: names will not resolve, for direct connections \
                      either. Add an encrypted or plain resolver by address (a DoH, DoT or UDP \
                      server), or use the Iran preset"
                .into(),
        });
    }
    warnings
}

/// Build, check and apply the configuration for `inbounds` and `settings`.
/// The caller holds the core lock and writes its change only on success.
pub async fn apply_with(
    state: &AppState,
    inbounds: &[Inbound],
    settings: &Value,
) -> Result<(Applied, Vec<Warning>), ApiError> {
    let (config, warnings) = compile(state, inbounds, settings).await?;
    let applied = state.node().apply(config).await.map_err(|error| {
        ApiError::new(StatusCode::BAD_REQUEST, "core_refused", error.to_string())
    })?;
    Ok((applied, warnings))
}

/// [`apply_with`] the stored settings: what an inbound or user change does.
pub async fn apply(state: &AppState, inbounds: &[Inbound]) -> Result<Applied, ApiError> {
    let settings = crate::core_settings::load(state.db()).await;
    let (config, _) = compile(state, inbounds, &settings).await.map_err(|error| {
        // A change to an inbound is refused for what is wrong with the inbound.
        if error.code == "invalid_core" {
            ApiError::bad_request("invalid_inbound", error.message)
        } else {
            error
        }
    })?;
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
    let applied = apply(state, &inbounds).await?;
    crate::accounts::apply_limits(state).await?;
    Ok(applied)
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
        let config = build(
            &[reality, tls, off],
            &certificates,
            &[],
            &[],
            &crate::core_settings::default_document(),
            Path::new("data"),
        )
        .unwrap();
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
        let error = build(
            &[tls],
            &HashMap::new(),
            &[],
            &[],
            &crate::core_settings::default_document(),
            Path::new("data"),
        )
        .unwrap_err();
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
        let config = build(
            &[reality],
            &HashMap::new(),
            &[],
            &[],
            &crate::core_settings::default_document(),
            Path::new("data"),
        )
        .unwrap();
        zero_config::compile_config(&config, zero_core::GenerationId(1)).unwrap();
    }
}
