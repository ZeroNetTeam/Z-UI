//! Users as the core's accounts.
//!
//! A user reaches the inbounds its access groups give, while its status lets
//! it connect (active, or on hold waiting for its first connection). Its
//! username is the account's name in the core, so traffic, online sessions
//! and speed limits are all counted under it.
//!
//! How changes reach the core: the whole configuration ([`crate::core_config`])
//! carries every account, and is rebuilt when inbounds change. A change to one
//! user is applied on its own instead ([`sync_user`]): its accounts are added
//! to or removed from the inbounds concerned while the core runs, which
//! rebuilds nothing and leaves every other session alone.

use std::collections::HashMap;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use z_db::{AccessGroup, Inbound, User};

use crate::error::ApiError;
use crate::server::AppState;

/// Statuses whose accounts are in the core.
pub fn connectable(user: &User) -> bool {
    matches!(user.status.as_str(), "active" | "on_hold")
}

/// Whether `user`'s groups give it `inbound`.
pub fn grants(groups: &[AccessGroup], user: &User, inbound: &Inbound) -> bool {
    groups.iter().any(|group| {
        user.group_ids.contains(&group.id)
            && (group.all_inbounds || group.inbound_ids.contains(&inbound.id))
    })
}

/// A Shadowsocks 2022 key for a user: `bytes` derived from its password, in
/// standard base64, so it changes when the password does and needs no column
/// of its own.
pub fn shadowsocks_2022_key(password: &str, bytes: usize) -> String {
    let digest = Sha256::new()
        .chain_update(b"z-ui shadowsocks 2022 user key\0")
        .chain_update(password.as_bytes())
        .finalize();
    base64(&digest[..bytes])
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n =
            chunk.iter().fold(0u32, |acc, &b| (acc << 8) | u32::from(b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// The account `user` has on `inbound`, in the shape of an entry of its
/// `settings.clients` (or `users`). `None` for a protocol whose accounts the
/// panel does not make yet (WireGuard peers).
pub fn client(inbound: &Inbound, user: &User) -> Option<Value> {
    let settings: Value = serde_json::from_str(&inbound.settings).unwrap_or_default();
    let email = user.username.as_str();
    Some(match inbound.protocol.as_str() {
        "vless" => {
            let mut client = json!({"id": user.uuid, "email": email});
            if let Some(flow) = settings["flow"].as_str().filter(|flow| !flow.is_empty()) {
                client["flow"] = json!(flow);
            }
            client
        }
        "vmess" => json!({"id": user.uuid, "email": email}),
        "trojan" | "hysteria2" | "anytls" => json!({"password": user.password, "email": email}),
        "tuic" => json!({"uuid": user.uuid, "password": user.password, "email": email}),
        "shadowsocks" => {
            let method = settings["method"].as_str().unwrap_or_default();
            let password = match method {
                "2022-blake3-aes-128-gcm" => shadowsocks_2022_key(&user.password, 16),
                m if m.starts_with("2022-") => shadowsocks_2022_key(&user.password, 32),
                _ => user.password.clone(),
            };
            json!({"password": password, "email": email})
        }
        _ => return None,
    })
}

/// Every account `inbound` carries, for building the whole configuration.
pub fn clients_of(inbound: &Inbound, users: &[User], groups: &[AccessGroup]) -> Vec<Value> {
    users
        .iter()
        .filter(|user| connectable(user) && grants(groups, user, inbound))
        .filter_map(|user| client(inbound, user))
        .collect()
}

fn refused(error: z_common::node::NodeError) -> ApiError {
    ApiError::bad_request("core_refused", error.to_string())
}

/// Bring the core in line with one user's change: `before` as it was (None
/// when new), `after` as it is now (None when deleted). The caller holds the
/// core lock. Nothing happens while the core is not running; the next whole
/// configuration carries the user.
pub async fn sync_user(
    state: &AppState,
    before: Option<&User>,
    after: Option<&User>,
) -> Result<(), ApiError> {
    if !state.node().status().await.running {
        return Ok(());
    }
    let inbounds = state.db().inbounds().await?;
    let groups = state.db().access_groups().await?;
    let wants = |user: Option<&User>, inbound: &Inbound| {
        user.is_some_and(|user| connectable(user) && grants(&groups, user, inbound))
    };
    for inbound in inbounds.iter().filter(|inbound| inbound.enabled) {
        // A renamed user is a different account in the core.
        let renamed = matches!((before, after), (Some(b), Some(a)) if b.username != a.username);
        if wants(before, inbound) && (renamed || !wants(after, inbound)) {
            let old = before.map(|user| user.username.clone()).unwrap_or_default();
            state
                .node()
                .remove_accounts(&inbound.tag, &[old])
                .await
                .map_err(refused)?;
        }
        if let Some(user) = after.filter(|_| wants(after, inbound)) {
            if let Some(client) = client(inbound, user) {
                // Replaces the account of the same name; a changed
                // credential ends the sessions opened with the old one.
                state
                    .node()
                    .add_accounts(&inbound.tag, &[client])
                    .await
                    .map_err(refused)?;
            }
        }
    }
    if let Some(before) = before {
        if after.is_none_or(|after| after.username != before.username) {
            state.node().set_limit(&before.username, 0, 0).await;
        }
    }
    if let Some(user) = after {
        state
            .node()
            .set_limit(
                &user.username,
                user.speed_up.max(0) as u64,
                user.speed_down.max(0) as u64,
            )
            .await;
    }
    Ok(())
}

/// Give the core every user's speed limit, after it starts.
pub async fn apply_limits(state: &AppState) -> Result<(), ApiError> {
    for user in state.db().all_users().await? {
        if user.speed_up > 0 || user.speed_down > 0 {
            state
                .node()
                .set_limit(&user.username, user.speed_up as u64, user.speed_down as u64)
                .await;
        }
    }
    Ok(())
}

/// Users by username, for matching the core's figures back to them.
pub fn by_username(users: &[User]) -> HashMap<&str, &User> {
    users
        .iter()
        .map(|user| (user.username.as_str(), user))
        .collect()
}

#[cfg(test)]
pub mod tests_support {
    use super::*;

    /// A user with everything at its default.
    pub fn user(name: &str, status: &str) -> User {
        User {
            id: 1,
            username: name.into(),
            status: status.into(),
            uuid: "5783a3e7-e373-51cd-8642-c83782b807c5".into(),
            password: "pw".into(),
            sub_token: "t".into(),
            data_limit: 0,
            used_traffic: 0,
            lifetime_traffic: 0,
            expire_at: None,
            on_hold_duration: None,
            on_hold_timeout: None,
            reset_strategy: "no_reset".into(),
            last_reset_at: None,
            ip_limit: 0,
            speed_up: 0,
            speed_down: 0,
            note: String::new(),
            online_at: None,
            admin_id: None,
            created_at: 0,
            updated_at: 0,
            group_ids: Vec::new(),
            next_plan: None,
            hwid_limit: 0,
            sub_updated_at: None,
            sub_user_agent: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str, status: &str, groups: Vec<i64>) -> User {
        User {
            group_ids: groups,
            ..super::tests_support::user(name, status)
        }
    }

    fn inbound(id: i64, protocol: &str, settings: Value) -> Inbound {
        Inbound {
            id,
            tag: format!("in-{id}"),
            remark: String::new(),
            enabled: true,
            listen: String::new(),
            port: 1000 + id,
            protocol: protocol.into(),
            settings: settings.to_string(),
            stream: "{}".into(),
            sniffing: "{}".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn group(id: i64, all: bool, inbounds: Vec<i64>) -> AccessGroup {
        AccessGroup {
            id,
            name: format!("g{id}"),
            all_inbounds: all,
            inbound_ids: inbounds,
            created_at: 0,
        }
    }

    #[test]
    fn groups_decide_the_inbounds() {
        let groups = [group(1, true, vec![]), group(2, false, vec![7])];
        let everywhere = user("a", "active", vec![1]);
        let one = user("b", "active", vec![2]);
        let nowhere = user("c", "active", vec![]);
        let seven = inbound(7, "vless", json!({}));
        let eight = inbound(8, "vless", json!({}));
        assert!(grants(&groups, &everywhere, &eight));
        assert!(grants(&groups, &one, &seven) && !grants(&groups, &one, &eight));
        assert!(!grants(&groups, &nowhere, &seven));
        let users = [everywhere, one, nowhere, user("d", "limited", vec![1])];
        let names: Vec<_> = clients_of(&seven, &users, &groups)
            .iter()
            .map(|c| c["email"].clone())
            .collect();
        assert_eq!(
            names,
            [json!("a"), json!("b")],
            "a limited user has no account"
        );
    }

    #[test]
    fn each_protocol_gets_its_shape() {
        let alice = user("alice", "on_hold", vec![]);
        let vision = inbound(1, "vless", json!({"flow": "xtls-rprx-vision"}));
        assert_eq!(
            client(&vision, &alice).unwrap(),
            json!({"id": alice.uuid, "email": "alice", "flow": "xtls-rprx-vision"})
        );
        assert_eq!(
            client(&inbound(2, "trojan", json!({})), &alice).unwrap(),
            json!({"password": "pw", "email": "alice"})
        );
        assert_eq!(
            client(&inbound(3, "tuic", json!({})), &alice).unwrap(),
            json!({"uuid": alice.uuid, "password": "pw", "email": "alice"})
        );
        let ss16 = client(
            &inbound(
                4,
                "shadowsocks",
                json!({"method": "2022-blake3-aes-128-gcm"}),
            ),
            &alice,
        )
        .unwrap();
        assert_eq!(
            ss16["password"].as_str().unwrap().len(),
            24,
            "16 bytes in base64"
        );
        let ss32 = client(
            &inbound(
                5,
                "shadowsocks",
                json!({"method": "2022-blake3-aes-256-gcm"}),
            ),
            &alice,
        )
        .unwrap();
        assert_eq!(
            ss32["password"].as_str().unwrap().len(),
            44,
            "32 bytes in base64"
        );
        let legacy = client(
            &inbound(6, "shadowsocks", json!({"method": "aes-256-gcm"})),
            &alice,
        )
        .unwrap();
        assert_eq!(legacy["password"], "pw");
        assert!(client(&inbound(7, "wireguard", json!({})), &alice).is_none());
    }

    #[test]
    fn a_2022_key_compiles_in_the_core() {
        let key = shadowsocks_2022_key("some-password", 16);
        let config = json!({
            "inbounds": [{"tag": "ss", "listen": "127.0.0.1", "port": 1, "protocol": "shadowsocks",
                "settings": {"method": "2022-blake3-aes-128-gcm", "password": shadowsocks_2022_key("server", 16),
                    "clients": [{"password": key, "email": "a"}]}}],
            "outbounds": [{"tag": "direct", "protocol": "freedom"}],
        });
        zero_config::compile_config(&config, zero_core::GenerationId(1)).unwrap();
    }
}
