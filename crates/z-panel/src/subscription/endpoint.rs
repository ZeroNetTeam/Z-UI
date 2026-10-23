//! What a client needs to reach one inbound as one user: the address, the
//! credential, the transport and the security, in one description that every
//! subscription format is rendered from ([`super::formats`]).
//!
//! Where each part comes from:
//!  * the inbound (stored in Xray's shape, with the panel's own keys kept for
//!    this: `flow`, the REALITY `publicKey`, `fingerprint`, `spiderX`);
//!  * a host, which says how clients reach the inbound from outside (another
//!    address or port, a CDN name, another SNI or path); without one, the
//!    server's own address;
//!  * the user's credentials, in the shape [`crate::accounts::client`] gives
//!    the core.

use std::collections::HashMap;
use std::net::IpAddr;

use serde_json::Value;
use z_db::{Certificate, Host, Inbound, User};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// The name shown in the client, before its variables are filled in.
    pub remark: String,
    pub address: String,
    pub port: u16,
    pub protocol: Protocol,
    pub transport: Transport,
    pub security: Security,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protocol {
    Vless {
        id: String,
        flow: String,
    },
    Vmess {
        id: String,
    },
    Trojan {
        password: String,
    },
    /// For a Shadowsocks 2022 inbound with users, `password` is
    /// `serverKey:userKey`.
    Shadowsocks {
        method: String,
        password: String,
    },
    Hysteria2 {
        password: String,
        obfs: Option<String>,
    },
    Tuic {
        id: String,
        password: String,
    },
    AnyTls {
        password: String,
    },
}

impl Protocol {
    pub fn name(&self) -> &'static str {
        match self {
            Protocol::Vless { .. } => "vless",
            Protocol::Vmess { .. } => "vmess",
            Protocol::Trojan { .. } => "trojan",
            Protocol::Shadowsocks { .. } => "shadowsocks",
            Protocol::Hysteria2 { .. } => "hysteria2",
            Protocol::Tuic { .. } => "tuic",
            Protocol::AnyTls { .. } => "anytls",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    Raw,
    Ws {
        path: String,
        host: String,
    },
    Grpc {
        service: String,
    },
    HttpUpgrade {
        path: String,
        host: String,
    },
    Xhttp {
        path: String,
        host: String,
        mode: String,
    },
}

impl Transport {
    pub fn name(&self) -> &'static str {
        match self {
            Transport::Raw => "raw",
            Transport::Ws { .. } => "ws",
            Transport::Grpc { .. } => "grpc",
            Transport::HttpUpgrade { .. } => "httpupgrade",
            Transport::Xhttp { .. } => "xhttp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Security {
    None,
    Tls {
        sni: String,
        alpn: Vec<String>,
        fingerprint: String,
        insecure: bool,
    },
    Reality {
        sni: String,
        fingerprint: String,
        public_key: String,
        short_id: String,
        spider_x: String,
        mldsa65_verify: String,
    },
}

impl Security {
    pub fn name(&self) -> &'static str {
        match self {
            Security::None => "none",
            Security::Tls { .. } => "tls",
            Security::Reality { .. } => "reality",
        }
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

fn first_text(value: &Value, key: &str) -> String {
    value[key]
        .as_array()
        .and_then(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .find(|s| !s.is_empty())
        })
        .unwrap_or_default()
        .to_string()
}

fn or(override_: &str, default: impl Into<String>) -> String {
    if override_.is_empty() {
        default.into()
    } else {
        override_.to_string()
    }
}

/// The endpoints `user` has on `inbound`: one per enabled host, in their
/// order, or one at `server` without hosts. Empty for a protocol whose
/// accounts the panel does not make (WireGuard).
pub fn endpoints(
    inbound: &Inbound,
    hosts: &[&Host],
    user: &User,
    server: &str,
    certificates: &HashMap<i64, Certificate>,
) -> Vec<Endpoint> {
    let Some(client) = crate::accounts::client(inbound, user) else {
        return Vec::new();
    };
    let settings: Value = serde_json::from_str(&inbound.settings).unwrap_or_default();
    let stream: Value = serde_json::from_str(&inbound.stream).unwrap_or_default();
    let protocol = match inbound.protocol.as_str() {
        "vless" => Protocol::Vless {
            id: user.uuid.clone(),
            flow: text(&client, "flow").to_string(),
        },
        "vmess" => Protocol::Vmess {
            id: user.uuid.clone(),
        },
        "trojan" => Protocol::Trojan {
            password: user.password.clone(),
        },
        "shadowsocks" => {
            let method = text(&settings, "method").to_string();
            let user_password = text(&client, "password");
            let password = match text(&settings, "password") {
                server_key if method.starts_with("2022-") && !server_key.is_empty() => {
                    format!("{server_key}:{user_password}")
                }
                _ => user_password.to_string(),
            };
            Protocol::Shadowsocks { method, password }
        }
        "hysteria2" => Protocol::Hysteria2 {
            password: user.password.clone(),
            obfs: Some(text(&settings["obfs"], "password").to_string()).filter(|p| !p.is_empty()),
        },
        "tuic" => Protocol::Tuic {
            id: user.uuid.clone(),
            password: user.password.clone(),
        },
        "anytls" => Protocol::AnyTls {
            password: user.password.clone(),
        },
        _ => return Vec::new(),
    };

    let mut ordered: Vec<&Host> = hosts
        .iter()
        .copied()
        .filter(|host| host.inbound_id == inbound.id && host.fields.enabled)
        .collect();
    ordered.sort_by_key(|host| (host.fields.priority, host.id));
    let plain = Host {
        id: 0,
        inbound_id: inbound.id,
        fields: z_db::HostFields {
            address: server.to_string(),
            enabled: true,
            ..Default::default()
        },
        created_at: 0,
    };
    if ordered.is_empty() {
        ordered.push(&plain);
    }

    ordered
        .into_iter()
        .map(|host| {
            let f = &host.fields;
            let address = or(&f.address, server);
            let transport = transport(&stream, f);
            let security = security(&stream, f, &address, &inbound.protocol, certificates);
            Endpoint {
                remark: or(&f.remark, inbound.remark.clone()),
                port: f
                    .port
                    .filter(|port| (1..=65535).contains(port))
                    .unwrap_or(inbound.port) as u16,
                address,
                protocol: protocol.clone(),
                transport,
                security,
            }
        })
        .collect()
}

fn transport(stream: &Value, host: &z_db::HostFields) -> Transport {
    match text(stream, "network") {
        "ws" => {
            let ws = &stream["wsSettings"];
            Transport::Ws {
                path: or(&host.path, or(text(ws, "path"), "/")),
                host: or(&host.host, text(&ws["headers"], "Host")),
            }
        }
        "grpc" => Transport::Grpc {
            service: or(&host.path, text(&stream["grpcSettings"], "serviceName")),
        },
        "httpupgrade" => {
            let upgrade = &stream["httpupgradeSettings"];
            Transport::HttpUpgrade {
                path: or(&host.path, or(text(upgrade, "path"), "/")),
                host: or(&host.host, text(upgrade, "host")),
            }
        }
        "xhttp" => {
            let xhttp = &stream["xhttpSettings"];
            Transport::Xhttp {
                path: or(&host.path, or(text(xhttp, "path"), "/")),
                host: or(&host.host, text(xhttp, "host")),
                mode: or(text(xhttp, "mode"), "auto"),
            }
        }
        _ => Transport::Raw,
    }
}

fn security(
    stream: &Value,
    host: &z_db::HostFields,
    address: &str,
    protocol: &str,
    certificates: &HashMap<i64, Certificate>,
) -> Security {
    let quic = matches!(protocol, "hysteria2" | "tuic");
    match text(stream, "security") {
        "reality" => {
            let reality = &stream["realitySettings"];
            Security::Reality {
                sni: or(&host.sni, first_text(reality, "serverNames")),
                fingerprint: or(
                    &host.fingerprint,
                    or(text(reality, "fingerprint"), "chrome"),
                ),
                public_key: text(reality, "publicKey").to_string(),
                short_id: first_text(reality, "shortIds"),
                spider_x: text(reality, "spiderX").to_string(),
                mldsa65_verify: text(reality, "mldsa65Verify").to_string(),
            }
        }
        "tls" => {
            let tls = &stream["tlsSettings"];
            // The name the certificate is for, unless the host says
            // otherwise; a bare address has no name to send.
            let certificate_name = tls["certificateId"]
                .as_i64()
                .and_then(|id| certificates.get(&id))
                .and_then(|cert| {
                    cert.domains
                        .split(',')
                        .map(str::trim)
                        .find(|d| !d.is_empty() && !d.starts_with('*'))
                        .map(str::to_string)
                });
            let sni = if !host.sni.is_empty() {
                host.sni.clone()
            } else if let Some(name) = certificate_name {
                name
            } else if address.parse::<IpAddr>().is_err() {
                address.to_string()
            } else {
                String::new()
            };
            let alpn: Vec<String> = if !host.alpn.is_empty() {
                host.alpn
                    .split(',')
                    .map(|a| a.trim().to_string())
                    .filter(|a| !a.is_empty())
                    .collect()
            } else if quic {
                vec!["h3".into()]
            } else {
                tls["alpn"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            Security::Tls {
                sni,
                alpn,
                fingerprint: if quic {
                    String::new()
                } else {
                    or(&host.fingerprint, "chrome")
                },
                insecure: host.allow_insecure,
            }
        }
        _ => Security::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub fn inbound(id: i64, protocol: &str, settings: Value, stream: Value) -> Inbound {
        Inbound {
            id,
            tag: format!("in-{id}"),
            remark: format!("in {id}"),
            enabled: true,
            listen: String::new(),
            port: 443,
            protocol: protocol.into(),
            settings: settings.to_string(),
            stream: stream.to_string(),
            sniffing: "{}".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn host(inbound_id: i64, fields: z_db::HostFields) -> Host {
        Host {
            id: 1,
            inbound_id,
            fields,
            created_at: 0,
        }
    }

    #[test]
    fn a_host_overrides_what_it_names_and_inherits_the_rest() {
        let user = crate::accounts::tests_support::user("alice", "active");
        let vless = inbound(
            1,
            "vless",
            json!({"decryption": "none", "flow": "xtls-rprx-vision"}),
            json!({"network": "raw", "security": "reality", "realitySettings": {
                "serverNames": ["www.example.com"], "publicKey": "PBK", "shortIds": ["", "ab12"],
                "fingerprint": "firefox",
            }}),
        );
        let plain = endpoints(&vless, &[], &user, "203.0.113.5", &HashMap::new());
        assert_eq!(plain.len(), 1);
        assert_eq!(
            (plain[0].address.as_str(), plain[0].port),
            ("203.0.113.5", 443)
        );
        assert_eq!(
            plain[0].protocol,
            Protocol::Vless {
                id: user.uuid.clone(),
                flow: "xtls-rprx-vision".into()
            }
        );
        assert_eq!(
            plain[0].security,
            Security::Reality {
                sni: "www.example.com".into(),
                fingerprint: "firefox".into(),
                public_key: "PBK".into(),
                short_id: "ab12".into(),
                spider_x: String::new(),
                mldsa65_verify: String::new(),
            }
        );
        let cdn = host(
            1,
            z_db::HostFields {
                remark: "via cdn".into(),
                address: "cdn.example.net".into(),
                port: Some(8443),
                sni: "edge.example.net".into(),
                enabled: true,
                ..Default::default()
            },
        );
        let off = host(
            1,
            z_db::HostFields {
                enabled: false,
                ..Default::default()
            },
        );
        let via = endpoints(&vless, &[&cdn, &off], &user, "203.0.113.5", &HashMap::new());
        assert_eq!(via.len(), 1, "a disabled host gives nothing");
        assert_eq!((via[0].remark.as_str(), via[0].port), ("via cdn", 8443));
        assert!(
            matches!(&via[0].security, Security::Reality { sni, .. } if sni == "edge.example.net")
        );
    }

    #[test]
    fn tls_names_the_certificate_and_quic_asks_for_h3() {
        let user = crate::accounts::tests_support::user("bob", "active");
        let mut certificates = HashMap::new();
        certificates.insert(
            7,
            Certificate {
                id: 7,
                name: "c".into(),
                domains: "*.example.org, vpn.example.org".into(),
                cert_pem: String::new(),
                key_pem: String::new(),
                source: "upload".into(),
                not_before: 0,
                not_after: 0,
                created_at: 0,
                ..Default::default()
            },
        );
        let ws = inbound(
            2,
            "trojan",
            json!({}),
            json!({"network": "ws", "security": "tls", "wsSettings": {"path": "/t", "headers": {"Host": "h.example.org"}},
                "tlsSettings": {"certificateId": 7, "alpn": ["http/1.1"]}}),
        );
        let e = &endpoints(&ws, &[], &user, "198.51.100.1", &certificates)[0];
        assert_eq!(
            e.transport,
            Transport::Ws {
                path: "/t".into(),
                host: "h.example.org".into()
            }
        );
        assert_eq!(
            e.security,
            Security::Tls {
                sni: "vpn.example.org".into(),
                alpn: vec!["http/1.1".into()],
                fingerprint: "chrome".into(),
                insecure: false,
            }
        );
        let hy = inbound(
            3,
            "hysteria2",
            json!({"obfs": {"type": "salamander", "password": "squeak"}}),
            json!({"security": "tls"}),
        );
        let e = &endpoints(&hy, &[], &user, "vpn.example.org", &certificates)[0];
        assert_eq!(
            e.protocol,
            Protocol::Hysteria2 {
                password: "pw".into(),
                obfs: Some("squeak".into())
            }
        );
        assert!(
            matches!(&e.security, Security::Tls { alpn, sni, .. } if alpn == &["h3"] && sni == "vpn.example.org")
        );
        assert!(endpoints(
            &inbound(4, "wireguard", json!({}), json!({})),
            &[],
            &user,
            "x",
            &certificates
        )
        .is_empty());
    }
}
