//! The core's configuration apart from its inbounds: outbounds, routing, DNS,
//! balancers, the observatory, geodata, and the rest of what Xray's JSON
//! holds. The inbounds have a page of their own ([`crate::core_config`]);
//! everything else is one document, kept in the settings table, that the Core
//! page edits and the raw JSON editor shows whole.
//!
//! The document is Xray's JSON as the core reads it, plus one key of the
//! panel's own, `blockPrivate`. When it is on (the default) the panel puts a
//! rule first in the routing that sends loopback, private and link-local
//! addresses to a blackhole, so a client cannot reach services on the server's
//! own network that trust it. It is a switch rather than a rule in the list so
//! that editing the rules cannot remove it by accident.
//!
//! A change is built together with the inbounds, compiled by the core's own
//! compiler, applied, and only then stored ([`crate::api::core`]); so what is
//! stored is always what the core accepted.

use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Map, Value};

/// The settings key the document is kept under.
pub const SETTING: &str = "core.settings";

/// The most a document may weigh, in bytes of JSON.
pub const MAX_BYTES: usize = 1024 * 1024;

/// Address ranges a client has no business reaching through the server.
const PRIVATE_RANGES: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "::1/128",
    "fc00::/7",
    "fe80::/10",
];

/// A server that sends its clients out directly and never into its own
/// network: the machine's own resolver (the one a provider's network always
/// reaches), a freedom outbound and a blackhole.
pub fn default_document() -> Value {
    json!({
        "blockPrivate": true,
        "log": {"loglevel": "warning"},
        "dns": {
            "servers": ["localhost"],
            "leakPolicy": "fallback",
            "queryStrategy": if crate::core_config::has_ipv6() { "UseIP" } else { "UseIPv4" },
        },
        "outbounds": [
            {"tag": "direct", "protocol": "freedom"},
            {"tag": "block", "protocol": "blackhole"},
        ],
        "routing": {"rules": []},
    })
}

/// The stored document, or the default when none is stored (or the stored
/// one cannot be read).
pub async fn load(db: &z_db::Database) -> Value {
    match db.setting(SETTING).await {
        Ok(Some(text)) => match serde_json::from_str::<Value>(&text) {
            Ok(document) if document.is_object() => document,
            _ => {
                tracing::warn!(
                    "the stored core settings are not a JSON object; using the defaults"
                );
                default_document()
            }
        },
        _ => default_document(),
    }
}

pub async fn save(db: &z_db::Database, document: &Value) -> Result<(), z_db::DbError> {
    db.set_setting(SETTING, &document.to_string()).await
}

/// Why `document` cannot be a core's settings, or `Ok`. The core's own
/// compiler judges the rest.
pub fn check(document: &Value) -> Result<(), String> {
    let Some(object) = document.as_object() else {
        return Err("the settings must be a JSON object".into());
    };
    if document.to_string().len() > MAX_BYTES {
        return Err("the settings are over 1 MiB".into());
    }
    if object.contains_key("inbounds") {
        return Err("inbounds are managed on the Inbounds page, not here".into());
    }
    if object
        .get("blockPrivate")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err("blockPrivate must be true or false".into());
    }
    match object.get("outbounds") {
        Some(Value::Array(list)) if !list.is_empty() => {
            if list.iter().any(|outbound| !outbound.is_object()) {
                return Err("every outbound must be an object".into());
            }
        }
        _ => {
            return Err(
                "there must be at least one outbound; the first one takes whatever no rule sends elsewhere"
                    .into(),
            )
        }
    }
    Ok(())
}

/// The tag of a blackhole outbound in `outbounds`, adding one when there is
/// none, for the rule that blocks private addresses to name.
fn blackhole_tag(outbounds: &mut Vec<Value>) -> String {
    if let Some(tag) = outbounds
        .iter()
        .find(|outbound| outbound["protocol"] == "blackhole")
        .and_then(|outbound| outbound["tag"].as_str())
        .filter(|tag| !tag.is_empty())
    {
        return tag.to_string();
    }
    let taken = |tag: &str, list: &[Value]| list.iter().any(|o| o["tag"] == tag);
    let tag = ["block", "z-ui-block"]
        .into_iter()
        .find(|tag| !taken(tag, outbounds))
        .unwrap_or("z-ui-block-private");
    outbounds.push(json!({"tag": tag, "protocol": "blackhole"}));
    tag.to_string()
}

/// The core's whole configuration: `document` (without the panel's own key)
/// with `inbounds`, the protective rule first in the routing when asked for,
/// and the geodata directory (inside `data_dir`) filled in where geodata is
/// managed but names no directory.
pub fn assemble(document: &Value, inbounds: Vec<Value>, data_dir: &Path) -> Value {
    let mut config: Map<String, Value> = document.as_object().cloned().unwrap_or_default();
    let block_private = config
        .remove("blockPrivate")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    config.insert("inbounds".into(), Value::Array(inbounds));
    if block_private {
        let mut outbounds = config
            .get("outbounds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let tag = blackhole_tag(&mut outbounds);
        config.insert("outbounds".into(), Value::Array(outbounds));
        let routing = config
            .entry("routing")
            .or_insert_with(|| json!({}))
            .as_object_mut();
        if let Some(routing) = routing {
            let mut rules = vec![json!({
                "type": "field",
                "ip": PRIVATE_RANGES,
                "outboundTag": tag,
            })];
            if let Some(Value::Array(own)) = routing.remove("rules") {
                rules.extend(own);
            }
            routing.insert("rules".into(), Value::Array(rules));
        }
    }
    if let Some(assets) = config.get_mut("assets").and_then(Value::as_object_mut) {
        if !assets.contains_key("directory") {
            let directory = std::path::absolute(data_dir.join("geodata"))
                .unwrap_or_else(|_| data_dir.join("geodata"));
            assets.insert("directory".into(), json!(directory.to_string_lossy()));
        }
    }
    Value::Object(config)
}

/// What the Iran preset is asked for. Every field but `proxies` has a default
/// that fits most servers.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct IranOptions {
    /// Share links of the outbounds the traffic goes out through; the first
    /// takes whatever no rule matches.
    pub proxies: Vec<String>,
    /// `auto`, `cloudflare`, `google`, `quad9` or `adguard`.
    pub remote_dns: String,
    /// `google`, `cloudflare`, `quad9` or `system`.
    pub local_dns: String,
    /// `auto`, `shecan`, `403`, `electro`, `radar`, `begzar`, `none`…
    pub anti_sanction_dns: String,
    pub block_ads: bool,
    /// Split the TLS hello of every proxy outbound.
    pub fragment: bool,
    /// Keep the geosite and geoip lists the rules name up to date.
    pub manage_geodata: bool,
    /// Edge addresses (`ip:port`) to find a clean Cloudflare address among.
    pub clean_ip_candidates: Vec<String>,
    pub clean_ip_host: String,
    /// The admin's own rules, tried before the preset's: Xray-style routing
    /// rule objects.
    #[schema(value_type = Vec<Object>)]
    pub user_rules: Vec<zero_config::UserRule>,
}

impl Default for IranOptions {
    fn default() -> Self {
        Self {
            proxies: Vec::new(),
            remote_dns: "auto".into(),
            local_dns: "google".into(),
            anti_sanction_dns: "auto".into(),
            block_ads: true,
            fragment: false,
            manage_geodata: true,
            clean_ip_candidates: Vec::new(),
            clean_ip_host: "www.speedtest.net".into(),
            user_rules: Vec::new(),
        }
    }
}

/// The document the Iran preset makes of `options`, keeping `current`'s
/// `blockPrivate` and log settings. The preset's own inbounds (a local SOCKS
/// and HTTP proxy for a client) are not taken: a panel's inbounds are its own.
pub fn iran_preset(options: &IranOptions, current: &Value) -> Result<Value, String> {
    let proxies: Vec<&str> = options
        .proxies
        .iter()
        .map(|link| link.trim())
        .filter(|link| !link.is_empty())
        .collect();
    if proxies.is_empty() {
        return Err("give at least one share link for the traffic to go out through".into());
    }
    for link in &proxies {
        zero_config::parse_link(link).map_err(|error| format!("{link}: {error}"))?;
    }
    for rule in &options.user_rules {
        rule.check()?;
    }
    let preset = zero_config::IranPreset {
        outbounds: zero_config::presets::outbounds_from_links(&proxies),
        remote_dns: zero_config::RemoteDns::parse(&options.remote_dns)
            .ok_or_else(|| format!("{:?} is not a remote resolver", options.remote_dns))?,
        local_dns: zero_config::LocalDns::parse(&options.local_dns)
            .ok_or_else(|| format!("{:?} is not a local resolver", options.local_dns))?,
        anti_sanction_dns: zero_config::AntiSanctionDns::parse(&options.anti_sanction_dns)
            .ok_or_else(|| {
                format!(
                    "{:?} is not an anti-sanction resolver",
                    options.anti_sanction_dns
                )
            })?,
        block_ads: options.block_ads,
        fragment: options.fragment,
        manage_assets: options.manage_geodata,
        clean_ip_candidates: options.clean_ip_candidates.clone(),
        clean_ip_host: options.clean_ip_host.clone(),
        user_rules: options.user_rules.clone(),
        ..zero_config::IranPreset::default()
    };
    let Value::Object(mut built) = preset.build() else {
        return Err("the preset did not make a configuration".into());
    };
    built.remove("inbounds");
    let mut document = Map::new();
    document.insert(
        "blockPrivate".into(),
        current.get("blockPrivate").cloned().unwrap_or(json!(true)),
    );
    for (key, value) in built {
        document.insert(key, value);
    }
    if let Some(log) = current.get("log") {
        document.insert("log".into(), log.clone());
    }
    Ok(Value::Object(document))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK: &str = "vless://5783a3e7-e373-51cd-8642-c83782b807c5@203.0.113.9:443?encryption=none&security=reality&sni=www.example.com&fp=chrome&pbk=6X6TqIqO7Hh4kq-OdBzc7r8xbvxz3e0iZ8LgXzn8WlU&sid=ab12cd34&type=tcp&flow=xtls-rprx-vision#out";

    #[test]
    fn the_default_document_compiles_with_an_inbound() {
        let inbound = json!({"tag": "in", "listen": "127.0.0.1", "port": 1, "protocol": "vless",
            "settings": {"clients": [], "decryption": "none"}});
        let config = assemble(&default_document(), vec![inbound], Path::new("data"));
        zero_config::compile_config(&config, zero_core::GenerationId(1)).unwrap();
        assert!(
            config.get("blockPrivate").is_none(),
            "the panel's key never reaches the core"
        );
        assert_eq!(config["routing"]["rules"][0]["outboundTag"], "block");
        assert!(
            config["routing"]["rules"][0]["ip"]
                .as_array()
                .unwrap()
                .len()
                >= 10
        );
    }

    #[test]
    fn the_protective_rule_is_first_and_optional() {
        let mut document = default_document();
        document["routing"]["rules"] =
            json!([{"type": "field", "port": "25", "outboundTag": "block"}]);
        let on = assemble(&document, vec![], Path::new("d"));
        assert_eq!(on["routing"]["rules"].as_array().unwrap().len(), 2);
        assert_eq!(
            on["routing"]["rules"][1]["port"], "25",
            "the admin's rules follow it"
        );
        document["blockPrivate"] = json!(false);
        let off = assemble(&document, vec![], Path::new("d"));
        assert_eq!(off["routing"]["rules"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_blackhole_is_found_or_added_for_the_rule() {
        let only_freedom = json!({"outbounds": [{"tag": "block", "protocol": "freedom"}]});
        let config = assemble(&only_freedom, vec![], Path::new("d"));
        let rule_tag = config["routing"]["rules"][0]["outboundTag"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(rule_tag, "z-ui-block", "a taken name is not reused");
        assert!(config["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["tag"] == "z-ui-block" && o["protocol"] == "blackhole"));
        let named = json!({"outbounds": [{"tag": "drop", "protocol": "blackhole"}, {"tag": "d", "protocol": "freedom"}]});
        let config = assemble(&named, vec![], Path::new("d"));
        assert_eq!(config["routing"]["rules"][0]["outboundTag"], "drop");
        assert_eq!(
            config["outbounds"].as_array().unwrap().len(),
            2,
            "nothing is added when one exists"
        );
    }

    #[test]
    fn the_geodata_directory_is_inside_the_data_directory() {
        let document =
            json!({"outbounds": [{"tag": "d", "protocol": "freedom"}], "assets": {"files": []}});
        let config = assemble(&document, vec![], Path::new("/var/lib/z-ui"));
        assert_eq!(config["assets"]["directory"], "/var/lib/z-ui/geodata");
        let own = json!({"outbounds": [{"tag": "d", "protocol": "freedom"}], "assets": {"directory": "/elsewhere"}});
        assert_eq!(
            assemble(&own, vec![], Path::new("/x"))["assets"]["directory"],
            "/elsewhere"
        );
    }

    #[test]
    fn the_shape_is_checked() {
        assert!(check(&default_document()).is_ok());
        assert!(check(&json!([])).is_err());
        assert!(check(&json!({"inbounds": [], "outbounds": [{"tag": "d"}]}))
            .unwrap_err()
            .contains("Inbounds page"));
        assert!(check(&json!({"outbounds": []})).is_err());
        assert!(check(&json!({"outbounds": ["direct"]})).is_err());
        assert!(check(&json!({"outbounds": [{"tag": "d"}], "blockPrivate": "yes"})).is_err());
    }

    #[test]
    fn the_iran_preset_makes_a_document_the_core_accepts() {
        let options = IranOptions {
            proxies: vec![LINK.into()],
            fragment: true,
            clean_ip_candidates: vec!["104.16.0.1:443".into()],
            ..IranOptions::default()
        };
        let mut current = default_document();
        current["blockPrivate"] = json!(false);
        let document = iran_preset(&options, &current).unwrap();
        assert!(document.get("inbounds").is_none());
        assert_eq!(
            document["blockPrivate"], false,
            "what the admin chose stays"
        );
        assert_eq!(document["outbounds"][0]["tag"], "proxy");
        assert!(document["assets"]["files"].as_array().unwrap().len() == 2);
        assert!(document["observatory"]["cleanIp"].is_object());
        check(&document).unwrap();
        let inbound = json!({"tag": "in", "listen": "127.0.0.1", "port": 1, "protocol": "vless",
            "settings": {"clients": [], "decryption": "none"}});
        let config = assemble(&document, vec![inbound], Path::new("data"));
        let (_, notes) = zero_config::compile_config(&config, zero_core::GenerationId(1)).unwrap();
        assert!(notes.diagnostics.is_empty(), "{:?}", notes.diagnostics);
    }

    #[test]
    fn the_preset_refuses_what_it_cannot_use() {
        let current = default_document();
        let with = |f: &dyn Fn(&mut IranOptions)| {
            let mut options = IranOptions {
                proxies: vec![LINK.into()],
                ..IranOptions::default()
            };
            f(&mut options);
            iran_preset(&options, &current)
        };
        assert!(
            iran_preset(&IranOptions::default(), &current).is_err(),
            "no proxy"
        );
        assert!(with(&|o| o.proxies = vec!["vless://broken".into()]).is_err());
        assert!(with(&|o| o.remote_dns = "nope".into()).is_err());
        assert!(with(&|o| o.user_rules = vec![zero_config::UserRule::default()]).is_err());
        assert!(with(&|_| {}).is_ok());
    }
}
