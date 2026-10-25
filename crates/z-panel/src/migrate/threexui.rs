//! Reading a 3X-UI database (`x-ui.db`), of the classic kind (every inbound
//! holds its clients in its `settings`, with their traffic in
//! `client_traffics`) and of the newer kind (a `clients` table, and
//! `client_inbounds` saying which inbound each is on).
//!
//! 3X-UI's subscription link is `/{subPath}/{subId}`; a user's `subId` is kept
//! as its token, so the link keeps working at the same path.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};
use z_db::{DbError, Record, Source};

use super::common::{self, number};
use super::model::{Kind, Plan, PlannedGroup, PlannedInbound, PlannedSubscription, PlannedUser};

/// The protocols this panel builds that 3X-UI also has.
const PROTOCOLS: &[&str] = &["vless", "vmess", "trojan", "shadowsocks"];

/// A client, whichever way the old panel kept it.
#[derive(Debug, Default, Clone)]
struct Client {
    email: String,
    uuid: Option<String>,
    password: Option<String>,
    flow: String,
    limit_ip: i64,
    limit_hwid: i64,
    total: i64,
    /// Milliseconds; negative for "this long after the first use".
    expiry: i64,
    enable: bool,
    tg_id: i64,
    sub_id: String,
    comment: String,
    group: String,
    reset_days: i64,
    /// Renews on this day of each month / on this weekday (1 Monday … 7 Sunday).
    reset_day: i64,
    reset_weekday: i64,
    traffic_reset: String,
    created: Option<i64>,
    up: i64,
    down: i64,
    last_online: Option<i64>,
    /// Inbounds (old ids) it is on.
    inbounds: Vec<i64>,
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// A client out of an inbound's `settings.clients`.
fn client_of_json(entry: &Value, inbound: i64) -> Client {
    Client {
        email: text_of(&entry["email"]),
        uuid: Some(text_of(&entry["id"])).filter(|s| !s.is_empty()),
        password: Some(text_of(&entry["password"])).filter(|s| !s.is_empty()),
        flow: text_of(&entry["flow"]),
        limit_ip: number(&entry["limitIp"]).unwrap_or(0),
        limit_hwid: number(&entry["limitHwid"]).unwrap_or(0),
        total: number(&entry["totalGB"]).unwrap_or(0),
        expiry: number(&entry["expiryTime"]).unwrap_or(0),
        enable: entry.get("enable").and_then(Value::as_bool).unwrap_or(true),
        tg_id: number(&entry["tgId"]).unwrap_or(0),
        sub_id: text_of(&entry["subId"]),
        comment: text_of(&entry["comment"]),
        group: text_of(&entry["group"]),
        reset_days: number(&entry["reset"]).unwrap_or(0),
        traffic_reset: text_of(&entry["trafficReset"]),
        created: number(&entry["created_at"]).filter(|n| *n > 0).map(|n| {
            if n > 100_000_000_000 {
                n / 1000
            } else {
                n
            }
        }),
        inbounds: vec![inbound],
        ..Default::default()
    }
}

fn client_of_row(row: &Record) -> Client {
    Client {
        email: row.filled("email").unwrap_or_default().to_string(),
        uuid: row.filled("uuid").map(str::to_string),
        password: row.filled("password").map(str::to_string),
        flow: row.filled("flow").unwrap_or_default().to_string(),
        limit_ip: row.int("limit_ip").unwrap_or(0),
        limit_hwid: row.int("limit_hwid").unwrap_or(0),
        total: row.int("total_gb").unwrap_or(0),
        expiry: row.int("expiry_time").unwrap_or(0),
        enable: row.flag("enable").unwrap_or(true),
        tg_id: row.int("tg_id").unwrap_or(0),
        sub_id: row.filled("sub_id").unwrap_or_default().to_string(),
        comment: row.filled("comment").unwrap_or_default().to_string(),
        group: row.filled("group_name").unwrap_or_default().to_string(),
        reset_days: row.int("reset").unwrap_or(0),
        reset_day: row.int("reset_day").unwrap_or(0),
        reset_weekday: row.int("reset_weekday").unwrap_or(0),
        traffic_reset: row.filled("traffic_reset").unwrap_or_default().to_string(),
        created: row.time("created_at"),
        ..Default::default()
    }
}

fn reset_of(client: &Client, name: &str, plan: &mut Plan) -> &'static str {
    match client.traffic_reset.as_str() {
        "daily" => return "day",
        "weekly" => return "week",
        "monthly" => return "month",
        "hourly" => {
            plan.warn(
                "user",
                name,
                "reset_approximated",
                "resets every hour in 3X-UI; here it resets every day",
            );
            return "day";
        }
        _ => {}
    }
    if client.reset_days == 0 {
        if client.reset_weekday > 0 {
            return "week";
        }
        if client.reset_day > 0 {
            return "month";
        }
    }
    let (strategy, approximated) = common::reset_from_days(client.reset_days);
    if approximated {
        plan.warn(
            "user",
            name,
            "reset_approximated",
            format!(
                "resets every {} days in 3X-UI; here it resets every {strategy}",
                client.reset_days
            ),
        );
    }
    strategy
}

pub async fn read(source: &Source) -> Result<Plan, DbError> {
    let tables = source.tables().await?;
    let mut plan = Plan::new(Kind::ThreeXUi);
    let now = z_db::now();

    // ---- inbounds (and the clients inside them)
    let mut raw_clients: Vec<Client> = Vec::new();
    let mut tags = HashSet::new();
    // Old inbound id → the tag it has in the plan.
    let mut imported: HashMap<i64, String> = HashMap::new();
    for row in source.rows("inbounds").await? {
        let id = row.int("id").unwrap_or(0);
        let protocol = row
            .filled("protocol")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let port = row.int("port").unwrap_or(0);
        let raw_tag = row
            .filled("tag")
            .map(str::to_string)
            .unwrap_or_else(|| format!("{protocol}-{port}"));
        let remark = row.filled("remark").unwrap_or(&raw_tag).to_string();
        if row.int("node_id").is_some_and(|n| n > 0) {
            plan.warn(
                "inbound",
                &raw_tag,
                "remote_inbound",
                "it runs on another node of the old panel, not on this server",
            );
            continue;
        }
        let mut settings = row
            .text("settings")
            .and_then(common::parse_json_text)
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        // Its accounts are read whether or not the inbound can be taken: a
        // user on it is still a user.
        let clients: Vec<Value> = settings
            .get("clients")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for entry in &clients {
            let client = client_of_json(entry, id);
            if client.email.is_empty() {
                plan.warn(
                    "inbound",
                    &raw_tag,
                    "client_without_email",
                    "a client has no email, which is its name here",
                );
                continue;
            }
            raw_clients.push(client);
        }
        if !PROTOCOLS.contains(&protocol.as_str()) {
            plan.warn(
                "inbound",
                &raw_tag,
                "unsupported_protocol",
                format!("{protocol} inbounds are not imported (vless, vmess, trojan and shadowsocks are)"),
            );
            continue;
        }
        if row.int("total").unwrap_or(0) > 0 || row.int("expiry_time").unwrap_or(0) > 0 {
            plan.warn(
                "inbound",
                &raw_tag,
                "inbound_limit_ignored",
                "a limit on the whole inbound is not imported; users have their own",
            );
        }
        if let Some(map) = settings.as_object_mut() {
            map.remove("clients");
        }
        let mut stream = row
            .text("stream_settings")
            .and_then(common::parse_json_text)
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        common::lift_reality(&mut stream);
        let certificate = common::certificate_of(&stream);
        let sniffing = row
            .text("sniffing")
            .and_then(common::parse_json_text)
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        let (tag, changed) = common::tag(&raw_tag, &mut tags);
        if changed {
            plan.warn(
                "inbound",
                &raw_tag,
                "renamed",
                format!("its tag became {tag}"),
            );
        }
        imported.insert(id, tag.clone());
        plan.inbounds.push(PlannedInbound {
            tag,
            remark,
            enabled: row.flag("enable").unwrap_or(true),
            listen: row.filled("listen").unwrap_or_default().to_string(),
            port,
            protocol,
            settings,
            stream,
            sniffing,
            certificate,
        });
    }

    // ---- the newer kind keeps clients in a table of their own
    if tables.contains("clients") {
        let mut by_id: HashMap<i64, usize> = HashMap::new();
        let mut table_clients: Vec<Client> = Vec::new();
        for row in source.rows("clients").await? {
            let client = client_of_row(&row);
            if client.email.is_empty() {
                continue;
            }
            by_id.insert(row.int("id").unwrap_or(-1), table_clients.len());
            table_clients.push(client);
        }
        if tables.contains("client_inbounds") {
            for row in source.rows("client_inbounds").await? {
                if let (Some(client), Some(inbound)) = (row.int("client_id"), row.int("inbound_id"))
                {
                    if let Some(index) = by_id.get(&client) {
                        table_clients[*index].inbounds.push(inbound);
                    }
                }
            }
        }
        // What the table holds wins; a client only in an inbound's settings stays.
        let known: HashSet<String> = table_clients.iter().map(|c| c.email.clone()).collect();
        raw_clients.retain(|c| !known.contains(&c.email));
        table_clients.append(&mut raw_clients);
        raw_clients = table_clients;
    }

    // ---- the flow is the inbound's here, not each client's
    for (id, tag) in &imported {
        let Some(inbound) = plan
            .inbounds
            .iter()
            .position(|i| &i.tag == tag && i.protocol == "vless")
        else {
            continue;
        };
        let on: Vec<&Client> = raw_clients
            .iter()
            .filter(|c| c.inbounds.contains(id))
            .collect();
        let mut flows: HashMap<&str, usize> = HashMap::new();
        for client in &on {
            if !client.flow.is_empty() {
                *flows.entry(client.flow.as_str()).or_default() += 1;
            }
        }
        let Some((flow, _)) = flows
            .iter()
            .max_by_key(|(flow, n)| (**n, std::cmp::Reverse(**flow)))
        else {
            continue;
        };
        let (flow, mixed) = (
            (*flow).to_string(),
            flows.len() > 1 || flows.values().sum::<usize>() != on.len(),
        );
        plan.inbounds[inbound].settings["flow"] = json!(flow);
        if mixed {
            plan.warn(
                "inbound",
                tag,
                "flow_for_all",
                format!(
                    "clients had different flows; every client on this inbound now uses {flow}"
                ),
            );
        }
    }

    // ---- merge by email: one user, however many inbounds it is on
    let mut order: Vec<String> = Vec::new();
    let mut merged: HashMap<String, Client> = HashMap::new();
    let mut warned_ids: HashSet<String> = HashSet::new();
    for client in raw_clients {
        match merged.get_mut(&client.email) {
            None => {
                order.push(client.email.clone());
                merged.insert(client.email.clone(), client);
            }
            Some(first) => {
                for inbound in &client.inbounds {
                    if !first.inbounds.contains(inbound) {
                        first.inbounds.push(*inbound);
                    }
                }
                let differs = |a: &Option<String>, b: &Option<String>| matches!((a, b), (Some(a), Some(b)) if a != b);
                if (differs(&first.uuid, &client.uuid)
                    || differs(&first.password, &client.password))
                    && warned_ids.insert(client.email.clone())
                {
                    plan.warn(
                        "user",
                        &client.email,
                        "different_ids",
                        "it had a different id on different inbounds; one is kept, so its saved configs for the others stop working, and its subscription is the way to get them again",
                    );
                }
                first.uuid = first.uuid.take().or(client.uuid);
                first.password = first.password.take().or(client.password);
                if first.sub_id.is_empty() {
                    first.sub_id = client.sub_id;
                }
                if first.flow.is_empty() {
                    first.flow = client.flow;
                }
            }
        }
    }

    // ---- what each client used, and its limits as the panel enforces them
    if tables.contains("client_traffics") {
        for row in source.rows("client_traffics").await? {
            let Some(email) = row.filled("email") else {
                continue;
            };
            let Some(client) = merged.get_mut(email) else {
                continue;
            };
            client.up = row.int("up").unwrap_or(0).max(0);
            client.down = row.int("down").unwrap_or(0).max(0);
            if let Some(enable) = row.flag("enable") {
                client.enable = enable;
            }
            if row.has("total") {
                client.total = row.int("total").unwrap_or(client.total);
            }
            if row.has("expiry_time") {
                client.expiry = row.int("expiry_time").unwrap_or(client.expiry);
            }
            if client.reset_days == 0 {
                client.reset_days = row.int("reset").unwrap_or(0);
            }
            if client.reset_day == 0 {
                client.reset_day = row.int("reset_day").unwrap_or(0);
            }
            if client.reset_weekday == 0 {
                client.reset_weekday = row.int("reset_weekday").unwrap_or(0);
            }
            client.last_online = row.int("last_online").filter(|n| *n > 0).map(|n| n / 1000);
        }
    }

    // ---- users
    let mut names = HashSet::new();
    let mut sub_ids: HashSet<String> = HashSet::new();
    let mut set_names: Vec<(Vec<String>, String)> = Vec::new();
    for (index, email) in order.iter().enumerate() {
        let client = &merged[email];
        let (username, changed) = common::username(email, &mut names);
        if changed {
            plan.warn(
                "user",
                email,
                "renamed",
                format!("its name became {username}"),
            );
        }
        let used = client.up + client.down;
        let expiry_seconds = (client.expiry > 0).then_some(client.expiry / 1000);
        let on_hold = client.expiry < 0;
        let status = if on_hold {
            if client.enable {
                "on_hold"
            } else {
                "disabled"
            }
        } else if client.total > 0 && used >= client.total {
            "limited"
        } else if expiry_seconds.is_some_and(|at| at <= now) {
            "expired"
        } else if !client.enable {
            "disabled"
        } else {
            "active"
        };
        let mut notes: Vec<String> = Vec::new();
        if !client.comment.is_empty() {
            notes.push(client.comment.clone());
        }
        if client.tg_id != 0 {
            notes.push(format!("telegram: {}", client.tg_id));
        }
        if !client.group.is_empty() {
            notes.push(format!("group: {}", client.group));
        }
        let uuid = client.uuid.clone().filter(|u| common::is_uuid(u));
        if client.uuid.is_some() && uuid.is_none() {
            plan.warn(
                "user",
                email,
                "id_not_uuid",
                "its id is not a UUID, so a new one is made",
            );
        }
        let sub_token = Some(client.sub_id.clone()).filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        });
        let sub_token = match sub_token {
            Some(token) if !sub_ids.insert(token.clone()) => {
                plan.warn(
                    "user",
                    email,
                    "shared_sub_id",
                    "another user has the same subscription link; the first keeps it and this one gets a new one",
                );
                None
            }
            other => {
                if let Some(token) = &other {
                    sub_ids.insert(token.clone());
                }
                other
            }
        };
        // The inbounds it was on that this panel takes.
        let mut group_tags: Vec<String> = client
            .inbounds
            .iter()
            .filter_map(|id| imported.get(id).cloned())
            .collect();
        group_tags.sort();
        group_tags.dedup();
        let group_name = if group_tags.is_empty() {
            plan.warn(
                "user",
                email,
                "no_inbound",
                "none of the inbounds it was on is imported, so it has no access yet",
            );
            None
        } else {
            let found = set_names
                .iter()
                .find(|(tags, _)| *tags == group_tags)
                .map(|(_, name)| name.clone());
            Some(found.unwrap_or_else(|| {
                let name = format!("3x-ui-{}", set_names.len() + 1);
                set_names.push((group_tags.clone(), name.clone()));
                name
            }))
        };
        let reset_strategy: String = reset_of(client, email, &mut plan).into();
        plan.users.push(PlannedUser {
            source_id: index as i64 + 1,
            source_name: email.clone(),
            username,
            status: status.into(),
            uuid,
            password: client.password.clone().filter(|p| !p.is_empty()),
            sub_token,
            data_limit: client.total.max(0),
            used_traffic: used,
            lifetime_traffic: used,
            expire_at: expiry_seconds,
            on_hold_duration: on_hold.then(|| -client.expiry / 1000),
            reset_strategy,
            ip_limit: client.limit_ip.max(0),
            hwid_limit: client.limit_hwid.max(0),
            note: notes.join(" · "),
            online_at: client.last_online,
            created_at: client.created,
            group_names: group_name.into_iter().collect(),
            ..Default::default()
        });
    }
    // One group, named for what it holds, for each set of inbounds a user was on.
    let all: Vec<String> = {
        let mut tags: Vec<String> = plan.inbounds.iter().map(|i| i.tag.clone()).collect();
        tags.sort();
        tags
    };
    let single = set_names.len() == 1 && set_names[0].0 == all;
    for (tags, name) in &set_names {
        let shown = if single {
            "3x-ui".to_string()
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

    // ---- its one admin
    if tables.contains("users") {
        for row in source.rows("users").await? {
            if let (Some(id), Some(name)) = (row.int("id"), row.filled("username")) {
                plan.admins.push(super::model::PlannedAdmin {
                    source_id: id,
                    name: name.to_string(),
                    sudo: true,
                    enabled: true,
                });
            }
        }
    }

    // ---- subscription options
    if tables.contains("settings") {
        let mut options: HashMap<String, String> = HashMap::new();
        for row in source.rows("settings").await? {
            if let (Some(key), Some(value)) = (row.filled("key"), row.text("value")) {
                options.insert(key.to_string(), value.to_string());
            }
        }
        let path = |key: &str| {
            options
                .get(key)
                .map(|p| format!("/{}", p.trim().trim_matches('/')))
                .filter(|p| {
                    p.len() > 1
                        && p.bytes().all(|b| {
                            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/')
                        })
                })
        };
        let wanted = |key: &str| options.get(key).is_none_or(|v| v != "false");
        let subscription = PlannedSubscription {
            path: path("subPath"),
            json_path: path("subJsonPath").filter(|_| wanted("subJsonEnable")),
            clash_path: path("subClashPath").filter(|_| wanted("subClashEnable")),
            title: options
                .get("subTitle")
                .filter(|t| !t.trim().is_empty())
                .cloned(),
            update_hours: options
                .get("subUpdates")
                .and_then(|v| v.trim().parse().ok())
                .filter(|h| *h > 0),
            support_url: options
                .get("subSupportUrl")
                .filter(|t| !t.trim().is_empty())
                .cloned(),
        };
        if subscription != PlannedSubscription::default() {
            plan.subscription = Some(subscription);
        }
    }
    Ok(plan)
}
