//! The endpoints of a subscription in each format a client reads:
//!  * share links (`vless://…`), one per line, which most clients take
//!    base64-encoded;
//!  * Xray JSON, a whole client configuration per endpoint (what v2rayN and
//!    v2rayNG import as "custom configs"), which zray reads as well;
//!  * sing-box JSON, one configuration with every endpoint and a selector;
//!  * Clash Meta (mihomo) YAML, the same for Clash clients.
//!
//! A combination a format cannot express (XHTTP in sing-box or Clash, the
//! QUIC protocols in Xray) is left out of that format rather than written
//! wrong.

use std::collections::HashSet;

use base64::Engine as _;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde_json::{json, Map, Value};

use super::endpoint::{Endpoint, Protocol, Security, Transport};

/// Everything but unreserved characters, for query values and fragments.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn enc(text: &str) -> String {
    utf8_percent_encode(text, COMPONENT).to_string()
}

/// An IPv6 address needs brackets before a port.
fn authority(address: &str, port: u16) -> String {
    if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

// ---- Share links ----

/// The share link of `e`.
pub fn link(e: &Endpoint) -> String {
    let mut query: Vec<(&str, String)> = Vec::new();
    let mut push = |key: &'static str, value: &str| {
        if !value.is_empty() {
            query.push((key, value.to_string()));
        }
    };
    let stream_query = |push: &mut dyn FnMut(&'static str, &str)| {
        push(
            "type",
            match &e.transport {
                Transport::Raw => "tcp",
                other => other.name(),
            },
        );
        match &e.transport {
            Transport::Raw => {}
            Transport::Ws { path, host } | Transport::HttpUpgrade { path, host } => {
                push("path", path);
                push("host", host);
            }
            Transport::Grpc { service } => {
                push("serviceName", service);
                push("mode", "gun");
            }
            Transport::Xhttp { path, host, mode } => {
                push("path", path);
                push("host", host);
                push("mode", mode);
            }
        }
        push("security", e.security.name());
        tls_query(&e.security, push);
    };
    let fragment = enc(&e.remark);
    let at = authority(&e.address, e.port);
    match &e.protocol {
        Protocol::Vless { id, flow } => {
            push("encryption", "none");
            push("flow", flow);
            stream_query(&mut push);
            format!("vless://{id}@{at}?{}#{fragment}", query_text(&query))
        }
        Protocol::Trojan { password } => {
            stream_query(&mut push);
            format!(
                "trojan://{}@{at}?{}#{fragment}",
                enc(password),
                query_text(&query)
            )
        }
        Protocol::Vmess { id } => {
            let (path, host) = match &e.transport {
                Transport::Ws { path, host }
                | Transport::HttpUpgrade { path, host }
                | Transport::Xhttp { path, host, .. } => (path.clone(), host.clone()),
                Transport::Grpc { service } => (service.clone(), String::new()),
                Transport::Raw => (String::new(), String::new()),
            };
            let (sni, alpn, fp) = match &e.security {
                Security::Tls {
                    sni,
                    alpn,
                    fingerprint,
                    ..
                } => (sni.clone(), alpn.join(","), fingerprint.clone()),
                _ => Default::default(),
            };
            let body = json!({
                "v": "2", "ps": e.remark, "add": e.address, "port": e.port.to_string(), "id": id,
                "aid": "0", "scy": "auto",
                "net": match &e.transport { Transport::Raw => "tcp", other => other.name() },
                "type": if matches!(e.transport, Transport::Grpc { .. }) { "gun" } else { "none" },
                "host": host, "path": path,
                "tls": if matches!(e.security, Security::Tls { .. }) { "tls" } else { "" },
                "sni": sni, "alpn": alpn, "fp": fp,
            });
            format!(
                "vmess://{}",
                base64::engine::general_purpose::STANDARD.encode(body.to_string())
            )
        }
        Protocol::Shadowsocks { method, password } => {
            let userinfo = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(format!("{method}:{password}"));
            format!("ss://{userinfo}@{at}#{fragment}")
        }
        Protocol::Hysteria2 { password, obfs } => {
            tls_query(&e.security, &mut push);
            if let Some(obfs) = obfs {
                push("obfs", "salamander");
                push("obfs-password", obfs);
            }
            format!(
                "hysteria2://{}@{at}?{}#{fragment}",
                enc(password),
                query_text(&query)
            )
        }
        Protocol::Tuic { id, password } => {
            tls_query(&e.security, &mut push);
            push("congestion_control", "bbr");
            push("udp_relay_mode", "native");
            format!(
                "tuic://{id}:{}@{at}?{}#{fragment}",
                enc(password),
                query_text(&query)
            )
        }
        Protocol::AnyTls { password } => {
            push("security", e.security.name());
            tls_query(&e.security, &mut push);
            format!(
                "anytls://{}@{at}?{}#{fragment}",
                enc(password),
                query_text(&query)
            )
        }
    }
}

fn tls_query(security: &Security, push: &mut dyn FnMut(&'static str, &str)) {
    match security {
        Security::None => {}
        Security::Tls {
            sni,
            alpn,
            fingerprint,
            insecure,
        } => {
            push("sni", sni);
            push("alpn", &alpn.join(","));
            push("fp", fingerprint);
            if *insecure {
                push("allowInsecure", "1");
                push("insecure", "1");
            }
        }
        Security::Reality {
            sni,
            fingerprint,
            public_key,
            short_id,
            spider_x,
            mldsa65_verify,
        } => {
            push("sni", sni);
            push("fp", fingerprint);
            push("pbk", public_key);
            push("sid", short_id);
            push("spx", spider_x);
            push("pqv", mldsa65_verify);
        }
    }
}

fn query_text(query: &[(&str, String)]) -> String {
    query
        .iter()
        .map(|(key, value)| format!("{key}={}", enc(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Links, one per line, base64-encoded the way subscription clients expect.
pub fn base64_links(endpoints: &[Endpoint]) -> String {
    let text = endpoints.iter().map(link).collect::<Vec<_>>().join("\n");
    base64::engine::general_purpose::STANDARD.encode(text)
}

// ---- Xray / zray JSON ----

fn xray_stream(e: &Endpoint) -> Value {
    let mut stream = Map::new();
    stream.insert("network".into(), json!(e.transport.name()));
    match &e.transport {
        Transport::Raw => {}
        Transport::Ws { path, host } => {
            let mut ws = json!({"path": path});
            if !host.is_empty() {
                ws["headers"] = json!({"Host": host});
            }
            stream.insert("wsSettings".into(), ws);
        }
        Transport::Grpc { service } => {
            stream.insert("grpcSettings".into(), json!({"serviceName": service}));
        }
        Transport::HttpUpgrade { path, host } => {
            stream.insert(
                "httpupgradeSettings".into(),
                json!({"path": path, "host": host}),
            );
        }
        Transport::Xhttp { path, host, mode } => {
            stream.insert(
                "xhttpSettings".into(),
                json!({"path": path, "host": host, "mode": mode}),
            );
        }
    }
    stream.insert("security".into(), json!(e.security.name()));
    match &e.security {
        Security::None => {}
        Security::Tls {
            sni,
            alpn,
            fingerprint,
            insecure,
        } => {
            let mut tls = json!({"serverName": sni, "allowInsecure": insecure});
            if !alpn.is_empty() {
                tls["alpn"] = json!(alpn);
            }
            if !fingerprint.is_empty() {
                tls["fingerprint"] = json!(fingerprint);
            }
            stream.insert("tlsSettings".into(), tls);
        }
        Security::Reality {
            sni,
            fingerprint,
            public_key,
            short_id,
            spider_x,
            mldsa65_verify,
        } => {
            let mut reality = json!({
                "serverName": sni, "fingerprint": fingerprint, "publicKey": public_key,
                "shortId": short_id, "spiderX": spider_x,
            });
            if !mldsa65_verify.is_empty() {
                reality["mldsa65Verify"] = json!(mldsa65_verify);
            }
            stream.insert("realitySettings".into(), reality);
        }
    }
    Value::Object(stream)
}

/// `e` as an outbound tagged `tag`. With `zray`, the protocols only zray
/// speaks (Hysteria2, TUIC, AnyTLS) are written too; Xray has none of them.
pub fn xray_outbound(e: &Endpoint, tag: &str, zray: bool) -> Option<Value> {
    let (protocol, settings) = match &e.protocol {
        Protocol::Vless { id, flow } => {
            let mut user = json!({"id": id, "encryption": "none"});
            if !flow.is_empty() {
                user["flow"] = json!(flow);
            }
            (
                "vless",
                json!({"vnext": [{"address": e.address, "port": e.port, "users": [user]}]}),
            )
        }
        Protocol::Vmess { id } => (
            "vmess",
            json!({"vnext": [{"address": e.address, "port": e.port,
                "users": [{"id": id, "alterId": 0, "security": "auto"}]}]}),
        ),
        Protocol::Trojan { password } => (
            "trojan",
            json!({"servers": [{"address": e.address, "port": e.port, "password": password}]}),
        ),
        Protocol::Shadowsocks { method, password } => (
            "shadowsocks",
            json!({"servers": [{"address": e.address, "port": e.port, "method": method, "password": password}]}),
        ),
        _ if !zray => return None,
        Protocol::Hysteria2 { password, obfs } => {
            let mut settings =
                json!({"servers": [{"address": e.address, "port": e.port, "password": password}]});
            if let Some(obfs) = obfs {
                settings["obfs"] = json!({"type": "salamander", "password": obfs});
            }
            ("hysteria2", settings)
        }
        Protocol::Tuic { id, password } => (
            "tuic",
            json!({"servers": [{"address": e.address, "port": e.port, "uuid": id, "password": password}]}),
        ),
        Protocol::AnyTls { password } => (
            "anytls",
            json!({"servers": [{"address": e.address, "port": e.port, "password": password}]}),
        ),
    };
    let mut outbound = json!({"tag": tag, "protocol": protocol, "settings": settings});
    if !matches!(e.protocol, Protocol::Shadowsocks { .. }) {
        outbound["streamSettings"] = xray_stream(e);
    }
    Some(outbound)
}

/// One whole client configuration per endpoint: a local SOCKS and HTTP proxy,
/// the endpoint, and private addresses sent direct.
pub fn xray_configs(endpoints: &[Endpoint], zray: bool) -> Value {
    Value::Array(
        endpoints
            .iter()
            .filter_map(|e| {
                let proxy = xray_outbound(e, "proxy", zray)?;
                Some(json!({
                    "remarks": e.remark,
                    "log": {"loglevel": "warning"},
                    "inbounds": [
                        {"tag": "socks", "listen": "127.0.0.1", "port": 10808, "protocol": "socks",
                            "settings": {"udp": true},
                            "sniffing": {"enabled": true, "destOverride": ["http", "tls", "quic"]}},
                        {"tag": "http", "listen": "127.0.0.1", "port": 10809, "protocol": "http"},
                    ],
                    "outbounds": [
                        proxy,
                        {"tag": "direct", "protocol": "freedom"},
                        {"tag": "block", "protocol": "blackhole"},
                    ],
                    "routing": {"domainStrategy": "AsIs", "rules": [
                        {"type": "field", "ip": ["geoip:private"], "outboundTag": "direct"},
                    ]},
                }))
            })
            .collect(),
    )
}

// ---- Names ----

/// `name`, or `name 2`, `name 3`… when it is taken.
fn unique(name: &str, taken: &mut HashSet<String>) -> String {
    let base = if name.trim().is_empty() {
        "proxy"
    } else {
        name.trim()
    };
    let mut candidate = base.to_string();
    let mut n = 2;
    while !taken.insert(candidate.clone()) {
        candidate = format!("{base} {n}");
        n += 1;
    }
    candidate
}

// ---- sing-box ----

fn singbox_tls(security: &Security) -> Option<Value> {
    match security {
        Security::None => None,
        Security::Tls {
            sni,
            alpn,
            fingerprint,
            insecure,
        } => {
            let mut tls = json!({"enabled": true, "server_name": sni, "insecure": insecure});
            if !alpn.is_empty() {
                tls["alpn"] = json!(alpn);
            }
            if !fingerprint.is_empty() {
                tls["utls"] = json!({"enabled": true, "fingerprint": fingerprint});
            }
            Some(tls)
        }
        Security::Reality {
            sni,
            fingerprint,
            public_key,
            short_id,
            ..
        } => Some(json!({
            "enabled": true, "server_name": sni,
            "utls": {"enabled": true, "fingerprint": fingerprint},
            "reality": {"enabled": true, "public_key": public_key, "short_id": short_id},
        })),
    }
}

/// `e` as a sing-box outbound, or `None` when sing-box cannot carry it.
pub fn singbox_outbound(e: &Endpoint, tag: &str) -> Option<Value> {
    let transport = match &e.transport {
        Transport::Raw => None,
        Transport::Ws { path, host } => {
            let mut ws = json!({"type": "ws", "path": path});
            if !host.is_empty() {
                ws["headers"] = json!({"Host": host});
            }
            Some(ws)
        }
        Transport::Grpc { service } => Some(json!({"type": "grpc", "service_name": service})),
        Transport::HttpUpgrade { path, host } => {
            Some(json!({"type": "httpupgrade", "path": path, "host": host}))
        }
        Transport::Xhttp { .. } => return None,
    };
    let mut outbound = match &e.protocol {
        Protocol::Vless { id, flow } => json!({"type": "vless", "uuid": id, "flow": flow}),
        Protocol::Vmess { id } => {
            json!({"type": "vmess", "uuid": id, "security": "auto", "alter_id": 0})
        }
        Protocol::Trojan { password } => json!({"type": "trojan", "password": password}),
        Protocol::Shadowsocks { method, password } => {
            json!({"type": "shadowsocks", "method": method, "password": password})
        }
        Protocol::Hysteria2 { password, obfs } => {
            let mut hy = json!({"type": "hysteria2", "password": password});
            if let Some(obfs) = obfs {
                hy["obfs"] = json!({"type": "salamander", "password": obfs});
            }
            hy
        }
        Protocol::Tuic { id, password } => json!({
            "type": "tuic", "uuid": id, "password": password,
            "congestion_control": "bbr", "udp_relay_mode": "native",
        }),
        Protocol::AnyTls { password } => json!({"type": "anytls", "password": password}),
    };
    outbound["tag"] = json!(tag);
    outbound["server"] = json!(e.address);
    outbound["server_port"] = json!(e.port);
    if let Some(tls) = singbox_tls(&e.security) {
        outbound["tls"] = tls;
    }
    if let Some(transport) = transport {
        outbound["transport"] = transport;
    }
    Some(outbound)
}

/// One sing-box configuration (1.11 or later): a TUN and a local mixed
/// proxy, every endpoint, a selector over them with an automatic choice
/// first, and private addresses sent direct.
pub fn singbox_config(endpoints: &[Endpoint]) -> Value {
    let mut taken: HashSet<String> = ["proxy", "auto", "direct"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let proxies: Vec<Value> = endpoints
        .iter()
        .filter_map(|e| {
            let tag = unique(&e.remark, &mut taken);
            singbox_outbound(e, &tag)
        })
        .collect();
    let tags: Vec<Value> = proxies.iter().map(|p| p["tag"].clone()).collect();
    let mut outbounds = vec![
        json!({"type": "selector", "tag": "proxy",
            "outbounds": std::iter::once(json!("auto")).chain(tags.iter().cloned()).collect::<Vec<_>>()}),
        json!({"type": "urltest", "tag": "auto", "outbounds": tags,
            "url": "https://www.gstatic.com/generate_204", "interval": "5m"}),
    ];
    outbounds.extend(proxies);
    outbounds.push(json!({"type": "direct", "tag": "direct"}));
    json!({
        "log": {"level": "warn"},
        "dns": {
            "servers": [
                {"tag": "remote", "type": "https", "server": "1.1.1.1", "detour": "proxy"},
                {"tag": "local", "type": "local"},
            ],
            "final": "remote",
        },
        "inbounds": [
            {"type": "tun", "tag": "tun-in", "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
                "auto_route": true, "strict_route": true},
            {"type": "mixed", "tag": "mixed-in", "listen": "127.0.0.1", "listen_port": 2080},
        ],
        "outbounds": outbounds,
        "route": {
            "rules": [
                {"action": "sniff"},
                {"protocol": "dns", "action": "hijack-dns"},
                {"ip_is_private": true, "outbound": "direct"},
            ],
            "final": "proxy",
            "auto_detect_interface": true,
            "default_domain_resolver": "local",
        },
    })
}

// ---- Clash Meta ----

/// `e` as a Clash Meta (mihomo) proxy, or `None` when Clash cannot carry it.
pub fn clash_proxy(e: &Endpoint, name: &str) -> Option<Value> {
    let mut proxy = match &e.protocol {
        Protocol::Vless { id, flow } => {
            let mut p = json!({"type": "vless", "uuid": id});
            if !flow.is_empty() {
                p["flow"] = json!(flow);
            }
            p
        }
        Protocol::Vmess { id } => {
            json!({"type": "vmess", "uuid": id, "alterId": 0, "cipher": "auto"})
        }
        Protocol::Trojan { password } => json!({"type": "trojan", "password": password}),
        Protocol::Shadowsocks { method, password } => {
            json!({"type": "ss", "cipher": method, "password": password})
        }
        Protocol::Hysteria2 { password, obfs } => {
            let mut p = json!({"type": "hysteria2", "password": password});
            if let Some(obfs) = obfs {
                p["obfs"] = json!("salamander");
                p["obfs-password"] = json!(obfs);
            }
            p
        }
        Protocol::Tuic { id, password } => json!({
            "type": "tuic", "uuid": id, "password": password,
            "congestion-controller": "bbr", "udp-relay-mode": "native",
        }),
        Protocol::AnyTls { password } => json!({"type": "anytls", "password": password}),
    };
    proxy["name"] = json!(name);
    proxy["server"] = json!(e.address);
    proxy["port"] = json!(e.port);
    proxy["udp"] = json!(true);
    match &e.transport {
        Transport::Raw => {}
        Transport::Ws { path, host } => {
            proxy["network"] = json!("ws");
            let mut opts = json!({"path": path});
            if !host.is_empty() {
                opts["headers"] = json!({"Host": host});
            }
            proxy["ws-opts"] = opts;
        }
        Transport::HttpUpgrade { path, host } => {
            proxy["network"] = json!("ws");
            let mut opts = json!({"path": path, "v2ray-http-upgrade": true});
            if !host.is_empty() {
                opts["headers"] = json!({"Host": host});
            }
            proxy["ws-opts"] = opts;
        }
        Transport::Grpc { service } => {
            proxy["network"] = json!("grpc");
            proxy["grpc-opts"] = json!({"grpc-service-name": service});
        }
        Transport::Xhttp { .. } => return None,
    }
    let quic = matches!(
        e.protocol,
        Protocol::Hysteria2 { .. } | Protocol::Tuic { .. }
    );
    let sni_key = if matches!(e.protocol, Protocol::Vless { .. } | Protocol::Vmess { .. }) {
        "servername"
    } else {
        "sni"
    };
    match &e.security {
        Security::None => {}
        Security::Tls {
            sni,
            alpn,
            fingerprint,
            insecure,
        } => {
            if !quic
                && !matches!(
                    e.protocol,
                    Protocol::Trojan { .. } | Protocol::AnyTls { .. }
                )
            {
                proxy["tls"] = json!(true);
            }
            if !sni.is_empty() {
                proxy[sni_key] = json!(sni);
            }
            if !alpn.is_empty() {
                proxy["alpn"] = json!(alpn);
            }
            if !fingerprint.is_empty() {
                proxy["client-fingerprint"] = json!(fingerprint);
            }
            if *insecure {
                proxy["skip-cert-verify"] = json!(true);
            }
        }
        Security::Reality {
            sni,
            fingerprint,
            public_key,
            short_id,
            ..
        } => {
            if !matches!(e.protocol, Protocol::Trojan { .. }) {
                proxy["tls"] = json!(true);
            }
            proxy[sni_key] = json!(sni);
            proxy["client-fingerprint"] = json!(fingerprint);
            proxy["reality-opts"] = json!({"public-key": public_key, "short-id": short_id});
        }
    }
    Some(proxy)
}

/// One Clash Meta configuration: a local mixed port, every endpoint, a
/// selector over them with an automatic choice first, and private addresses
/// sent direct.
pub fn clash_config(endpoints: &[Endpoint]) -> String {
    let mut taken: HashSet<String> = ["Proxy", "Auto", "DIRECT", "REJECT"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let proxies: Vec<Value> = endpoints
        .iter()
        .filter_map(|e| {
            let name = unique(&e.remark, &mut taken);
            clash_proxy(e, &name)
        })
        .collect();
    let names: Vec<Value> = proxies.iter().map(|p| p["name"].clone()).collect();
    let config = json!({
        "mixed-port": 7890,
        "allow-lan": false,
        "mode": "rule",
        "log-level": "warning",
        "ipv6": true,
        "proxies": proxies,
        "proxy-groups": [
            {"name": "Proxy", "type": "select",
                "proxies": std::iter::once(json!("Auto")).chain(names.iter().cloned()).collect::<Vec<_>>()},
            {"name": "Auto", "type": "url-test", "proxies": names,
                "url": "https://www.gstatic.com/generate_204", "interval": 300},
        ],
        "rules": [
            "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
            "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve",
            "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve",
            "IP-CIDR,127.0.0.0/8,DIRECT,no-resolve",
            "MATCH,Proxy",
        ],
    });
    yaml(&config)
}

/// `value` as block YAML. Every string is double-quoted with JSON's escapes,
/// which YAML reads the same, so no value can turn into another type or
/// break the structure.
pub fn yaml(value: &Value) -> String {
    fn scalar(value: &Value) -> String {
        match value {
            Value::String(s) => Value::String(s.clone()).to_string(),
            other => other.to_string(),
        }
    }
    fn is_block(value: &Value) -> bool {
        matches!(value, Value::Object(map) if !map.is_empty())
            || matches!(value, Value::Array(list) if !list.is_empty())
    }
    fn write(value: &Value, indent: usize, out: &mut String) {
        let pad = " ".repeat(indent);
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let key = Value::String(key.clone()).to_string();
                    if is_block(child) {
                        out.push_str(&format!("{pad}{key}:\n"));
                        write(child, indent + 2, out);
                    } else {
                        out.push_str(&format!("{pad}{key}: {}\n", empty_or_scalar(child)));
                    }
                }
            }
            Value::Array(list) => {
                for child in list {
                    if is_block(child) {
                        // The first line of the child after "- ", the rest
                        // indented under it.
                        let mut inner = String::new();
                        write(child, indent + 2, &mut inner);
                        let inner = inner.trim_start();
                        out.push_str(&format!("{pad}- {inner}"));
                    } else {
                        out.push_str(&format!("{pad}- {}\n", empty_or_scalar(child)));
                    }
                }
            }
            other => out.push_str(&format!("{pad}{}\n", scalar(other))),
        }
    }
    fn empty_or_scalar(value: &Value) -> String {
        match value {
            Value::Object(_) => "{}".into(),
            Value::Array(_) => "[]".into(),
            other => scalar(other),
        }
    }
    let mut out = String::new();
    write(value, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "5783a3e7-e373-51cd-8642-c83782b807c5";

    fn tls() -> Security {
        Security::Tls {
            sni: "vpn.example.org".into(),
            alpn: vec!["h2".into(), "http/1.1".into()],
            fingerprint: "chrome".into(),
            insecure: false,
        }
    }

    fn reality() -> Security {
        Security::Reality {
            sni: "www.example.com".into(),
            fingerprint: "chrome".into(),
            public_key: "6X6TqIqO7Hh4kq-OdBzc7r8xbvxz3e0iZ8LgXzn8WlU".into(),
            short_id: "ab12cd34".into(),
            spider_x: "/".into(),
            mldsa65_verify: String::new(),
        }
    }

    fn endpoint(
        remark: &str,
        protocol: Protocol,
        transport: Transport,
        security: Security,
    ) -> Endpoint {
        Endpoint {
            remark: remark.into(),
            address: "203.0.113.9".into(),
            port: 443,
            protocol,
            transport,
            security,
        }
    }

    /// One of each protocol, over the transports and security each takes.
    fn every_kind() -> Vec<Endpoint> {
        let vless = |flow: &str| Protocol::Vless {
            id: UUID.into(),
            flow: flow.into(),
        };
        let ws = || Transport::Ws {
            path: "/ws path".into(),
            host: "cdn.example.org".into(),
        };
        vec![
            endpoint(
                "vless reality",
                vless("xtls-rprx-vision"),
                Transport::Raw,
                reality(),
            ),
            endpoint("vless ws tls", vless(""), ws(), tls()),
            endpoint(
                "vless grpc",
                vless(""),
                Transport::Grpc {
                    service: "svc".into(),
                },
                tls(),
            ),
            endpoint(
                "vless xhttp",
                vless(""),
                Transport::Xhttp {
                    path: "/x".into(),
                    host: String::new(),
                    mode: "auto".into(),
                },
                reality(),
            ),
            endpoint(
                "vless upgrade",
                vless(""),
                Transport::HttpUpgrade {
                    path: "/u".into(),
                    host: "h.example.org".into(),
                },
                Security::None,
            ),
            endpoint("vmess ws", Protocol::Vmess { id: UUID.into() }, ws(), tls()),
            endpoint(
                "trojan",
                Protocol::Trojan {
                    password: "p@ss w0rd".into(),
                },
                Transport::Raw,
                tls(),
            ),
            endpoint(
                "ss 2022",
                Protocol::Shadowsocks {
                    method: "2022-blake3-aes-128-gcm".into(),
                    password: "MDEyMzQ1Njc4OWFiY2RlZg==:YWxpY2UtYWxpY2UtYWxpYw==".into(),
                },
                Transport::Raw,
                Security::None,
            ),
            endpoint(
                "ss legacy",
                Protocol::Shadowsocks {
                    method: "aes-256-gcm".into(),
                    password: "plain".into(),
                },
                Transport::Raw,
                Security::None,
            ),
            endpoint(
                "hysteria2",
                Protocol::Hysteria2 {
                    password: "hy".into(),
                    obfs: Some("squeak".into()),
                },
                Transport::Raw,
                Security::Tls {
                    sni: "vpn.example.org".into(),
                    alpn: vec!["h3".into()],
                    fingerprint: String::new(),
                    insecure: true,
                },
            ),
            endpoint(
                "tuic",
                Protocol::Tuic {
                    id: UUID.into(),
                    password: "tu".into(),
                },
                Transport::Raw,
                Security::Tls {
                    sni: "vpn.example.org".into(),
                    alpn: vec!["h3".into()],
                    fingerprint: String::new(),
                    insecure: false,
                },
            ),
            endpoint(
                "anytls",
                Protocol::AnyTls {
                    password: "any".into(),
                },
                Transport::Raw,
                tls(),
            ),
        ]
    }

    #[test]
    fn every_link_reads_back_in_zray_with_its_name() {
        for e in every_kind() {
            let text = link(&e);
            let parsed = zero_config::share_link::parse_link(&text)
                .unwrap_or_else(|error| panic!("{}: {error}\n{text}", e.remark));
            assert_eq!(parsed.remark, e.remark, "{text}");
        }
        let body = base64_links(&every_kind());
        let back = zero_config::share_link::parse_subscription(&body);
        assert_eq!(back.len(), every_kind().len());
        assert!(back.iter().all(Result::is_ok));
    }

    #[test]
    fn every_json_config_compiles_in_zray() {
        for zray in [false, true] {
            let configs = xray_configs(&every_kind(), zray);
            let configs = configs.as_array().unwrap();
            // Xray has no Hysteria2, TUIC or AnyTLS; zray has them all.
            assert_eq!(configs.len(), if zray { 12 } else { 9 });
            for config in configs {
                zero_config::compile_config(config, zero_core::GenerationId(1))
                    .unwrap_or_else(|error| panic!("{}: {error}\n{config:#}", config["remarks"]));
            }
        }
    }

    #[test]
    fn sing_box_and_clash_take_what_they_can_carry() {
        let mut kinds = every_kind();
        kinds.push(kinds[0].clone()); // the same name twice
        let singbox = singbox_config(&kinds);
        let tags: Vec<&str> = singbox["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["tag"].as_str().unwrap())
            .collect();
        assert!(!tags.contains(&"vless xhttp"), "sing-box has no XHTTP");
        assert!(
            tags.contains(&"vless reality 2"),
            "names are made unique: {tags:?}"
        );
        assert_eq!(singbox["outbounds"][0]["outbounds"][0], "auto");
        let reality = singbox["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["tag"] == "vless reality")
            .unwrap();
        assert_eq!(reality["tls"]["reality"]["short_id"], "ab12cd34");
        assert_eq!(reality["flow"], "xtls-rprx-vision");

        let clash = clash_config(&kinds);
        assert!(clash.contains("\"type\": \"hysteria2\""));
        assert!(clash.contains("\"reality-opts\":"));
        assert!(!clash.contains("vless xhttp"), "Clash has no XHTTP");
        assert!(clash.contains("- \"MATCH,Proxy\""));
    }

    #[test]
    fn yaml_nests_lists_of_maps_and_quotes_strings() {
        let text = yaml(&json!({"a": [{"b": 1, "c": "x: y"}, "z"], "e": [], "f": {"g": true}}));
        assert_eq!(
            text,
            "\"a\":\n  - \"b\": 1\n    \"c\": \"x: y\"\n  - \"z\"\n\"e\": []\n\"f\":\n  \"g\": true\n"
        );
    }
}
