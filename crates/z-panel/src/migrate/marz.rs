//! Reading a PasarGuard database, and a Marzban one (which PasarGuard grew
//! out of): users with their proxies, hosts, templates, the key their
//! subscription links are signed with, and the usage logs.
//!
//! The two differ where PasarGuard moved things into the database. A
//! PasarGuard user has its credentials in `proxy_settings`, belongs to groups
//! that hold inbounds, and the Xray configuration (the inbounds) is a row of
//! `core_configs`. A Marzban user has rows in `proxies`, reaches every
//! inbound of the protocols it has but those excluded, and the Xray
//! configuration is a file, which is given separately.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};
use z_db::{DbError, Record, Source};

use super::common;
use super::model::{
    Kind, Plan, PlannedGroup, PlannedHost, PlannedInbound, PlannedPlan, PlannedTemplate,
    PlannedUser,
};

/// The protocols this panel builds that these have.
const PROTOCOLS: &[&str] = &["vless", "vmess", "trojan", "shadowsocks"];
/// How long usage by the hour is worth keeping (the panel forgets older).
const RETENTION: i64 = 400 * 86_400;

/// Inbounds out of an Xray configuration. `exclude` names tags to leave out.
/// Returns the tag each source tag became.
pub fn inbounds_from_xray(
    plan: &mut Plan,
    config: &Value,
    exclude: &HashSet<String>,
    tags: &mut HashSet<String>,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for entry in config["inbounds"].as_array().cloned().unwrap_or_default() {
        let source_tag = entry["tag"].as_str().unwrap_or_default().to_string();
        let protocol = entry["protocol"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let port = common::number(&entry["port"]).unwrap_or(0);
        let name = if source_tag.is_empty() {
            format!("{protocol}-{port}")
        } else {
            source_tag.clone()
        };
        if exclude.contains(&source_tag) {
            continue;
        }
        if !PROTOCOLS.contains(&protocol.as_str()) {
            // The panel's own API inbound is not the admin's to carry over.
            if !(protocol == "dokodemo-door" && name.eq_ignore_ascii_case("api_inbound")) {
                plan.warn(
                    "inbound",
                    &name,
                    "unsupported_protocol",
                    format!("{protocol} inbounds are not imported (vless, vmess, trojan and shadowsocks are)"),
                );
            }
            continue;
        }
        let mut settings = if entry["settings"].is_object() {
            entry["settings"].clone()
        } else {
            json!({})
        };
        if let Some(map) = settings.as_object_mut() {
            map.remove("clients");
        }
        let mut stream = if entry["streamSettings"].is_object() {
            entry["streamSettings"].clone()
        } else {
            json!({})
        };
        common::lift_reality(&mut stream);
        let certificate = common::certificate_of(&stream);
        let (tag, changed) = common::tag(&name, tags);
        if changed {
            plan.warn("inbound", &name, "renamed", format!("its tag became {tag}"));
        }
        map.insert(source_tag, tag.clone());
        plan.inbounds.push(PlannedInbound {
            tag,
            remark: name.chars().take(128).collect(),
            enabled: true,
            listen: entry["listen"].as_str().unwrap_or_default().to_string(),
            port,
            protocol,
            settings,
            stream,
            sniffing: if entry["sniffing"].is_object() {
                entry["sniffing"].clone()
            } else {
                json!({})
            },
            certificate,
        });
    }
    map
}

/// The flow VLESS users had is the inbound's here: set on the VLESS inbounds
/// it can work on (over TCP, with TLS or REALITY), from the most common flow.
fn apply_flow(plan: &mut Plan, flows: &HashMap<String, usize>) {
    let Some((flow, _)) = flows
        .iter()
        .max_by_key(|(flow, n)| (**n, std::cmp::Reverse((*flow).clone())))
    else {
        return;
    };
    let flow = flow.clone();
    let mut applied = Vec::new();
    for inbound in plan.inbounds.iter_mut().filter(|i| i.protocol == "vless") {
        let tcp = matches!(
            inbound.stream["network"].as_str(),
            None | Some("tcp" | "raw")
        );
        let secure = matches!(inbound.stream["security"].as_str(), Some("tls" | "reality"));
        if tcp && secure {
            inbound.settings["flow"] = json!(flow);
            applied.push(inbound.tag.clone());
        }
    }
    if flows.len() > 1 || !applied.is_empty() {
        plan.warn(
            "source",
            "",
            "flow_for_all",
            format!(
                "users had flows {:?}; {flow} is set on the inbounds it can work on ({}), for everyone on them",
                flows.keys().collect::<Vec<_>>(),
                applied.join(", ")
            ),
        );
    }
}

/// Shadowsocks inbounds need a method of their own; users each had one.
fn apply_methods(plan: &mut Plan, methods: &HashMap<String, usize>) {
    let method = methods
        .iter()
        .max_by_key(|(m, n)| (**n, std::cmp::Reverse((*m).clone())))
        .map(|(m, _)| m.clone())
        .unwrap_or_else(|| "chacha20-ietf-poly1305".into());
    let mut set = Vec::new();
    for inbound in plan
        .inbounds
        .iter_mut()
        .filter(|i| i.protocol == "shadowsocks")
    {
        if inbound.settings["method"]
            .as_str()
            .is_none_or(str::is_empty)
        {
            inbound.settings["method"] = json!(method);
            set.push(inbound.tag.clone());
        }
    }
    for tag in set {
        plan.warn(
            "inbound",
            tag,
            "method_set",
            format!("its users each had a method; the inbound uses {method} for all"),
        );
    }
}

/// Remark variables, in this panel's spelling.
fn remark_variables(text: &str) -> String {
    [
        ("{USERNAME}", "{username}"),
        ("{DATA_LEFT}", "{data_left}"),
        ("{DAYS_LEFT}", "{days_left}"),
        ("{EXPIRE_DATE}", "{expire_date}"),
        ("{PROTOCOL}", "{protocol}"),
        ("{TRANSPORT}", "{transport}"),
        ("{STATUS_TEXT}", "{status}"),
    ]
    .iter()
    .fold(text.to_string(), |acc, (from, to)| acc.replace(from, to))
}

/// Marzban stores a host's ALPN as the name of an enum (`h2_http_1_1`).
fn alpn_of_name(name: &str) -> String {
    if name == "none" {
        return String::new();
    }
    name.replace("http_1_1", "http/1.1").replace('_', ",")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Pasarguard,
    Marzban,
}

fn read_hosts(plan: &mut Plan, rows: &[Record], tag_map: &HashMap<String, String>, style: Style) {
    for row in rows {
        let remark = remark_variables(row.filled("remark").unwrap_or("host"));
        let Some(source_tag) = row.filled("inbound_tag") else {
            plan.warn(
                "host",
                &remark,
                "host_without_inbound",
                "it is for no inbound",
            );
            continue;
        };
        let Some(tag) = tag_map.get(source_tag) else {
            // Its inbound was not taken (the warning is that inbound's).
            continue;
        };
        let addresses = row.list("address");
        if addresses.is_empty() {
            continue;
        }
        let first = |column: &str| row.list(column).into_iter().next().unwrap_or_default();
        if row.list("sni").len() > 1 || row.list("host").len() > 1 {
            plan.warn(
                "host",
                &remark,
                "host_first_only",
                "it listed several SNI or Host names; the first is kept",
            );
        }
        let security = row.filled("security").unwrap_or("inbound_default");
        if security != "inbound_default" {
            plan.warn(
                "host",
                &remark,
                "host_security_ignored",
                format!(
                    "its security override ({security}) is not imported; the inbound's applies"
                ),
            );
        }
        let mut ignored: Vec<&str> = Vec::new();
        for column in [
            "fragment_settings",
            "noise_settings",
            "http_headers",
            "transport_settings",
            "mux_settings",
            "ech",
            "final_mask_settings",
            "wireguard_overrides",
            "subscription_templates",
            "fragment_setting",
            "noise_setting",
        ] {
            if row
                .filled(column)
                .is_some_and(|text| !matches!(text, "null" | "{}" | "[]"))
            {
                ignored.push(column);
            }
        }
        if row.flag("mux_enable").unwrap_or(false) {
            ignored.push("mux_enable");
        }
        if row.flag("random_user_agent").unwrap_or(false) {
            ignored.push("random_user_agent");
        }
        if !ignored.is_empty() {
            plan.warn(
                "host",
                &remark,
                "host_features_ignored",
                format!("not imported: {}", ignored.join(", ")),
            );
        }
        let alpn = match style {
            Style::Pasarguard => row.list("alpn").join(","),
            Style::Marzban => alpn_of_name(row.filled("alpn").unwrap_or("none")),
        };
        let fingerprint = row
            .filled("fingerprint")
            .filter(|f| *f != "none")
            .unwrap_or_default()
            .to_string();
        for address in addresses {
            plan.hosts.push(PlannedHost {
                inbound_tag: tag.clone(),
                remark: remark.clone(),
                address,
                port: row.int("port").filter(|p| *p > 0),
                sni: first("sni"),
                host: first("host"),
                path: row.filled("path").unwrap_or_default().to_string(),
                alpn: alpn.clone(),
                fingerprint: fingerprint.clone(),
                allow_insecure: row.flag("allowinsecure").unwrap_or(false),
                enabled: !row.flag("is_disabled").unwrap_or(false),
                priority: row.int("priority").unwrap_or(0),
            });
        }
    }
}

/// Usage by the hour (the whole of it as downlink, since these panels count
/// one figure), for the last stretch the panel keeps.
async fn read_usage(
    plan: &mut Plan,
    source: &Source,
    tables: &HashSet<String>,
) -> Result<(), DbError> {
    if !tables.contains("node_user_usages") {
        return Ok(());
    }
    let rows = source
        .query(
            "SELECT CAST(user_id AS TEXT), SUBSTR(CAST(created_at AS TEXT), 1, 13), CAST(SUM(used_traffic) AS TEXT) \
             FROM node_user_usages GROUP BY user_id, SUBSTR(CAST(created_at AS TEXT), 1, 13)",
        )
        .await?;
    let since = z_db::now() - RETENTION;
    for row in rows {
        let (Some(user), Some(hour), Some(bytes)) = (&row[0], &row[1], &row[2]) else {
            continue;
        };
        let (Ok(user), Ok(bytes)) = (user.parse::<i64>(), bytes.parse::<f64>()) else {
            continue;
        };
        let Some(at) = z_db::parse_datetime(&format!("{}:00:00", hour.replace('T', " "))) else {
            continue;
        };
        if at >= since && bytes > 0.0 {
            plan.usage
                .entry(user)
                .or_default()
                .push((at, 0, bytes as i64));
        }
    }
    Ok(())
}

async fn read_reset_logs(
    source: &Source,
    tables: &HashSet<String>,
) -> Result<HashMap<i64, (i64, i64)>, DbError> {
    // user id → (traffic used before its resets, when it last reset)
    let mut logs: HashMap<i64, (i64, i64)> = HashMap::new();
    if tables.contains("user_usage_logs") {
        for row in source.rows("user_usage_logs").await? {
            let (Some(user), Some(used)) = (row.int("user_id"), row.int("used_traffic_at_reset"))
            else {
                continue;
            };
            let entry = logs.entry(user).or_default();
            entry.0 += used.max(0);
            entry.1 = entry.1.max(row.time("reset_at").unwrap_or(0));
        }
    }
    Ok(logs)
}

async fn next_plans(
    source: &Source,
    tables: &HashSet<String>,
) -> Result<HashMap<i64, PlannedPlan>, DbError> {
    let mut plans = HashMap::new();
    if tables.contains("next_plans") {
        for row in source.rows("next_plans").await? {
            let Some(user) = row.int("user_id") else {
                continue;
            };
            plans.insert(
                user,
                PlannedPlan {
                    data_limit: row.int("data_limit").unwrap_or(0).max(0),
                    expire_days: (row.int("expire").unwrap_or(0).max(0) + 86_399) / 86_400,
                    carry_over: row.flag("add_remaining_traffic").unwrap_or(false),
                },
            );
        }
    }
    Ok(plans)
}

/// What a user is, from the columns the two have in common.
fn user_of(row: &Record, names: &mut HashSet<String>, plan: &mut Plan) -> Option<PlannedUser> {
    let source_name = row.filled("username")?.to_string();
    let (username, changed) = common::username(&source_name, names);
    if changed {
        plan.warn(
            "user",
            &source_name,
            "renamed",
            format!("its name became {username}"),
        );
    }
    let created = row.time("created_at");
    let revoked = row.time("sub_revoked_at").unwrap_or(0);
    let used = row.int("used_traffic").unwrap_or(0).max(0);
    let on_hold_duration = row.int("on_hold_expire_duration").filter(|d| *d > 0);
    Some(PlannedUser {
        source_id: row.int("id")?,
        source_name,
        username,
        status: row
            .filled("status")
            .unwrap_or("active")
            .to_ascii_lowercase(),
        data_limit: row.int("data_limit").unwrap_or(0).max(0),
        used_traffic: used,
        lifetime_traffic: used,
        expire_at: row.time("expire").filter(|t| *t > 0),
        on_hold_duration,
        on_hold_timeout: row.time("on_hold_timeout").filter(|t| *t > 0),
        reset_strategy: row
            .filled("data_limit_reset_strategy")
            .unwrap_or("no_reset")
            .to_ascii_lowercase(),
        hwid_limit: row.int("hwid_limit").unwrap_or(0).max(0),
        note: row.filled("note").unwrap_or_default().to_string(),
        online_at: row.time("online_at"),
        created_at: created,
        valid_after: created.unwrap_or(0).max(revoked),
        ..Default::default()
    })
}

fn valid_status(status: &str) -> &str {
    match status {
        "active" | "disabled" | "limited" | "expired" | "on_hold" => status,
        _ => "active",
    }
}

/// The one id and one password a user has here, out of the several it had
/// (an id for VLESS, another for VMess; a password for Trojan, another for
/// Shadowsocks): the protocols that have an inbound decide which are kept.
/// The last is whether a protocol with an inbound loses the credential it had.
fn credentials(
    protocols: &HashSet<String>,
    id_of: impl Fn(&str) -> Option<String>,
    password_of: impl Fn(&str) -> Option<String>,
) -> (Option<String>, Option<String>, bool) {
    let has = |protocol: &str| protocols.contains(protocol);
    let id = |protocol: &str| id_of(protocol).filter(|u| common::is_uuid(u));
    let uuid = if has("vless") || !has("vmess") {
        id("vless").or_else(|| id("vmess"))
    } else {
        id("vmess").or_else(|| id("vless"))
    };
    let password = if has("shadowsocks") && !has("trojan") {
        password_of("shadowsocks").or_else(|| password_of("trojan"))
    } else {
        password_of("trojan").or_else(|| password_of("shadowsocks"))
    };
    let lost = ["vless", "vmess"]
        .iter()
        .any(|p| has(p) && id(p).is_some() && id(p) != uuid)
        || ["trojan", "shadowsocks"]
            .iter()
            .any(|p| has(p) && password_of(p).is_some() && password_of(p) != password);
    (uuid, password, lost)
}

pub async fn read_pasarguard(source: &Source) -> Result<Plan, DbError> {
    let tables = source.tables().await?;
    let mut plan = Plan::new(Kind::Pasarguard);
    plan.legacy_secret = source
        .rows("jwt")
        .await?
        .iter()
        .find_map(|r| r.filled("secret_key").map(str::to_string));

    // ---- inbounds: the Xray configurations kept in the database
    let mut tags = HashSet::new();
    let mut tag_map: HashMap<String, String> = HashMap::new();
    for row in source.rows("core_configs").await? {
        let name = row.filled("name").unwrap_or("config").to_string();
        let kind = row.filled("type").unwrap_or("xray").to_ascii_lowercase();
        if kind != "xray" {
            plan.warn(
                "source",
                &name,
                "core_not_imported",
                format!("a {kind} core configuration is not imported"),
            );
            continue;
        }
        let Some(config) = row.json("config") else {
            plan.warn(
                "source",
                &name,
                "core_unreadable",
                "its configuration is not JSON",
            );
            continue;
        };
        let exclude: HashSet<String> = row.list("exclude_inbound_tags").into_iter().collect();
        let map = inbounds_from_xray(&mut plan, &config, &exclude, &mut tags);
        tag_map.extend(map);
    }

    // ---- groups: their inbounds by tag, their users by id
    let inbound_tags: HashMap<i64, String> = source
        .rows("inbounds")
        .await?
        .iter()
        .filter_map(|r| Some((r.int("id")?, r.filled("tag")?.to_string())))
        .collect();
    let mut group_tags: HashMap<i64, Vec<String>> = HashMap::new();
    for row in source.rows("inbounds_groups_association").await? {
        if let (Some(inbound), Some(group)) = (row.int("inbound_id"), row.int("group_id")) {
            if let Some(tag) = inbound_tags.get(&inbound).and_then(|t| tag_map.get(t)) {
                group_tags.entry(group).or_default().push(tag.clone());
            }
        }
    }
    let mut group_names: HashMap<i64, String> = HashMap::new();
    let mut used_group_names = HashSet::new();
    for row in source.rows("groups").await? {
        let Some(id) = row.int("id") else { continue };
        let raw = row.filled("name").unwrap_or("group");
        let mut name: String = raw.chars().take(60).collect();
        let mut n = 2;
        while !used_group_names.insert(name.to_ascii_lowercase()) {
            name = format!("{}-{n}", raw.chars().take(56).collect::<String>());
            n += 1;
        }
        let mut inbound_tags = group_tags.remove(&id).unwrap_or_default();
        inbound_tags.sort();
        inbound_tags.dedup();
        group_names.insert(id, name.clone());
        plan.groups.push(PlannedGroup {
            name,
            inbound_tags,
            disabled: row.flag("is_disabled").unwrap_or(false),
        });
    }
    let mut user_groups: HashMap<i64, Vec<String>> = HashMap::new();
    for row in source.rows("users_groups_association").await? {
        if let (Some(user), Some(group)) = (row.int("user_id"), row.int("groups_id")) {
            if let Some(name) = group_names.get(&group) {
                user_groups.entry(user).or_default().push(name.clone());
            }
        }
    }

    // ---- users
    let logs = read_reset_logs(source, &tables).await?;
    let plans = next_plans(source, &tables).await?;
    let mut names = HashSet::new();
    let mut flows: HashMap<String, usize> = HashMap::new();
    let mut methods: HashMap<String, usize> = HashMap::new();
    let mut changed_credentials = 0usize;
    let protocols: HashSet<String> = plan.inbounds.iter().map(|i| i.protocol.clone()).collect();
    for row in source.rows("users").await? {
        let Some(mut user) = user_of(&row, &mut names, &mut plan) else {
            continue;
        };
        user.status = valid_status(&user.status).into();
        let proxies = row.json("proxy_settings").unwrap_or_else(|| json!({}));
        let text = |value: &Value| value.as_str().filter(|s| !s.is_empty()).map(str::to_string);
        let (uuid, password, lost) = credentials(
            &protocols,
            |p| text(&proxies[p]["id"]),
            |p| text(&proxies[p]["password"]),
        );
        (user.uuid, user.password) = (uuid, password);
        changed_credentials += usize::from(lost);
        if let Some(flow) = text(&proxies["vless"]["flow"]) {
            *flows.entry(flow).or_default() += 1;
        }
        if let Some(method) = text(&proxies["shadowsocks"]["method"]) {
            *methods.entry(method).or_default() += 1;
        }
        if let Some((before, last)) = logs.get(&user.source_id) {
            user.lifetime_traffic = user.used_traffic + before;
            user.last_reset_at = (*last > 0).then_some(*last);
        }
        user.next_plan = plans.get(&user.source_id).cloned();
        user.group_names = user_groups.remove(&user.source_id).unwrap_or_default();
        plan.users.push(user);
    }
    if changed_credentials > 0 {
        plan.warn(
            "source",
            "",
            "credentials_changed",
            format!("{changed_credentials} of the users had an id (or password) for one protocol that differs from the one for another; here one of each is kept, so the saved configs of the other protocol stop working and the subscription gives the new ones"),
        );
    }
    apply_flow(&mut plan, &flows);
    apply_methods(&mut plan, &methods);

    // ---- templates
    let mut template_groups: HashMap<i64, Vec<String>> = HashMap::new();
    for row in source.rows("template_group_association").await? {
        if let (Some(template), Some(group)) = (row.int("user_template_id"), row.int("group_id")) {
            if let Some(name) = group_names.get(&group) {
                template_groups
                    .entry(template)
                    .or_default()
                    .push(name.clone());
            }
        }
    }
    let mut template_names = HashSet::new();
    for row in source.rows("user_templates").await? {
        let Some(id) = row.int("id") else { continue };
        let name = row.filled("name").unwrap_or("template").to_string();
        if !template_names.insert(name.clone()) {
            continue;
        }
        plan.templates.push(PlannedTemplate {
            name,
            data_limit: row.int("data_limit").unwrap_or(0).max(0),
            expire_days: (row.int("expire_duration").unwrap_or(0).max(0) + 86_399) / 86_400,
            on_hold: row
                .filled("status")
                .is_some_and(|s| s.eq_ignore_ascii_case("on_hold")),
            reset_strategy: row
                .filled("data_limit_reset_strategy")
                .unwrap_or("no_reset")
                .to_ascii_lowercase(),
            hwid_limit: row.int("hwid_limit").unwrap_or(0).max(0),
            group_names: template_groups.remove(&id).unwrap_or_default(),
        });
    }

    // ---- hosts and usage
    let hosts = source.rows("hosts").await?;
    read_hosts(&mut plan, &hosts, &tag_map, Style::Pasarguard);
    read_usage(&mut plan, source, &tables).await?;
    Ok(plan)
}

pub async fn read_marzban(source: &Source, xray_config: Option<&Value>) -> Result<Plan, DbError> {
    let tables = source.tables().await?;
    let mut plan = Plan::new(Kind::Marzban);
    if tables.contains("jwt") {
        plan.legacy_secret = source
            .rows("jwt")
            .await?
            .iter()
            .find_map(|r| r.filled("secret_key").map(str::to_string));
    }

    // ---- inbounds: from the configuration file, if it was given
    let mut tags = HashSet::new();
    let mut tag_map: HashMap<String, String> = HashMap::new();
    let mut protocol_of: HashMap<String, String> = HashMap::new();
    match xray_config {
        Some(config) => {
            tag_map = inbounds_from_xray(&mut plan, config, &HashSet::new(), &mut tags);
            for inbound in &plan.inbounds {
                protocol_of.insert(inbound.tag.clone(), inbound.protocol.clone());
            }
        }
        None => plan.warn(
            "source",
            "",
            "no_xray_config",
            "Marzban keeps its inbounds in a file (xray_config.json), which was not given: users, hosts and templates are imported, but no inbounds and no access groups",
        ),
    }

    // ---- proxies: what each user has, and what it may not use
    let mut proxies: HashMap<i64, Vec<(i64, String, Value)>> = HashMap::new();
    if tables.contains("proxies") {
        for row in source.rows("proxies").await? {
            if let (Some(id), Some(user)) = (row.int("id"), row.int("user_id")) {
                let kind = row.filled("type").unwrap_or_default().to_ascii_lowercase();
                proxies.entry(user).or_default().push((
                    id,
                    kind,
                    row.json("settings").unwrap_or_else(|| json!({})),
                ));
            }
        }
    }
    let mut excluded: HashMap<i64, HashSet<String>> = HashMap::new();
    // Marzban's table is `exclude_inbounds_association`; the other spelling is
    // the name of the variable it is kept in, which a dump may be mistaken for.
    for table in [
        "exclude_inbounds_association",
        "excluded_inbounds_association",
    ] {
        if !tables.contains(table) {
            continue;
        }
        for row in source.rows(table).await? {
            if let (Some(proxy), Some(tag)) = (row.int("proxy_id"), row.filled("inbound_tag")) {
                excluded.entry(proxy).or_default().insert(tag.to_string());
            }
        }
    }

    // ---- users
    let logs = read_reset_logs(source, &tables).await?;
    let plans = next_plans(source, &tables).await?;
    let mut names = HashSet::new();
    let mut flows: HashMap<String, usize> = HashMap::new();
    let mut methods: HashMap<String, usize> = HashMap::new();
    let mut changed_credentials = 0usize;
    let mut sets: Vec<(Vec<String>, String)> = Vec::new();
    let protocols: HashSet<String> = plan.inbounds.iter().map(|i| i.protocol.clone()).collect();
    for row in source.rows("users").await? {
        let Some(mut user) = user_of(&row, &mut names, &mut plan) else {
            continue;
        };
        user.status = valid_status(&user.status).into();
        let mine = proxies.remove(&user.source_id).unwrap_or_default();
        let text = |value: &Value| value.as_str().filter(|s| !s.is_empty()).map(str::to_string);
        let setting = |kind: &str, key: &str| {
            mine.iter()
                .find(|(_, k, _)| k == kind)
                .and_then(|(_, _, s)| text(&s[key]))
        };
        let (uuid, password, lost) =
            credentials(&protocols, |p| setting(p, "id"), |p| setting(p, "password"));
        (user.uuid, user.password) = (uuid, password);
        changed_credentials += usize::from(lost);
        if let Some(flow) = setting("vless", "flow") {
            *flows.entry(flow).or_default() += 1;
        }
        if let Some(method) = setting("shadowsocks", "method") {
            *methods.entry(method).or_default() += 1;
        }
        if let Some((before, last)) = logs.get(&user.source_id) {
            user.lifetime_traffic = user.used_traffic + before;
            user.last_reset_at = (*last > 0).then_some(*last);
        }
        user.next_plan = plans.get(&user.source_id).cloned();
        // The inbounds it reaches: all of its protocols, but those excluded.
        if !protocol_of.is_empty() {
            let mut reach: Vec<String> = Vec::new();
            for (proxy_id, kind, _) in &mine {
                let skip = excluded.get(proxy_id);
                for (source_tag, tag) in &tag_map {
                    if protocol_of.get(tag) == Some(kind)
                        && !skip.is_some_and(|s| s.contains(source_tag))
                    {
                        reach.push(tag.clone());
                    }
                }
            }
            reach.sort();
            reach.dedup();
            if !reach.is_empty() {
                let name = sets
                    .iter()
                    .find(|(r, _)| *r == reach)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| {
                        let name = format!("marzban-{}", sets.len() + 1);
                        sets.push((reach.clone(), name.clone()));
                        name
                    });
                user.group_names = vec![name];
            }
        }
        plan.users.push(user);
    }
    if changed_credentials > 0 {
        plan.warn(
            "source",
            "",
            "credentials_changed",
            format!("{changed_credentials} of the users had an id (or password) for one protocol that differs from the one for another; here one of each is kept, so the saved configs of the other protocol stop working and the subscription gives the new ones"),
        );
    }
    let all: Vec<String> = {
        let mut all: Vec<String> = plan.inbounds.iter().map(|i| i.tag.clone()).collect();
        all.sort();
        all
    };
    let single = sets.len() == 1 && sets[0].0 == all;
    for (tags, name) in &sets {
        let shown = if single {
            "marzban".to_string()
        } else {
            name.clone()
        };
        plan.groups.push(PlannedGroup {
            name: shown.clone(),
            inbound_tags: tags.clone(),
            disabled: false,
        });
        if single {
            for user in &mut plan.users {
                for group in &mut user.group_names {
                    if group == name {
                        *group = shown.clone();
                    }
                }
            }
        }
    }
    apply_flow(&mut plan, &flows);
    apply_methods(&mut plan, &methods);

    // ---- templates: the inbounds they gave become groups of their own kind
    if tables.contains("user_templates") {
        let mut template_inbounds: HashMap<i64, Vec<String>> = HashMap::new();
        if tables.contains("template_inbounds_association") {
            for row in source.rows("template_inbounds_association").await? {
                if let (Some(template), Some(tag)) =
                    (row.int("user_template_id"), row.filled("inbound_tag"))
                {
                    if let Some(tag) = tag_map.get(tag) {
                        template_inbounds
                            .entry(template)
                            .or_default()
                            .push(tag.clone());
                    }
                }
            }
        }
        let mut template_names = HashSet::new();
        for row in source.rows("user_templates").await? {
            let Some(id) = row.int("id") else { continue };
            let name = row.filled("name").unwrap_or("template").to_string();
            if !template_names.insert(name.clone()) {
                continue;
            }
            let mut tags = template_inbounds.remove(&id).unwrap_or_default();
            tags.sort();
            tags.dedup();
            let group = if tags.is_empty() {
                None
            } else {
                // The group that holds exactly these, made if there is none.
                Some(match plan.groups.iter().find(|g| g.inbound_tags == tags) {
                    Some(group) => group.name.clone(),
                    None => {
                        let group = format!("marzban-t{}", plan.groups.len() + 1);
                        plan.groups.push(PlannedGroup {
                            name: group.clone(),
                            inbound_tags: tags,
                            disabled: false,
                        });
                        group
                    }
                })
            };
            plan.templates.push(PlannedTemplate {
                name,
                data_limit: row.int("data_limit").unwrap_or(0).max(0),
                expire_days: (row.int("expire_duration").unwrap_or(0).max(0) + 86_399) / 86_400,
                reset_strategy: "no_reset".into(),
                group_names: group.into_iter().collect(),
                ..Default::default()
            });
        }
    }

    // ---- hosts and usage
    let hosts = source.rows("hosts").await?;
    read_hosts(&mut plan, &hosts, &tag_map, Style::Marzban);
    read_usage(&mut plan, source, &tables).await?;
    Ok(plan)
}
