//! The readers against databases made the way each panel makes its own, and
//! the whole import through the API.

use std::path::PathBuf;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use super::model::{Kind, Plan};
use super::*;

pub(crate) const PASARGUARD: &str = include_str!("fixtures/pasarguard.sql");
pub(crate) const CLASSIC: &str = include_str!("fixtures/threexui_classic.sql");
const V3: &str = include_str!("fixtures/threexui_v3.sql");
const MARZBAN: &str = include_str!("fixtures/marzban.sql");
const MARZBAN_XRAY: &str = include_str!("fixtures/marzban_xray.json");

/// A SQLite file made from a script, removed when dropped.
pub struct TempDb(PathBuf);

impl TempDb {
    pub async fn new(script: &str) -> Self {
        use std::hash::{BuildHasher, Hasher};
        let unique = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        let path = std::env::temp_dir().join(format!("z-import-test-{unique:x}.db"));
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(script.to_string()))
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        Self(path)
    }

    pub fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }

    pub fn bytes(&self) -> Vec<u8> {
        std::fs::read(&self.0).unwrap()
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn plan_of(script: &str) -> Plan {
    let db = TempDb::new(script).await;
    read(db.path(), None, None).await.unwrap()
}

fn has_warning(plan: &Plan, scope: &str, name: &str, code: &str) -> bool {
    plan.warnings
        .iter()
        .any(|w| w.scope == scope && w.code == code && (name.is_empty() || w.name == name))
}

fn user<'a>(plan: &'a Plan, name: &str) -> &'a super::model::PlannedUser {
    plan.users
        .iter()
        .find(|u| u.source_name == name)
        .unwrap_or_else(|| panic!("no user {name}"))
}

fn inbound<'a>(plan: &'a Plan, tag: &str) -> &'a super::model::PlannedInbound {
    plan.inbounds
        .iter()
        .find(|i| i.tag == tag)
        .unwrap_or_else(|| panic!("no inbound {tag}"))
}

fn group<'a>(plan: &'a Plan, name: &str) -> &'a super::model::PlannedGroup {
    plan.groups
        .iter()
        .find(|g| g.name == name)
        .unwrap_or_else(|| panic!("no group {name}"))
}

const GIB: i64 = 1 << 30;

fn marzban_config() -> serde_json::Value {
    serde_json::from_str(MARZBAN_XRAY).unwrap()
}

#[tokio::test]
async fn each_panel_is_told_from_its_tables_and_anything_else_is_refused() {
    for (script, kind) in [
        (PASARGUARD, Kind::Pasarguard),
        (MARZBAN, Kind::Marzban),
        (CLASSIC, Kind::ThreeXUi),
        (V3, Kind::ThreeXUi),
    ] {
        let db = TempDb::new(script).await;
        let source = z_db::Source::open(db.path()).await.unwrap();
        assert_eq!(detect(&source).await.unwrap(), Some(kind));
    }
    let other = TempDb::new("CREATE TABLE notes (id integer, body text);").await;
    assert!(matches!(
        read(other.path(), None, None).await,
        Err(ReadError::Unknown)
    ));

    let junk = std::env::temp_dir().join("z-import-test-junk.db");
    std::fs::write(
        &junk,
        b"this is not a database, only text that is long enough to be a file",
    )
    .unwrap();
    assert!(matches!(
        read(junk.to_str().unwrap(), None, None).await,
        Err(ReadError::Open(_))
    ));
    let _ = std::fs::remove_file(junk);

    // A kind that is given is believed over the tables.
    let db = TempDb::new(PASARGUARD).await;
    assert!(
        read(db.path(), Some(Kind::ThreeXUi), None).await.is_ok(),
        "an empty plan, not an error"
    );
}

#[tokio::test]
async fn pasarguard_users_come_with_their_quota_dates_and_credentials() {
    let plan = plan_of(PASARGUARD).await;
    assert_eq!(plan.kind, Kind::Pasarguard);
    assert_eq!(plan.users.len(), 6);

    let alice = user(&plan, "alice");
    assert_eq!(alice.status, "active");
    assert_eq!(
        (alice.data_limit, alice.used_traffic, alice.lifetime_traffic),
        (10 * GIB, 3 * GIB, 3 * GIB)
    );
    assert_eq!(alice.expire_at, Some(1_803_859_200), "2027-03-01");
    assert_eq!(alice.reset_strategy, "month");
    assert_eq!(
        alice.uuid.as_deref(),
        Some("00000001-aaaa-4bbb-8ccc-000000000001")
    );
    assert_eq!(alice.password.as_deref(), Some("trojan-pass-1"));
    assert_eq!(alice.note, "VIP customer");
    assert_eq!(alice.created_at, Some(1_768_037_400), "2026-01-10 09:30");
    assert_eq!(alice.online_at, Some(1_792_692_300));
    let mut groups = alice.group_names.clone();
    groups.sort();
    assert_eq!(groups, ["Basic", "Premium"]);
    assert_eq!(alice.sub_token, None, "its link is signed, not a token");

    let bob = user(&plan, "bob_sh");
    assert_eq!(
        (
            bob.status.as_str(),
            bob.on_hold_duration,
            bob.on_hold_timeout
        ),
        ("on_hold", Some(30 * 86_400), Some(1_796_083_200))
    );

    let carol = user(&plan, "carol");
    assert_eq!((carol.status.as_str(), carol.hwid_limit), ("limited", 2));

    let dave = user(&plan, "dave");
    assert_eq!(dave.status, "expired");
    assert_eq!(
        dave.valid_after, 1_780_272_000,
        "links from before the revoke on 2026-06-01 are void"
    );
    assert_eq!(
        user(&plan, "alice").valid_after,
        1_768_037_400,
        "no revoke: since it was made"
    );

    // A reset log adds what was used before it to the lifetime, and says when.
    let frank = user(&plan, "frank");
    assert_eq!((frank.used_traffic, frank.lifetime_traffic), (GIB, 6 * GIB));
    assert!(frank.last_reset_at.is_some());
    let next = frank.next_plan.as_ref().unwrap();
    assert_eq!(
        (next.data_limit, next.expire_days, next.carry_over),
        (50 * GIB, 30, true)
    );

    // `eve.test` had another VMess id and another Shadowsocks password.
    assert!(has_warning(&plan, "source", "", "credentials_changed"));
    assert_eq!(user(&plan, "eve.test").status, "disabled");
}

#[tokio::test]
async fn pasarguard_inbounds_groups_hosts_templates_and_usage_come_over() {
    let plan = plan_of(PASARGUARD).await;
    let tags: Vec<&str> = plan.inbounds.iter().map(|i| i.tag.as_str()).collect();
    assert_eq!(
        tags,
        [
            "VLESS-TCP-REALITY",
            "Trojan-WS",
            "VMess-WS",
            "Shadowsocks-TCP"
        ],
        "the API inbound is not one"
    );
    let reality = inbound(&plan, "VLESS-TCP-REALITY");
    assert_eq!((reality.port, reality.protocol.as_str()), (443, "vless"));
    assert_eq!(
        reality.settings["flow"], "xtls-rprx-vision",
        "the users' flow is the inbound's here"
    );
    assert_eq!(
        reality.stream["realitySettings"]["serverNames"][0],
        "yahoo.com"
    );
    assert_eq!(
        inbound(&plan, "Shadowsocks-TCP").settings["method"],
        "chacha20-ietf-poly1305"
    );
    // The file holds only the private key; the public one that links need is derived.
    let private = reality.stream["realitySettings"]["privateKey"]
        .as_str()
        .unwrap();
    assert_eq!(
        reality.stream["realitySettings"]["publicKey"],
        zero_runtime::keys::x25519_from_private(private)
            .unwrap()
            .public_key
    );
    assert!(has_warning(
        &plan,
        "inbound",
        "VLESS TCP REALITY",
        "renamed"
    ));

    assert_eq!(
        group(&plan, "Premium").inbound_tags,
        ["Trojan-WS", "VLESS-TCP-REALITY"]
    );
    assert!(group(&plan, "Retired").disabled);

    let template = &plan.templates[0];
    assert_eq!(
        (
            template.name.as_str(),
            template.data_limit,
            template.expire_days
        ),
        ("monthly-10g", 10 * GIB, 30)
    );
    assert_eq!(template.group_names, ["Premium"]);

    assert_eq!(plan.hosts.len(), 2);
    let cdn = &plan.hosts[0];
    assert_eq!(
        (
            cdn.inbound_tag.as_str(),
            cdn.address.as_str(),
            cdn.port,
            cdn.alpn.as_str()
        ),
        ("Trojan-WS", "cdn.example.com", Some(2053), "h2,http/1.1")
    );
    assert_eq!(
        cdn.remark, "{username} - {data_left}",
        "PasarGuard's variables are this panel's"
    );
    assert!(has_warning(&plan, "host", "", "host_security_ignored"));
    assert!(!plan.hosts[1].enabled);

    // Hourly usage, the master and a node added up.
    let alice = user(&plan, "alice").source_id;
    assert_eq!(
        plan.usage[&alice],
        [
            (1_792_645_200, 0, 1_342_177_280),
            (1_792_648_800, 0, 671_088_640)
        ]
    );
    assert_eq!(plan.legacy_secret.as_deref(), Some(PASARGUARD_SECRET));
}

const PASARGUARD_SECRET: &str = "5f3a9c1e7b2d4f60a8c3e1d9b7f50426c8a1e3d5f7092b4c6e8a0d2f4b6c8e1a";

#[derive(serde::Deserialize)]
struct Tokens {
    secret: String,
    tokens: Vec<Token>,
}

#[derive(serde::Deserialize)]
struct Token {
    name: String,
    token: String,
    claim: Option<Claim>,
}

#[derive(serde::Deserialize)]
struct Claim {
    created_at: i64,
    user_id: Option<i64>,
    username: Option<String>,
}

#[test]
fn a_link_is_judged_the_way_pasarguard_judged_it_when_it_made_it() {
    // The tokens in this file were made by PasarGuard's own code and each was
    // judged by its own `get_subscription_payload`; `claim` is what it said.
    let file: Tokens =
        serde_json::from_str(include_str!("fixtures/pasarguard.tokens.json")).unwrap();
    assert_eq!(file.secret, PASARGUARD_SECRET);
    let now = z_db::now();
    let mut accepted = 0;
    for token in &file.tokens {
        let ours = legacy::parse(&token.token, &file.secret, now);
        match (&token.claim, ours) {
            (None, None) => {}
            (Some(want), Some(got)) => {
                accepted += 1;
                assert_eq!(got.created_at, want.created_at, "{}", token.name);
                let who = match (&want.user_id, &want.username) {
                    (Some(id), _) => legacy::Who::Id(*id),
                    (_, Some(name)) => legacy::Who::Name(name.clone()),
                    _ => unreachable!(),
                };
                assert_eq!(got.who, who, "{}", token.name);
            }
            (want, got) => panic!(
                "{}: PasarGuard said {:?}, this said {got:?}",
                token.name,
                want.is_some()
            ),
        }
    }
    assert!(
        accepted >= 6,
        "the file holds every kind of link: {accepted}"
    );
    // The key is the whole of it: the same link under another key is nothing.
    let v3 = file.tokens.iter().find(|t| t.name == "v3_alice").unwrap();
    assert!(legacy::parse(&v3.token, "another-key", now).is_none());
    assert!(legacy::parse("short", &file.secret, now).is_none());
    assert!(legacy::parse("", &file.secret, now).is_none());
    assert!(legacy::parse("a.b.c.d.e.f.g.h.i.j.k.l", &file.secret, now).is_none());
}

#[tokio::test]
async fn marzban_reads_its_inbounds_from_the_file_and_what_each_user_may_reach() {
    let db = TempDb::new(MARZBAN).await;
    let plan = read(db.path(), None, Some(&marzban_config()))
        .await
        .unwrap();
    assert_eq!(plan.kind, Kind::Marzban);
    assert_eq!(plan.inbounds.len(), 4);
    assert_eq!(
        inbound(&plan, "VLESS-TCP-REALITY").settings["flow"],
        "xtls-rprx-vision"
    );
    assert!(has_warning(&plan, "inbound", "API", "unsupported_protocol"));

    let zed = user(&plan, "zed");
    assert_eq!(
        (
            zed.status.as_str(),
            zed.data_limit,
            zed.used_traffic,
            zed.expire_at
        ),
        ("active", 20 * GIB, 5 * GIB, Some(1_830_000_000))
    );
    assert_eq!(zed.reset_strategy, "month");
    assert_eq!(
        zed.lifetime_traffic,
        12 * GIB,
        "the 7 GiB of before its reset on 2026-09-12"
    );
    assert_eq!(
        zed.uuid.as_deref(),
        Some("00000001-2222-4333-8444-000000000001")
    );

    let yan = user(&plan, "yan");
    assert_eq!(
        (yan.status.as_str(), yan.on_hold_duration),
        ("on_hold", Some(604_800))
    );

    // zed has all four protocols; yan only VMess; wei's VLESS excludes its only inbound.
    assert_eq!(group(&plan, &zed.group_names[0]).inbound_tags.len(), 4);
    assert_eq!(
        group(&plan, &user(&plan, "yan").group_names[0]).inbound_tags,
        ["VMess-WS"]
    );
    assert_eq!(
        group(&plan, &user(&plan, "wei").group_names[0]).inbound_tags,
        ["VMess-WS"]
    );
    assert_eq!(
        user(&plan, "xia").next_plan.as_ref().map(|p| p.data_limit),
        Some(10 * GIB)
    );

    let template = &plan.templates[0];
    assert_eq!(
        (template.name.as_str(), template.expire_days),
        ("basic-5g", 30)
    );
    assert_eq!(
        group(&plan, &template.group_names[0]).inbound_tags,
        ["Trojan-TCP", "VMess-WS"]
    );

    let cdn = &plan.hosts[0];
    assert_eq!(
        (
            cdn.alpn.as_str(),
            cdn.fingerprint.as_str(),
            cdn.path.as_str()
        ),
        ("h2,http/1.1", "chrome", "/vmess")
    );
    assert_eq!(plan.hosts[1].alpn, "");
    let zed_hours = &plan.usage[&zed.source_id];
    assert_eq!(zed_hours, &[(1_792_645_200, 0, 1_610_612_736)]);
    assert!(plan.legacy_secret.is_some());
}

#[tokio::test]
async fn marzban_without_its_config_file_still_brings_its_users_and_says_what_is_missing() {
    let db = TempDb::new(MARZBAN).await;
    let plan = read(db.path(), None, None).await.unwrap();
    assert!(plan.inbounds.is_empty() && plan.groups.is_empty());
    assert_eq!(plan.users.len(), 4);
    assert!(plan.users.iter().all(|u| u.group_names.is_empty()));
    assert!(has_warning(&plan, "source", "", "no_xray_config"));
}

#[test]
fn a_marzban_link_is_good_only_for_a_user_it_names_and_after_its_last_revoke() {
    let file: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/marzban.tokens.json")).unwrap();
    let secret = file["secret"].as_str().unwrap();
    for token in file["tokens"].as_array().unwrap() {
        let claim = legacy::parse(token["token"].as_str().unwrap(), secret, z_db::now());
        match token["who"].as_str() {
            Some(who) => {
                let claim = claim.unwrap_or_else(|| panic!("{} is signed", token["name"]));
                assert_eq!(
                    claim.who,
                    legacy::Who::Name(who.into()),
                    "{}",
                    token["name"]
                );
            }
            None => assert!(
                claim.is_some(),
                "a well signed link to nobody still parses; no user is found for it"
            ),
        }
    }
}

#[tokio::test]
async fn threexui_classic_users_keep_their_subscription_ids_and_limits() {
    let plan = plan_of(CLASSIC).await;
    assert_eq!(plan.kind, Kind::ThreeXUi);
    let tags: Vec<&str> = plan.inbounds.iter().map(|i| i.tag.as_str()).collect();
    assert_eq!(
        tags,
        [
            "inbound-443",
            "inbound-8443",
            "inbound-2096",
            "inbound-1080"
        ],
        "the Hysteria one is not taken"
    );
    assert!(has_warning(
        &plan,
        "inbound",
        "inbound-4443",
        "unsupported_protocol"
    ));

    let reality = inbound(&plan, "inbound-443");
    assert_eq!(reality.settings["flow"], "xtls-rprx-vision");
    assert!(
        reality.settings.get("clients").is_none(),
        "the accounts are the users'"
    );
    assert_eq!(
        reality.stream["realitySettings"]["publicKey"],
        "Zk3m8vQ1nR6tY0wA9bC2dE4fG7hJ5lN8pS1uV3xX6zA",
        "lifted out of `settings`"
    );
    assert_eq!(reality.stream["realitySettings"]["fingerprint"], "chrome");
    assert!(inbound(&plan, "inbound-2096").certificate.is_some());

    let ali = user(&plan, "ali@home");
    assert_eq!(
        ali.sub_token.as_deref(),
        Some("ali0sub1id"),
        "the same link, the same path"
    );
    assert_eq!(
        (
            ali.data_limit,
            ali.used_traffic,
            ali.expire_at,
            ali.ip_limit
        ),
        (10 * GIB, 3 * GIB, Some(1_803_859_200), 2)
    );
    assert_eq!(ali.reset_strategy, "month");
    assert_eq!(ali.note, "friend · telegram: 12345");
    assert_eq!(
        ali.uuid.as_deref(),
        Some("00000001-1111-4222-8333-000000000001")
    );
    assert!(has_warning(&plan, "user", "ali@home", "different_ids"));

    let statuses: Vec<(&str, &str)> = plan
        .users
        .iter()
        .map(|u| (u.source_name.as_str(), u.status.as_str()))
        .take(5)
        .collect();
    assert_eq!(
        statuses,
        [
            ("ali@home", "active"),
            ("neg@home", "on_hold"),
            ("off@home", "disabled"),
            ("quota@home", "limited"),
            ("old@home", "expired")
        ]
    );
    assert_eq!(
        user(&plan, "neg@home").on_hold_duration,
        Some(2_592_000),
        "30 days after first use"
    );
    assert_eq!(
        user(&plan, "troj@home").password.as_deref(),
        Some("trojan-secret-1")
    );

    // The same subscription id on two users: the first keeps it.
    assert_eq!(user(&plan, "bob@home").sub_token.as_deref(), Some("bobsub"));
    assert_eq!(user(&plan, "shared@home").sub_token, None);
    assert!(has_warning(&plan, "user", "shared@home", "shared_sub_id"));
    assert!(has_warning(&plan, "user", "hy@home", "no_inbound"));

    let sub = plan.subscription.as_ref().unwrap();
    assert_eq!(
        (
            sub.path.as_deref(),
            sub.json_path.as_deref(),
            sub.clash_path.as_deref()
        ),
        (Some("/sub"), Some("/json"), None)
    );
    assert_eq!(
        (sub.title.as_deref(), sub.update_hours),
        (Some("My VPN"), Some(6))
    );
    assert!(plan.legacy_secret.is_none(), "its links are tokens");
}

#[tokio::test]
async fn threexui_with_a_clients_table_is_read_the_same_way() {
    let plan = plan_of(V3).await;
    assert!(has_warning(&plan, "inbound", "node2-443", "remote_inbound"));
    assert_eq!(plan.inbounds.len(), 3);
    assert_eq!(
        inbound(&plan, "inbound-443").settings["flow"],
        "xtls-rprx-vision",
        "from the table's clients"
    );

    let ali = user(&plan, "ali@home");
    assert_eq!(
        (ali.reset_strategy.as_str(), ali.note.as_str()),
        ("month", "friend · telegram: 12345 · group: vip")
    );
    assert_eq!(ali.group_names.len(), 1);
    assert_eq!(
        group(&plan, &ali.group_names[0]).inbound_tags,
        ["inbound-443", "inbound-8443"]
    );
    assert_eq!(
        user(&plan, "weekly@home").reset_strategy,
        "week",
        "renews on a weekday"
    );
    let hwid = user(&plan, "hwid@home");
    assert_eq!(hwid.hwid_limit, 3);
    assert_eq!(
        group(&plan, &hwid.group_names[0]).inbound_tags,
        ["inbound-8443"],
        "the node's inbound is left out"
    );
    assert!(has_warning(
        &plan,
        "user",
        "hwid@home",
        "reset_approximated"
    ));
    assert_eq!(
        plan.subscription.as_ref().unwrap().path.as_deref(),
        Some("/s")
    );
}

// ---- through the API

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::server::testing::{panel, Panel, Reply};

async fn world() -> (Panel, String) {
    let panel = panel("/").await;
    let owner = panel.set_up("").await;
    (panel, owner)
}

async fn upload(panel: &Panel, owner: &str, db: &TempDb, query: &str) -> Reply {
    panel
        .upload(
            &format!("/api/migrate{query}"),
            &[("cookie", owner)],
            db.bytes(),
        )
        .await
}

async fn apply(panel: &Panel, owner: &str, id: &str, options: Value) -> Reply {
    panel
        .send(
            "POST",
            &format!("/api/migrate/{id}/apply"),
            Some(owner),
            Some(options),
        )
        .await
}

async fn imported(panel: &Panel, owner: &str, script: &str, options: Value) -> Value {
    let db = TempDb::new(script).await;
    let up = upload(panel, owner, &db, "").await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.body);
    let id = up.json()["id"].as_str().unwrap().to_string();
    let done = apply(panel, owner, &id, options).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    done.json()
}

/// Turn the imported inbounds on in the database, as the admin does once the
/// old panel has stopped: the subscription is built from what is enabled.
async fn enable_all(panel: &Panel) {
    let db = panel.state.db();
    for inbound in db.inbounds().await.unwrap() {
        let fields = z_db::InboundFields {
            tag: inbound.tag,
            remark: inbound.remark,
            enabled: true,
            listen: inbound.listen,
            port: inbound.port,
            protocol: inbound.protocol,
            settings: inbound.settings,
            stream: inbound.stream,
            sniffing: inbound.sniffing,
        };
        db.update_inbound(inbound.id, &fields).await.unwrap();
    }
}

async fn fetch(panel: &Panel, path: &str) -> Reply {
    panel.fetch(path, &[("user-agent", "v2rayN/6.0")]).await
}

#[tokio::test]
async fn a_pasarguard_database_is_previewed_imported_and_its_links_keep_working() {
    let (panel, owner) = world().await;
    let db = TempDb::new(PASARGUARD).await;

    // Only a sudo admin may; a stranger is turned away before the file is read.
    let stranger = panel.upload("/api/migrate", &[], db.bytes()).await;
    assert_eq!(stranger.status, StatusCode::UNAUTHORIZED);

    let up = upload(&panel, &owner, &db, "").await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.body);
    let analysis = up.json();
    assert_eq!(analysis["kind"], "pasarguard");
    assert_eq!(analysis["summary"]["users"], 6);
    assert_eq!(analysis["summary"]["inbounds"], 4);
    assert_eq!(analysis["summary"]["links"], "signed");
    assert_eq!(analysis["needs_xray_config"], false);
    let id = analysis["id"].as_str().unwrap().to_string();
    assert_eq!(
        panel.state.db().all_users().await.unwrap().len(),
        0,
        "nothing is written by a preview"
    );
    let again = panel.get(&format!("/api/migrate/{id}"), Some(&owner)).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.json()["summary"]["users"], 6);

    let done = apply(&panel, &owner, &id, json!({})).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let report = done.json();
    for (key, want) in [
        ("users", 6),
        ("inbounds", 4),
        ("groups", 3),
        ("templates", 1),
        ("hosts", 2),
    ] {
        assert_eq!(report["imported"][key], want, "{key}: {}", done.body);
    }
    assert!(report["imported"]["usage_hours"].as_u64().unwrap() >= 3);
    assert_eq!(report["links"], "signed");
    assert!(
        report["backup"]
            .as_str()
            .unwrap()
            .ends_with("-before-import.zbk"),
        "{}",
        report["backup"]
    );
    let backups = panel.get("/api/backups", Some(&owner)).await.json();
    assert!(backups["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["before_import"] == true));
    // The upload is gone once it is imported.
    assert_eq!(
        panel
            .get(&format!("/api/migrate/{id}"), Some(&owner))
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // What the users had.
    let db = panel.state.db();
    let users = db.all_users().await.unwrap();
    let alice = users.iter().find(|u| u.username == "alice").unwrap();
    assert_eq!(
        (alice.data_limit, alice.used_traffic, alice.lifetime_traffic),
        (10 * GIB, 3 * GIB, 3 * GIB)
    );
    assert_eq!(
        (alice.uuid.as_str(), alice.password.as_str()),
        ("00000001-aaaa-4bbb-8ccc-000000000001", "trojan-pass-1")
    );
    assert_eq!(
        (
            alice.created_at,
            alice.expire_at,
            alice.reset_strategy.as_str()
        ),
        (1_768_037_400, Some(1_803_859_200), "month")
    );
    assert_eq!(
        users
            .iter()
            .find(|u| u.username == "bob_sh")
            .unwrap()
            .status,
        "on_hold"
    );
    let usage = db.user_usage(alice.id, 0).await.unwrap();
    assert_eq!(
        usage.iter().map(|p| p.downlink).sum::<i64>(),
        1_342_177_280 + 671_088_640,
        "two hours, master and node added"
    );
    // The inbounds come in switched off: the old panel still holds their ports.
    assert!(db.inbounds().await.unwrap().iter().all(|i| !i.enabled));

    // Every kind of link the old panel gave still shows the same user.
    enable_all(&panel).await;
    let file: Tokens =
        serde_json::from_str(include_str!("fixtures/pasarguard.tokens.json")).unwrap();
    let token = |name: &str| {
        file.tokens
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .token
            .clone()
    };
    let native = fetch(&panel, &format!("/sub/{}?format=links", alice.sub_token)).await;
    assert_eq!(native.status, StatusCode::OK, "{}", native.body);
    for name in ["v3_alice", "jwt_alice"] {
        let reply = fetch(&panel, &format!("/sub/{}?format=links", token(name))).await;
        assert_eq!(reply.status, StatusCode::OK, "{name}: {}", reply.body);
        assert_eq!(reply.body, native.body, "{name} shows alice's subscription");
        assert!(reply.headers["subscription-userinfo"]
            .to_str()
            .unwrap()
            .contains("total=10737418240"));
    }
    assert!(
        native
            .body
            .contains("vless://00000001-aaaa-4bbb-8ccc-000000000001@"),
        "{}",
        native.body
    );
    assert!(
        native.body.contains("flow=xtls-rprx-vision") && native.body.contains("pbk="),
        "{}",
        native.body
    );
    assert!(
        native.body.contains("trojan://trojan-pass-1@"),
        "{}",
        native.body
    );
    // The other formats the old links asked for by a last path part.
    let json = fetch(&panel, &format!("/sub/{}/json", token("v3_alice"))).await;
    assert_eq!(json.status, StatusCode::OK);
    assert!(
        serde_json::from_str::<Value>(&json.body).is_ok(),
        "{}",
        json.body
    );

    assert_eq!(
        fetch(&panel, &format!("/sub/{}", token("legacy_carol")))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        fetch(&panel, &format!("/sub/{}", token("legacy_carol_hex")))
            .await
            .status,
        StatusCode::OK
    );
    // A link made before dave's subscription was revoked is void, one after is not.
    assert_eq!(
        fetch(&panel, &format!("/sub/{}", token("v3_dave_before_revoke")))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fetch(&panel, &format!("/sub/{}", token("v3_dave_after_revoke")))
            .await
            .status,
        StatusCode::OK
    );
    for name in [
        "v3_wrong_key",
        "jwt_expired",
        "jwt_not_a_subscription",
        "legacy_carol_tampered",
    ] {
        assert_eq!(
            fetch(&panel, &format!("/sub/{}", token(name))).await.status,
            StatusCode::NOT_FOUND,
            "{name}"
        );
    }
    assert_eq!(
        fetch(&panel, "/sub/not-a-link-of-anyone").await.status,
        StatusCode::NOT_FOUND
    );

    // Revoking in this panel retires the old link as well.
    let revoked = panel
        .send(
            "POST",
            &format!("/api/users/{}/revoke", alice.id),
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.body);
    assert_eq!(
        fetch(&panel, &format!("/sub/{}", token("v3_alice")))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fetch(&panel, &format!("/sub/{}", token("jwt_alice")))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let audit = panel.get("/api/audit", Some(&owner)).await.json();
    let actions: Vec<&str> = audit["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["action"].as_str())
        .collect();
    assert!(
        actions.contains(&"migrate.upload") && actions.contains(&"migrate.apply"),
        "{actions:?}"
    );
}

#[tokio::test]
async fn a_threexui_import_keeps_every_subscription_link_where_it_was() {
    let (panel, owner) = world().await;
    let report = imported(&panel, &owner, CLASSIC, json!({})).await;
    assert_eq!(report["imported"]["users"], 10);
    assert_eq!(report["imported"]["inbounds"], 4);
    assert_eq!(
        report["imported"]["certificates"], 1,
        "the trojan inbound's certificate went to the store"
    );
    assert_eq!(report["links"], "tokens");
    let codes: Vec<&str> = report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|w| w["code"].as_str())
        .collect();
    assert!(
        codes.contains(&"unsupported_protocol") && codes.contains(&"shared_sub_id"),
        "{codes:?}"
    );

    let db = panel.state.db();
    let trojan = db
        .inbounds()
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.tag == "inbound-2096")
        .unwrap();
    let stream: Value = serde_json::from_str(&trojan.stream).unwrap();
    assert!(
        stream["tlsSettings"]["certificateId"].as_i64().is_some(),
        "{stream}"
    );
    assert!(stream["tlsSettings"].get("certificates").is_none());
    let certificates = db.certificates().await.unwrap();
    assert_eq!(
        (certificates.len(), certificates[0].domains.as_str()),
        (1, "trojan.example.com")
    );

    // The subscription settings of the old panel are this one's.
    let settings = panel.get("/api/settings/subscription", Some(&owner)).await;
    let settings = if settings.status == StatusCode::OK {
        settings.json()
    } else {
        json!(serde_json::to_value(panel.state.subscription()).unwrap())
    };
    assert_eq!(settings["title"], "My VPN");
    assert_eq!(settings["update_hours"], 6);
    assert_eq!(settings["legacy_json_path"], "/json");
    assert_eq!(settings["legacy_clash_path"], "");

    enable_all(&panel).await;
    // `/sub/<subId>`: the same address, the same token.
    let plain = fetch(&panel, "/sub/ali0sub1id?format=links").await;
    assert_eq!(plain.status, StatusCode::OK, "{}", plain.body);
    assert!(
        plain.body.contains("00000001-1111-4222-8333-000000000001"),
        "{}",
        plain.body
    );
    assert!(plain.headers["subscription-userinfo"]
        .to_str()
        .unwrap()
        .contains("download=3221225472"));
    assert_eq!(plain.headers["profile-update-interval"], "6");
    // `/json/<subId>`: 3X-UI's JSON subscription is the Xray format here.
    let json_sub = fetch(&panel, "/json/ali0sub1id").await;
    assert_eq!(json_sub.status, StatusCode::OK, "{}", json_sub.body);
    assert!(serde_json::from_str::<Value>(&json_sub.body).is_ok());
    // Clash was off there, and this path is nothing here.
    for path in ["/clash/ali0sub1id", "/json/nobody"] {
        // Not a subscription (the dashboard's page answers any other path).
        assert!(
            fetch(&panel, path)
                .await
                .headers
                .get("subscription-userinfo")
                .is_none(),
            "{path}"
        );
    }
    // The user that shared a link got a new one; the one who had it first kept it.
    assert_eq!(fetch(&panel, "/sub/bobsub").await.status, StatusCode::OK);
    let shared = db
        .all_users()
        .await
        .unwrap()
        .into_iter()
        .find(|u| u.username == "shared@home")
        .unwrap();
    assert_ne!(shared.sub_token, "bobsub");
    // Limited, expired and disabled users are still answered, with no configs.
    let limited = fetch(&panel, "/sub/quotasub?format=links").await;
    assert_eq!((limited.status, limited.body.trim()), (StatusCode::OK, ""));
}

#[tokio::test]
async fn a_marzban_import_needs_its_config_file_and_keeps_its_signed_links() {
    let (panel, owner) = world().await;
    let db = TempDb::new(MARZBAN).await;
    let up = upload(&panel, &owner, &db, "").await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.body);
    let id = up.json()["id"].as_str().unwrap().to_string();
    assert_eq!(up.json()["kind"], "marzban");
    assert_eq!(
        (
            up.json()["needs_xray_config"].clone(),
            up.json()["summary"]["inbounds"].clone()
        ),
        (json!(true), json!(0))
    );

    let not_json = panel
        .upload_as(
            "PUT",
            &format!("/api/migrate/{id}/xray-config"),
            &[("cookie", &owner)],
            b"nope".to_vec(),
        )
        .await;
    assert_eq!(not_json.json()["error"], "invalid_config");
    let wrong = panel
        .upload_as(
            "PUT",
            &format!("/api/migrate/{id}/xray-config"),
            &[("cookie", &owner)],
            b"{\"log\":{}}".to_vec(),
        )
        .await;
    assert_eq!(wrong.json()["error"], "invalid_config");
    let given = panel
        .upload_as(
            "PUT",
            &format!("/api/migrate/{id}/xray-config"),
            &[("cookie", &owner)],
            MARZBAN_XRAY.as_bytes().to_vec(),
        )
        .await;
    assert_eq!(given.status, StatusCode::OK, "{}", given.body);
    assert_eq!(
        (
            given.json()["needs_xray_config"].clone(),
            given.json()["summary"]["inbounds"].clone()
        ),
        (json!(false), json!(4))
    );

    let done = apply(&panel, &owner, &id, json!({})).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert_eq!(done.json()["imported"]["users"], 4);
    assert_eq!(done.json()["links"], "signed");

    enable_all(&panel).await;
    let file: Value = serde_json::from_str(include_str!("fixtures/marzban.tokens.json")).unwrap();
    for token in file["tokens"].as_array().unwrap() {
        let reply = fetch(
            &panel,
            &format!("/sub/{}", token["token"].as_str().unwrap()),
        )
        .await;
        let want = if token["valid"] == true {
            StatusCode::OK
        } else {
            StatusCode::NOT_FOUND
        };
        assert_eq!(reply.status, want, "{}", token["name"]);
    }
    let zed = fetch(
        &panel,
        &format!(
            "/sub/{}?format=links",
            file["tokens"][0]["token"].as_str().unwrap()
        ),
    )
    .await;
    assert!(
        zed.body.contains("vmess://")
            && zed
                .body
                .contains("vless://00000001-2222-4333-8444-000000000001@"),
        "{}",
        zed.body
    );
    // wei's VLESS proxy excluded the only VLESS inbound: only VMess is its.
    let wei = fetch(
        &panel,
        &format!(
            "/sub/{}?format=links",
            file["tokens"][3]["token"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(
        wei.body.matches("://").count(),
        0,
        "wei is disabled, so it gets no configs: {}",
        wei.body
    );
}

#[tokio::test]
async fn importing_again_adds_what_is_new_and_leaves_what_is_there() {
    let (panel, owner) = world().await;
    imported(&panel, &owner, CLASSIC, json!({})).await;
    let db = panel.state.db();
    let (users, inbounds, groups) = (
        db.all_users().await.unwrap().len(),
        db.inbounds().await.unwrap().len(),
        db.access_groups().await.unwrap().len(),
    );

    let second = imported(&panel, &owner, CLASSIC, json!({})).await;
    assert_eq!(second["imported"]["users"], 0);
    assert_eq!(
        (
            second["imported"]["inbounds"].clone(),
            second["imported"]["groups"].clone()
        ),
        (json!(0), json!(0))
    );
    let codes: Vec<&str> = second["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|w| w["code"].as_str())
        .collect();
    assert!(codes.contains(&"skipped_exists"), "{codes:?}");
    assert_eq!(db.all_users().await.unwrap().len(), users);
    assert_eq!(
        db.inbounds().await.unwrap().len(),
        inbounds,
        "the same inbound is not made twice"
    );
    assert_eq!(db.access_groups().await.unwrap().len(), groups);

    // Asked to rename, a user whose name is taken comes in under another.
    let third = imported(
        &panel,
        &owner,
        CLASSIC,
        json!({"on_conflict": "rename", "inbounds": false}),
    )
    .await;
    assert_eq!(third["imported"]["users"], 10);
    assert!(db
        .all_users()
        .await
        .unwrap()
        .iter()
        .any(|u| u.username == "ali@home-2"));
    // Its subscription token was taken, so it has a new one.
    assert_eq!(db.all_users().await.unwrap().len(), users + 10);
}

#[tokio::test]
async fn what_is_not_asked_for_is_not_imported() {
    let (panel, owner) = world().await;
    let report = imported(
        &panel,
        &owner,
        PASARGUARD,
        json!({"users": false, "hosts": false, "templates": false, "usage": false, "settings": false}),
    )
    .await;
    assert_eq!(
        (
            report["imported"]["users"].clone(),
            report["imported"]["hosts"].clone(),
            report["imported"]["templates"].clone()
        ),
        (json!(0), json!(0), json!(0))
    );
    assert_eq!(report["imported"]["inbounds"], 4);
    assert_eq!(
        report["links"], "tokens",
        "no users, so no old link to keep"
    );
    assert!(panel.state.db().all_users().await.unwrap().is_empty());
    assert!(panel
        .state
        .db()
        .legacy_secret(legacy::KIND)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn bad_uploads_are_refused_and_leave_nothing_behind() {
    let (panel, owner) = world().await;
    let junk = panel
        .upload(
            "/api/migrate",
            &[("cookie", &owner)],
            b"just some text".to_vec(),
        )
        .await;
    assert_eq!(
        (junk.status, junk.json()["error"].clone()),
        (StatusCode::BAD_REQUEST, json!("not_a_database"))
    );
    let empty = panel
        .upload("/api/migrate", &[("cookie", &owner)], Vec::new())
        .await;
    assert_eq!(empty.json()["error"], "empty_upload");
    let other = TempDb::new("CREATE TABLE notes (id integer, body text);").await;
    let unknown = upload(&panel, &owner, &other, "").await;
    assert_eq!(unknown.json()["error"], "unknown_panel");
    let bad_kind = upload(&panel, &owner, &other, "?kind=wordpress").await;
    assert_eq!(bad_kind.json()["error"], "unknown_panel");
    let folder = panel.state.config().data_dir.join("migrate");
    assert_eq!(
        std::fs::read_dir(&folder).map(|d| d.count()).unwrap_or(0),
        0,
        "refused files are removed"
    );

    // An id is only what an upload made: nothing a path can reach.
    for id in [
        "../../etc/passwd",
        "..%2f..%2fx",
        "0123456789abcdef0123456789abcdef",
        "ABCDEF0123456789ABCDEF0123456789",
    ] {
        let reply = panel.get(&format!("/api/migrate/{id}"), Some(&owner)).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{id}");
        assert_eq!(
            apply(&panel, &owner, id, json!({})).await.status,
            StatusCode::NOT_FOUND,
            "{id}"
        );
    }

    // A good upload can be thrown away.
    let db = TempDb::new(PASARGUARD).await;
    let id = upload(&panel, &owner, &db, "").await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
    let gone = panel
        .send("DELETE", &format!("/api/migrate/{id}"), Some(&owner), None)
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.body);
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
}
