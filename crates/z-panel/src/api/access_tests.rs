//! Who may do what: roles, resellers and their quotas, API keys and the
//! audit log, tried through the API as the dashboard and scripts use it.

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::server::testing::{panel, Panel, Reply};

const GB: i64 = 1 << 30;
const PASSWORD: &str = "a-reseller-password";

/// A panel with its first admin signed in.
async fn world() -> (Panel, String) {
    let panel = panel("/").await;
    let owner = panel.set_up("").await;
    (panel, owner)
}

fn id_of(list: &Value, name: &str) -> i64 {
    list.as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == name)
        .unwrap_or_else(|| panic!("no {name} in {list}"))["id"]
        .as_i64()
        .unwrap()
}

async fn role(panel: &Panel, owner: &str, name: &str) -> i64 {
    let roles = panel.get("/api/roles", Some(owner)).await.json();
    id_of(&roles["roles"], name)
}

/// An admin made by `owner`, signed in: `(id, session)`.
async fn admin(panel: &Panel, owner: &str, username: &str, extra: Value) -> (i64, String) {
    let mut body = json!({"username": username, "password": PASSWORD});
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let made = panel
        .send("POST", "/api/admins", Some(owner), Some(body))
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let session = panel.sign_in(username, PASSWORD).await;
    (made.json()["id"].as_i64().unwrap(), session)
}

async fn reseller(panel: &Panel, owner: &str, username: &str, extra: Value) -> (i64, String) {
    let role = role(panel, owner, "Reseller").await;
    let mut body = json!({"role_id": role});
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    admin(panel, owner, username, body).await
}

/// A request made with an API key.
async fn with(panel: &Panel, key: &str, method: &str, path: &str, body: Option<Value>) -> Reply {
    let bearer = format!("Bearer {key}");
    panel
        .request(method, path, &[("authorization", bearer.as_str())], body)
        .await
}

fn user(username: &str, groups: &[i64], data_limit: i64) -> Value {
    json!({"username": username, "group_ids": groups, "data_limit": data_limit})
}

async fn error(reply: Reply, status: StatusCode, code: &str) {
    assert_eq!(reply.status, status, "{}", reply.body);
    assert_eq!(reply.json()["error"], code, "{}", reply.body);
}

#[tokio::test]
async fn roles_are_made_checked_and_kept_while_admins_have_them() {
    let (panel, owner) = world().await;
    let roles = panel.get("/api/roles", Some(&owner)).await.json();
    assert_eq!(roles["catalog"].as_array().unwrap().len(), 12);
    assert_eq!(
        roles["catalog"][10],
        json!({"section": "audit", "writable": false})
    );
    let names: Vec<&str> = roles["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|role| role["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Reseller", "Operator", "Viewer"]);
    assert_eq!(
        roles["roles"][0]["permissions"],
        json!([
            "users.read",
            "users.write",
            "groups.read",
            "templates.read",
            "api.read",
            "api.write"
        ]),
        "writing counts as reading too"
    );
    assert_eq!(roles["roles"][0]["own_users_only"], true);

    for (body, code) in [
        (json!({"name": "", "permissions": []}), "invalid_name"),
        (
            json!({"name": "x", "permissions": ["teleport.write"]}),
            "invalid_permission",
        ),
        (
            json!({"name": "x", "permissions": ["audit.write"]}),
            "invalid_permission",
        ),
        (json!({"name": "Reseller", "permissions": []}), "name_taken"),
    ] {
        let status = if code == "name_taken" {
            StatusCode::CONFLICT
        } else {
            StatusCode::BAD_REQUEST
        };
        error(
            panel
                .send("POST", "/api/roles", Some(&owner), Some(body))
                .await,
            status,
            code,
        )
        .await;
    }
    let made = panel
        .send(
            "POST",
            "/api/roles",
            Some(&owner),
            Some(json!({"name": "Support", "permissions": ["users.read", "hosts.write"]})),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let support = made.json()["id"].as_i64().unwrap();
    assert_eq!(
        made.json()["permissions"],
        json!(["users.read", "hosts.read", "hosts.write"])
    );

    let changed = panel
        .send(
            "PUT",
            &format!("/api/roles/{support}"),
            Some(&owner),
            Some(json!({"name": "Support", "permissions": ["logs.read"], "own_users_only": true})),
        )
        .await
        .json();
    assert_eq!(changed["permissions"], json!(["logs.read"]));
    assert_eq!(changed["own_users_only"], true);
    error(
        panel
            .send(
                "PUT",
                "/api/roles/999",
                Some(&owner),
                Some(json!({"name": "Ghost", "permissions": []})),
            )
            .await,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;

    // A role in use stays; an unused one goes.
    admin(&panel, &owner, "helper", json!({"role_id": support})).await;
    error(
        panel
            .send(
                "DELETE",
                &format!("/api/roles/{support}"),
                Some(&owner),
                None,
            )
            .await,
        StatusCode::CONFLICT,
        "role_in_use",
    )
    .await;
    let spare = panel
        .send(
            "POST",
            "/api/roles",
            Some(&owner),
            Some(json!({"name": "Spare", "permissions": []})),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let gone = panel
        .send("DELETE", &format!("/api/roles/{spare}"), Some(&owner), None)
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT);
    let roles = panel.get("/api/roles", Some(&owner)).await.json();
    assert_eq!(roles["roles"].as_array().unwrap().len(), 4);
    let in_use = roles["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["name"] == "Support")
        .unwrap();
    assert_eq!(in_use["admins"], 1);
}

#[tokio::test]
async fn admins_are_made_protected_disabled_and_removed_by_a_sudo_admin() {
    let (panel, owner) = world().await;
    let role = role(&panel, &owner, "Reseller").await;
    for (extra, status, code) in [
        (
            json!({"password": "short", "role_id": role}),
            StatusCode::BAD_REQUEST,
            "weak_password",
        ),
        (
            json!({"role_id": null}),
            StatusCode::BAD_REQUEST,
            "role_required",
        ),
        (
            json!({"role_id": 999}),
            StatusCode::BAD_REQUEST,
            "invalid_role",
        ),
        (
            json!({"role_id": role, "group_ids": [999]}),
            StatusCode::BAD_REQUEST,
            "invalid_group",
        ),
        (
            json!({"role_id": role, "user_limit": -1}),
            StatusCode::BAD_REQUEST,
            "invalid_limit",
        ),
        (
            json!({"username": "a b", "role_id": role}),
            StatusCode::BAD_REQUEST,
            "invalid_username",
        ),
    ] {
        let mut body = json!({"username": "shop", "password": PASSWORD});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        error(
            panel
                .send("POST", "/api/admins", Some(&owner), Some(body))
                .await,
            status,
            code,
        )
        .await;
    }
    let (shop_id, shop) = reseller(
        &panel,
        &owner,
        "shop",
        json!({"user_limit": 3, "data_quota": 10 * GB, "note": "ali's shop"}),
    )
    .await;
    error(
        panel
            .send(
                "POST",
                "/api/admins",
                Some(&owner),
                Some(json!({"username": "shop", "password": PASSWORD, "role_id": role})),
            )
            .await,
        StatusCode::CONFLICT,
        "username_taken",
    )
    .await;

    // The log says in words what the admin was made.
    let log = panel
        .get("/api/audit?action=admin.", Some(&owner))
        .await
        .json();
    let made = log["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "admin.create" && e["target"] == "shop")
        .unwrap_or_else(|| panic!("{log}"));
    assert_eq!(
        made["detail"],
        "role Reseller, enabled, users: 3, data: 10.0 GB"
    );

    // What the dashboard learns about whoever signed in.
    let me = panel.get("/api/auth/me", Some(&shop)).await.json();
    assert_eq!(me["role"]["name"], "Reseller");
    assert_eq!(me["is_sudo"], false);
    assert_eq!(me["own_users_only"], true);
    assert_eq!(me["quota"]["user_limit"], 3);
    assert_eq!(me["quota"]["data_quota"], 10 * GB);
    assert!(me["permissions"]
        .as_array()
        .unwrap()
        .contains(&json!("users.write")));
    assert!(!me["permissions"]
        .as_array()
        .unwrap()
        .contains(&json!("core.read")));
    let boss = panel.get("/api/auth/me", Some(&owner)).await.json();
    assert_eq!(
        boss["permissions"].as_array().unwrap().len(),
        23,
        "everything but audit.write"
    );
    let owner_id = boss["id"].as_i64().unwrap();

    // Only a sudo admin gets to these.
    for (method, path) in [
        ("GET", "/api/admins"),
        ("POST", "/api/admins"),
        ("PUT", "/api/admins/1"),
        ("DELETE", "/api/admins/1"),
        ("GET", "/api/roles"),
        ("POST", "/api/roles"),
    ] {
        error(
            panel.send(method, path, Some(&shop), Some(json!({}))).await,
            StatusCode::FORBIDDEN,
            "forbidden",
        )
        .await;
    }
    let listed = panel.get("/api/admins", Some(&owner)).await.json();
    assert_eq!(listed.as_array().unwrap().len(), 2);
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["username"] == "shop")
        .unwrap();
    assert_eq!(row["role"], "Reseller");
    assert_eq!(row["note"], "ali's shop");
    assert_eq!(
        (row["users"].as_i64(), row["unlimited"].as_i64()),
        (Some(0), Some(0))
    );

    // Nobody locks themselves out.
    let edit = |extra: Value| {
        let mut body = json!({"is_sudo": true, "enabled": true});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    };
    for body in [
        edit(json!({"is_sudo": false, "role_id": role})),
        edit(json!({"enabled": false})),
    ] {
        error(
            panel
                .send(
                    "PUT",
                    &format!("/api/admins/{owner_id}"),
                    Some(&owner),
                    Some(body),
                )
                .await,
            StatusCode::CONFLICT,
            "cannot_change_self",
        )
        .await;
    }
    error(
        panel
            .send(
                "DELETE",
                &format!("/api/admins/{owner_id}"),
                Some(&owner),
                None,
            )
            .await,
        StatusCode::CONFLICT,
        "cannot_change_self",
    )
    .await;

    // Off: signed out at once, and kept out.
    let off = panel
        .send(
            "PUT",
            &format!("/api/admins/{shop_id}"),
            Some(&owner),
            Some(json!({"role_id": role, "enabled": false, "user_limit": 3})),
        )
        .await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.body);
    assert_eq!(off.json()["enabled"], false);
    assert_eq!(
        panel.get("/api/auth/me", Some(&shop)).await.status,
        StatusCode::UNAUTHORIZED
    );
    error(
        panel
            .send(
                "POST",
                "/api/auth/login",
                None,
                Some(json!({"username": "shop", "password": PASSWORD})),
            )
            .await,
        StatusCode::FORBIDDEN,
        "account_disabled",
    )
    .await;
    let on = panel
        .send(
            "PUT",
            &format!("/api/admins/{shop_id}"),
            Some(&owner),
            Some(json!({"role_id": role, "enabled": true, "password": "a-new-long-password"})),
        )
        .await;
    assert_eq!(on.status, StatusCode::OK, "{}", on.body);
    assert_eq!(on.json()["user_limit"], 0, "a PUT sets everything it names");
    let shop = panel.sign_in("shop", "a-new-long-password").await;
    assert_eq!(
        panel.get("/api/auth/me", Some(&shop)).await.status,
        StatusCode::OK
    );

    // Signed out on request, and a lost authenticator replaced.
    let out = panel
        .send(
            "POST",
            &format!("/api/admins/{shop_id}/sign-out"),
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert_eq!(
        panel.get("/api/auth/me", Some(&shop)).await.status,
        StatusCode::UNAUTHORIZED
    );
    error(
        panel
            .send(
                "DELETE",
                &format!("/api/admins/{shop_id}/2fa"),
                Some(&owner),
                None,
            )
            .await,
        StatusCode::BAD_REQUEST,
        "totp_disabled",
    )
    .await;
    panel
        .state
        .db()
        .set_admin_totp(shop_id, Some("JBSWY3DPEHPK3PXP"))
        .await
        .unwrap();
    error(
        panel
            .send(
                "POST",
                "/api/auth/login",
                None,
                Some(json!({"username": "shop", "password": "a-new-long-password"})),
            )
            .await,
        StatusCode::UNAUTHORIZED,
        "totp_required",
    )
    .await;
    let reset = panel
        .send(
            "DELETE",
            &format!("/api/admins/{shop_id}/2fa"),
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(reset.status, StatusCode::NO_CONTENT);
    let shop = panel.sign_in("shop", "a-new-long-password").await;

    // Removed, the admin's users go to whoever is named.
    let default = panel.get("/api/groups", Some(&owner)).await.json()[0]["id"].clone();
    let made = panel
        .send(
            "POST",
            "/api/users",
            Some(&shop),
            Some(json!({"username": "carol", "group_ids": [default]})),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let carol = made.json()["id"].as_i64().unwrap();
    assert_eq!(made.json()["owner"], "shop");
    error(
        panel
            .send(
                "DELETE",
                &format!("/api/admins/{shop_id}?transfer_to=999"),
                Some(&owner),
                None,
            )
            .await,
        StatusCode::BAD_REQUEST,
        "invalid_heir",
    )
    .await;
    error(
        panel
            .send(
                "DELETE",
                &format!("/api/admins/{shop_id}?transfer_to={shop_id}"),
                Some(&owner),
                None,
            )
            .await,
        StatusCode::BAD_REQUEST,
        "invalid_heir",
    )
    .await;
    let gone = panel
        .send(
            "DELETE",
            &format!("/api/admins/{shop_id}"),
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.body);
    let carol = panel
        .get(&format!("/api/users/{carol}"), Some(&owner))
        .await
        .json();
    assert_eq!(
        carol["admin_id"], owner_id,
        "the user went to who deleted the admin"
    );
    assert_eq!(
        panel.get("/api/auth/me", Some(&shop)).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_reseller_sees_only_their_users_and_is_held_to_their_quotas() {
    let (panel, owner) = world().await;
    let default = panel.get("/api/groups", Some(&owner)).await.json()[0]["id"]
        .as_i64()
        .unwrap();
    let vip = panel
        .send(
            "POST",
            "/api/groups",
            Some(&owner),
            Some(json!({"name": "vip", "inbound_ids": []})),
        )
        .await;
    assert_eq!(vip.status, StatusCode::CREATED, "{}", vip.body);
    let vip = vip.json()["id"].as_i64().unwrap();
    let (shop_id, shop) = reseller(
        &panel,
        &owner,
        "shop",
        json!({"user_limit": 3, "data_quota": 10 * GB, "group_ids": [default]}),
    )
    .await;

    // The quotas.
    let created = |name: &str, limit: i64| {
        panel.send(
            "POST",
            "/api/users",
            Some(&shop),
            Some(user(name, &[default], limit)),
        )
    };
    let u1 = created("usr1", 4 * GB).await;
    assert_eq!(u1.status, StatusCode::CREATED, "{}", u1.body);
    assert_eq!(
        (u1.json()["owner"].as_str(), u1.json()["admin_id"].as_i64()),
        (Some("shop"), Some(shop_id))
    );
    let u1 = u1.json()["id"].as_i64().unwrap();
    error(
        created("nolimit", 0).await,
        StatusCode::BAD_REQUEST,
        "quota_unlimited",
    )
    .await;
    error(
        created("toobig", 7 * GB).await,
        StatusCode::FORBIDDEN,
        "quota_exceeded",
    )
    .await;
    let u2 = created("usr2", 6 * GB).await;
    assert_eq!(u2.status, StatusCode::CREATED, "{}", u2.body);
    let u2 = u2.json()["id"].as_i64().unwrap();
    error(
        created("usr3", GB).await,
        StatusCode::FORBIDDEN,
        "quota_exceeded",
    )
    .await;
    let held = panel.get("/api/auth/me", Some(&shop)).await.json();
    assert_eq!(held["quota"]["users"], 2);
    assert_eq!(held["quota"]["allocated"], 10 * GB);

    // Raising a limit costs the difference; lowering frees it.
    let put = |id: i64, limit: i64| {
        let path = format!("/api/users/{id}");
        let body = user("usr1", &[default], limit);
        let (panel, shop) = (&panel, shop.clone());
        async move { panel.send("PUT", &path, Some(&shop), Some(body)).await }
    };
    error(
        put(u1, 5 * GB).await,
        StatusCode::FORBIDDEN,
        "quota_exceeded",
    )
    .await;
    assert_eq!(put(u1, 3 * GB).await.status, StatusCode::OK);
    let u3 = created("usr3", GB).await;
    assert_eq!(u3.status, StatusCode::CREATED, "{}", u3.body);
    let u3 = u3.json()["id"].as_i64().unwrap();
    error(
        created("usr4", 1).await,
        StatusCode::FORBIDDEN,
        "user_limit_reached",
    )
    .await;
    let bulk = panel
        .send(
            "POST",
            "/api/users/bulk",
            Some(&shop),
            Some(json!({"action": "add_data", "bytes": 10 * GB, "ids": [u1, u2]})),
        )
        .await;
    assert_eq!(
        (
            bulk.json()["changed"].as_i64(),
            bulk.json()["skipped"].as_i64()
        ),
        (Some(0), Some(2)),
        "{}",
        bulk.body
    );
    let many = panel
        .send(
            "POST",
            "/api/users/bulk-create",
            Some(&shop),
            Some(json!({"prefix": "guest", "count": 2, "group_ids": [default], "data_limit": GB})),
        )
        .await;
    error(many, StatusCode::FORBIDDEN, "user_limit_reached").await;
    // A sudo admin is held to none of it.
    let big = panel
        .send(
            "PUT",
            &format!("/api/users/{u1}"),
            Some(&owner),
            Some(user("usr1", &[default], 50 * GB)),
        )
        .await;
    assert_eq!(big.status, StatusCode::OK, "{}", big.body);

    // What belongs to someone else is not there.
    let mine = panel
        .send(
            "POST",
            "/api/users",
            Some(&owner),
            Some(user("ownerguy", &[default, vip], 0)),
        )
        .await
        .json();
    let theirs = mine["id"].as_i64().unwrap();
    for (method, path, body) in [
        ("GET", format!("/api/users/{theirs}"), None),
        (
            "PUT",
            format!("/api/users/{theirs}"),
            Some(user("ownerguy", &[default], 0)),
        ),
        ("DELETE", format!("/api/users/{theirs}"), None),
        ("POST", format!("/api/users/{theirs}/reset"), None),
        ("POST", format!("/api/users/{theirs}/revoke"), None),
        ("POST", format!("/api/users/{theirs}/end-sessions"), None),
        ("GET", format!("/api/users/{theirs}/usage"), None),
        ("GET", format!("/api/users/{theirs}/online"), None),
        ("GET", format!("/api/users/{theirs}/subscription"), None),
        ("GET", format!("/api/users/{theirs}/devices"), None),
        ("DELETE", format!("/api/users/{theirs}/devices"), None),
        ("DELETE", format!("/api/users/{theirs}/devices/1"), None),
    ] {
        error(
            panel.send(method, &path, Some(&shop), body).await,
            StatusCode::NOT_FOUND,
            "not_found",
        )
        .await;
    }
    let page = panel.get("/api/users", Some(&shop)).await.json();
    assert_eq!(page["total"], 3);
    assert!(page["users"]
        .as_array()
        .unwrap()
        .iter()
        .all(|u| u["owner"] == "shop"));
    let asked_for_all = panel
        .get(&format!("/api/users?owner={}", 12345), Some(&shop))
        .await
        .json();
    assert_eq!(
        asked_for_all["total"], 3,
        "a reseller cannot ask for another admin's users"
    );
    assert_eq!(
        panel.get("/api/users", Some(&owner)).await.json()["total"],
        4
    );
    let by_owner = panel
        .get(&format!("/api/users?owner={shop_id}"), Some(&owner))
        .await
        .json();
    assert_eq!(by_owner["total"], 3);
    assert_eq!(
        panel.get("/api/users/counts", Some(&shop)).await.json()["total"],
        3
    );
    assert_eq!(
        panel.get("/api/users/counts", Some(&owner)).await.json()["total"],
        4
    );
    let deleted = panel
        .send(
            "POST",
            "/api/users/bulk",
            Some(&shop),
            Some(json!({"action": "delete", "ids": [theirs, u3]})),
        )
        .await;
    assert_eq!(
        deleted.json()["changed"],
        1,
        "only the one that is theirs: {}",
        deleted.body
    );
    assert_eq!(
        panel
            .get(&format!("/api/users/{theirs}"), Some(&owner))
            .await
            .status,
        StatusCode::OK
    );

    // What the traffic charts add up.
    panel
        .state
        .db()
        .add_traffic(&[
            z_db::TrafficDelta {
                user_id: u1,
                uplink: 100,
                downlink: 0,
            },
            z_db::TrafficDelta {
                user_id: theirs,
                uplink: 5000,
                downlink: 0,
            },
        ])
        .await
        .unwrap();
    let theirs_total = panel.get("/api/monitor/traffic", Some(&owner)).await.json();
    let own_total = panel.get("/api/monitor/traffic", Some(&shop)).await.json();
    assert_eq!(theirs_total["lifetime"], 5100);
    assert_eq!(own_total["lifetime"], 100);
    assert_eq!(own_total["top"].as_array().unwrap().len(), 1);

    // Groups: only the allowed ones are shown or given, and the others stay.
    let groups = panel.get("/api/groups", Some(&shop)).await.json();
    assert_eq!(groups.as_array().unwrap().len(), 1);
    assert_eq!(groups[0]["id"], default);
    let denied = panel
        .send(
            "PUT",
            &format!("/api/users/{u2}"),
            Some(&shop),
            Some(user("usr2", &[default, vip], 6 * GB)),
        )
        .await;
    error(denied, StatusCode::FORBIDDEN, "group_not_allowed").await;
    panel
        .send(
            "PUT",
            &format!("/api/users/{u2}"),
            Some(&owner),
            Some(user("usr2", &[default, vip], 6 * GB)),
        )
        .await;
    let kept = panel
        .send(
            "PUT",
            &format!("/api/users/{u2}"),
            Some(&shop),
            Some(user("usr2", &[default], 6 * GB)),
        )
        .await;
    assert_eq!(kept.status, StatusCode::OK, "{}", kept.body);
    assert_eq!(
        kept.json()["group_ids"],
        json!([default, vip]),
        "a group the reseller cannot see is not taken away"
    );
    let moved = panel
        .send(
            "POST",
            "/api/users/bulk",
            Some(&shop),
            Some(json!({"action": "set_groups", "group_ids": [default], "ids": [u2]})),
        )
        .await;
    assert_eq!(moved.json()["changed"], 1);
    assert_eq!(
        panel
            .get(&format!("/api/users/{u2}"), Some(&owner))
            .await
            .json()["group_ids"],
        json!([default, vip])
    );
    error(
        panel
            .send(
                "POST",
                "/api/users/bulk",
                Some(&shop),
                Some(json!({"action": "set_groups", "group_ids": [vip], "ids": [u2]})),
            )
            .await,
        StatusCode::FORBIDDEN,
        "group_not_allowed",
    )
    .await;

    // The rest of the panel is not theirs.
    for path in [
        "/api/inbounds",
        "/api/hosts",
        "/api/certificates",
        "/api/core",
        "/api/core/geodata",
        "/api/monitor/system",
        "/api/logs",
        "/api/logs/level",
        "/api/settings/subscription",
        "/api/audit",
        "/api/tools/reality-check?host=example.com",
    ] {
        error(
            panel.get(path, Some(&shop)).await,
            StatusCode::FORBIDDEN,
            "forbidden",
        )
        .await;
    }
    for path in [
        "/api/health",
        "/api/groups",
        "/api/templates",
        "/api/keys/uuid",
        "/api/api-keys",
    ] {
        assert_eq!(
            panel.get(path, Some(&shop)).await.status,
            StatusCode::OK,
            "{path}"
        );
    }
    for (method, path) in [
        ("POST", "/api/groups"),
        ("POST", "/api/templates"),
        ("POST", "/api/inbounds"),
        ("POST", "/api/core/validate"),
        ("PUT", "/api/settings/subscription"),
    ] {
        error(
            panel.send(method, path, Some(&shop), Some(json!({}))).await,
            StatusCode::FORBIDDEN,
            "forbidden",
        )
        .await;
    }
}

#[tokio::test]
async fn an_api_key_acts_as_its_admin_with_no_more_than_it_was_given() {
    let (panel, owner) = world().await;
    let make = |body: Value| panel.send("POST", "/api/api-keys", Some(&owner), Some(body));

    // Made once; only the start of it is shown again.
    let full = make(json!({"name": "ci", "permissions": "*"})).await;
    assert_eq!(full.status, StatusCode::CREATED, "{}", full.body);
    let full = full.json();
    let secret = full["key"].as_str().unwrap().to_string();
    assert!(secret.starts_with("zk_") && secret.len() == 46);
    assert_eq!(full["full"], true);
    let listed = panel.get("/api/api-keys", Some(&owner)).await;
    assert!(!listed.body.contains(&secret), "{}", listed.body);
    assert_eq!(listed.json()[0]["prefix"], secret[..11]);
    let with_key = |method: &'static str, path: &'static str, body: Option<Value>| {
        with(&panel, &secret, method, path, body)
    };
    assert_eq!(
        with_key("GET", "/api/users", None).await.status,
        StatusCode::OK
    );
    assert_eq!(
        with_key("GET", "/api/auth/me", None).await.json()["username"],
        "owner"
    );
    let by_header = panel
        .request("GET", "/api/users", &[("x-api-key", secret.as_str())], None)
        .await;
    assert_eq!(by_header.status, StatusCode::OK);
    assert_eq!(
        with_key("GET", "/api/admins", None).await.status,
        StatusCode::OK,
        "a full key of a sudo manages admins"
    );
    let used = panel.get("/api/api-keys", Some(&owner)).await.json();
    assert!(used[0]["last_used_at"].is_number(), "{used}");
    assert_eq!(used[0]["last_used_ip"], "0.0.0.0");

    // Things done to oneself need a sign-in, never a key.
    for (method, path) in [
        ("GET", "/api/api-keys"),
        ("POST", "/api/api-keys"),
        ("POST", "/api/auth/password"),
        ("POST", "/api/auth/2fa/begin"),
    ] {
        error(
            with_key(method, path, Some(json!({"name": "x", "permissions": "*"}))).await,
            StatusCode::FORBIDDEN,
            "session_required",
        )
        .await;
    }

    // A limited key does what it names and nothing else, even for a sudo admin.
    let limited = make(json!({"name": "reader", "permissions": ["users.read", "hosts.read"]}))
        .await
        .json();
    let reader = limited["key"].as_str().unwrap().to_string();
    let read = |method: &'static str, path: &'static str| {
        with(&panel, &reader, method, path, Some(json!({})))
    };
    assert_eq!(read("GET", "/api/users").await.status, StatusCode::OK);
    assert_eq!(read("GET", "/api/hosts").await.status, StatusCode::OK);
    error(
        read("POST", "/api/users").await,
        StatusCode::FORBIDDEN,
        "forbidden",
    )
    .await;
    error(
        read("GET", "/api/inbounds").await,
        StatusCode::FORBIDDEN,
        "forbidden",
    )
    .await;
    error(
        read("GET", "/api/admins").await,
        StatusCode::FORBIDDEN,
        "forbidden",
    )
    .await;
    assert_eq!(limited["permissions"], json!(["users.read", "hosts.read"]));

    // Nobody gives a key more than they have.
    let (_, shop) = reseller(&panel, &owner, "shop", json!({})).await;
    let over = panel
        .send(
            "POST",
            "/api/api-keys",
            Some(&shop),
            Some(json!({"name": "greedy", "permissions": ["core.read"]})),
        )
        .await;
    error(over, StatusCode::FORBIDDEN, "permission_exceeds").await;
    let fine = panel
        .send(
            "POST",
            "/api/api-keys",
            Some(&shop),
            Some(json!({"name": "mine", "permissions": ["users.write"]})),
        )
        .await;
    assert_eq!(fine.status, StatusCode::CREATED, "{}", fine.body);
    let shop_key = format!("Bearer {}", fine.json()["key"].as_str().unwrap());
    let created = panel
        .request(
            "POST",
            "/api/users",
            &[("authorization", shop_key.as_str())],
            Some(user("viakey", &[], GB)),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(
        created.json()["owner"],
        "shop",
        "the key's work is its admin's"
    );
    let own = panel
        .request(
            "GET",
            "/api/users",
            &[("authorization", shop_key.as_str())],
            None,
        )
        .await;
    assert_eq!(own.json()["total"], 1);

    // Checked when made.
    for (body, code) in [
        (json!({"name": "", "permissions": "*"}), "invalid_name"),
        (
            json!({"name": "x", "permissions": "everything"}),
            "invalid_permission",
        ),
        (
            json!({"name": "x", "permissions": []}),
            "invalid_permission",
        ),
        (
            json!({"name": "x", "permissions": ["nothing.read"]}),
            "invalid_permission",
        ),
        (
            json!({"name": "x", "permissions": "*", "allowed_ips": ["not an address"]}),
            "invalid_address",
        ),
        (
            json!({"name": "x", "permissions": "*", "expires_at": 5}),
            "invalid_expiry",
        ),
    ] {
        error(make(body).await, StatusCode::BAD_REQUEST, code).await;
    }

    // Where from, and until when.
    let fenced = make(json!({"name": "fenced", "permissions": "*", "allowed_ips": ["10.0.0.0/8"]}))
        .await
        .json();
    let fenced = format!("Bearer {}", fenced["key"].as_str().unwrap());
    error(
        panel
            .request(
                "GET",
                "/api/users",
                &[("authorization", fenced.as_str())],
                None,
            )
            .await,
        StatusCode::FORBIDDEN,
        "ip_not_allowed",
    )
    .await;
    let anywhere = make(json!({"name": "open", "permissions": "*", "allowed_ips": ["0.0.0.0/0"], "expires_at": z_db::now() + 3600})).await.json();
    let anywhere_key = format!("Bearer {}", anywhere["key"].as_str().unwrap());
    assert_eq!(
        panel
            .request(
                "GET",
                "/api/users",
                &[("authorization", anywhere_key.as_str())],
                None
            )
            .await
            .status,
        StatusCode::OK
    );
    let (stale, _, hash) = crate::auth::keys::generate();
    panel
        .state
        .db()
        .create_api_key(
            1,
            "stale",
            "zk_stale",
            &hash,
            "*",
            "",
            Some(z_db::now() - 5),
        )
        .await
        .unwrap();
    error(
        panel
            .request(
                "GET",
                "/api/users",
                &[("authorization", format!("Bearer {stale}").as_str())],
                None,
            )
            .await,
        StatusCode::UNAUTHORIZED,
        "api_key_expired",
    )
    .await;
    let listed = panel.get("/api/api-keys", Some(&owner)).await.json();
    assert_eq!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .find(|k| k["name"] == "stale")
            .unwrap()["expired"],
        true
    );

    // Revoked, it stops at once; another admin cannot revoke it.
    let first = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["name"] == "ci")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    error(
        panel
            .send(
                "DELETE",
                &format!("/api/api-keys/{first}"),
                Some(&shop),
                None,
            )
            .await,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    let revoked = panel
        .send(
            "DELETE",
            &format!("/api/api-keys/{first}"),
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    error(
        with_key("GET", "/api/users", None).await,
        StatusCode::UNAUTHORIZED,
        "invalid_api_key",
    )
    .await;

    // A disabled admin's keys stop with them.
    let shop_id = panel.get("/api/auth/me", Some(&shop)).await.json()["id"]
        .as_i64()
        .unwrap();
    let role = role(&panel, &owner, "Reseller").await;
    panel
        .send(
            "PUT",
            &format!("/api/admins/{shop_id}"),
            Some(&owner),
            Some(json!({"role_id": role, "enabled": false})),
        )
        .await;
    assert_eq!(
        panel
            .request(
                "GET",
                "/api/users",
                &[("authorization", shop_key.as_str())],
                None
            )
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );

    // Guessing keys locks the guesser out of keys, not out of signing in:
    // the revoked key above was the first miss, so four more make five.
    for n in 0..4 {
        let guess = format!("Bearer zk_{n:0>43}");
        error(
            panel
                .request(
                    "GET",
                    "/api/users",
                    &[("authorization", guess.as_str())],
                    None,
                )
                .await,
            StatusCode::UNAUTHORIZED,
            "invalid_api_key",
        )
        .await;
    }
    let locked = panel
        .request(
            "GET",
            "/api/users",
            &[("authorization", "Bearer zk_nothing")],
            None,
        )
        .await;
    error(locked, StatusCode::TOO_MANY_REQUESTS, "too_many_attempts").await;
    assert_eq!(
        panel.get("/api/users", Some(&owner)).await.status,
        StatusCode::OK
    );
    assert_eq!(
        panel
            .request(
                "GET",
                "/api/users",
                &[("authorization", anywhere_key.as_str())],
                None
            )
            .await
            .status,
        StatusCode::TOO_MANY_REQUESTS,
        "even a right key waits while the address is locked"
    );
}

#[tokio::test]
async fn what_admins_do_is_written_down_and_found_again() {
    let (panel, owner) = world().await;
    let default = panel.get("/api/groups", Some(&owner)).await.json()[0]["id"].clone();
    let (_, shop) = reseller(&panel, &owner, "shop", json!({})).await;
    let (_, viewer) = admin(
        &panel,
        &owner,
        "watcher",
        json!({"role_id": role(&panel, &owner, "Viewer").await}),
    )
    .await;

    let alice = panel
        .send(
            "POST",
            "/api/users",
            Some(&owner),
            Some(json!({"username": "alice", "group_ids": [default]})),
        )
        .await
        .json();
    panel
        .send(
            "POST",
            &format!("/api/users/{}/reset", alice["id"]),
            Some(&owner),
            None,
        )
        .await;
    panel
        .send(
            "DELETE",
            &format!("/api/users/{}", alice["id"]),
            Some(&owner),
            None,
        )
        .await;
    // Refused, and the refusal is written too.
    let refused = panel
        .send(
            "POST",
            "/api/inbounds",
            Some(&shop),
            Some(json!({"remark": "x"})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    // A wrong password, from a name that exists and one that does not.
    for name in ["owner", "ghost-with-a-secret"] {
        panel
            .send(
                "POST",
                "/api/auth/login",
                None,
                Some(json!({"username": name, "password": "wrong-password"})),
            )
            .await;
    }
    // Reading is not written.
    panel.get("/api/users", Some(&owner)).await;
    let key = panel
        .send(
            "POST",
            "/api/api-keys",
            Some(&owner),
            Some(json!({"name": "bot", "permissions": "*"})),
        )
        .await
        .json();
    let bearer = format!("Bearer {}", key["key"].as_str().unwrap());
    panel
        .request(
            "POST",
            "/api/users",
            &[("authorization", bearer.as_str())],
            Some(json!({"username": "bob", "group_ids": [default]})),
        )
        .await;

    let log = panel.get("/api/audit?limit=100", Some(&owner)).await.json();
    let entries = log["entries"].as_array().unwrap();
    let find = |action: &str, target: &str| {
        entries
            .iter()
            .find(|e| e["action"] == action && e["target"] == target)
            .unwrap_or_else(|| panic!("no {action} {target} in {log}"))
    };
    let created = find("user.create", "alice");
    assert_eq!(
        (
            created["admin"].as_str(),
            created["via"].as_str(),
            created["status"].as_i64()
        ),
        (Some("owner"), Some("session"), Some(201))
    );
    assert_eq!(created["ip"], "0.0.0.0");
    find("user.reset", "alice");
    assert_eq!(find("user.delete", "alice")["status"], 204);
    let by_key = find("user.create", "bob");
    assert_eq!(
        (by_key["via"].as_str(), by_key["admin"].as_str()),
        (Some("key:bot"), Some("owner"))
    );
    find("api_key.create", "bot");
    find("auth.setup", "owner");
    assert_eq!(find("auth.login", "shop")["status"], 200);

    let denied = entries
        .iter()
        .find(|e| e["admin"] == "shop" && e["status"] == 403)
        .unwrap_or_else(|| panic!("the refusal is not in {log}"));
    assert_eq!(denied["action"], "inbounds.post");
    assert_eq!(denied["detail"], "forbidden");
    let wrong = find("auth.login", "owner");
    assert_eq!(
        (wrong["status"].as_i64(), wrong["detail"].as_str()),
        (Some(401), Some("wrong password"))
    );
    find("auth.login", "(unknown user)");
    assert!(
        !log.to_string().contains("ghost-with-a-secret"),
        "a name nobody has may be a password typed in the wrong box"
    );
    assert!(
        entries.iter().all(|e| e["action"] != "users.get"),
        "reading is not written"
    );

    // Found again.
    let ask = |query: &str| {
        let path = format!("/api/audit?{query}");
        let panel = &panel;
        let owner = owner.clone();
        async move { panel.get(&path, Some(&owner)).await.json() }
    };
    let users_only = ask("action=user.&limit=100").await;
    assert!(users_only["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["action"].as_str().unwrap().starts_with("user.")));
    assert_eq!(
        users_only["entries"].as_array().unwrap().len(),
        4,
        "{users_only}"
    );
    let alice_only = ask("q=ALICE&limit=100").await;
    assert_eq!(alice_only["entries"].as_array().unwrap().len(), 3);
    let failed = ask("failed=true&limit=100").await;
    assert!(failed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["status"].as_i64().unwrap() >= 400));
    assert_eq!(failed["entries"].as_array().unwrap().len(), 3, "{failed}");
    let shop_id = panel.get("/api/auth/me", Some(&shop)).await.json()["id"]
        .as_i64()
        .unwrap();
    let shops = ask(&format!("admin={shop_id}&limit=100")).await;
    assert!(shops["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["admin"] == "shop"));

    let first = ask("limit=3").await;
    assert_eq!(first["entries"].as_array().unwrap().len(), 3);
    let next = first["next"].as_i64().expect("there is more");
    let second = ask(&format!("limit=3&before={next}")).await;
    let seen: Vec<i64> = first["entries"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["entries"].as_array().unwrap())
        .map(|e| e["id"].as_i64().unwrap())
        .collect();
    assert!(
        seen.windows(2).all(|pair| pair[0] > pair[1]),
        "newest first, no repeats: {seen:?}"
    );

    // Whoever may read it, may.
    assert_eq!(
        panel.get("/api/audit", Some(&viewer)).await.status,
        StatusCode::OK
    );
    error(
        panel.get("/api/audit", Some(&shop)).await,
        StatusCode::FORBIDDEN,
        "forbidden",
    )
    .await;
}

/// Every operation of the API document, as `(method, path, operation)`.
fn operations() -> Vec<(&'static str, String, Value)> {
    let doc = serde_json::to_value(super::spec()).unwrap();
    let mut found = Vec::new();
    for (path, item) in doc["paths"].as_object().unwrap() {
        for method in ["get", "put", "post", "delete", "patch"] {
            if let Some(operation) = item.get(method) {
                found.push((method, path.clone(), operation.clone()));
            }
        }
    }
    found
}

/// A path of the document with its parameters filled in.
fn concrete(path: &str) -> String {
    format!("/api{}", path.replace("{id}", "1").replace("{device}", "1"))
}

#[test]
fn the_document_describes_every_operation_and_what_it_needs() {
    let doc = serde_json::to_value(super::spec()).unwrap();
    assert_eq!(doc["info"]["title"], "Z-UI API");
    let operations = operations();
    assert!(operations.len() >= 80, "{} operations", operations.len());
    let declared: Vec<&str> = doc["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tag| tag["name"].as_str().unwrap())
        .collect();
    for (method, path, operation) in &operations {
        let label = format!("{method} {path}");
        assert!(operation["summary"].is_string(), "{label} has no summary");
        let tag = operation["tags"][0]
            .as_str()
            .unwrap_or_else(|| panic!("{label} has no tag"));
        assert!(
            declared.contains(&tag),
            "{label}: tag {tag} is not declared"
        );
        assert!(
            operation["responses"]
                .as_object()
                .is_some_and(|r| r.len() >= 2),
            "{label}"
        );
    }
    let mut ids: Vec<&str> = operations
        .iter()
        .map(|(_, _, operation)| operation["operationId"].as_str().expect("an operation id"))
        .collect();
    let all = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(
        ids.len(),
        all,
        "operation ids repeat; clients are generated from them"
    );
    // The permission an operation needs comes from the guard its route has.
    let needs = |method: &str, path: &str| {
        operations
            .iter()
            .find(|(m, p, _)| *m == method && p == path)
            .unwrap_or_else(|| panic!("no {method} {path}"))
            .2["x-permission"]
            .as_str()
            .map(str::to_owned)
    };
    assert_eq!(needs("get", "/users").as_deref(), Some("users.read"));
    assert_eq!(needs("post", "/users").as_deref(), Some("users.write"));
    assert_eq!(
        needs("delete", "/users/{id}").as_deref(),
        Some("users.write")
    );
    assert_eq!(
        needs("get", "/monitor/traffic").as_deref(),
        Some("users.read")
    );
    assert_eq!(
        needs("get", "/monitor/system").as_deref(),
        Some("monitor.read")
    );
    assert_eq!(
        needs("post", "/core/validate").as_deref(),
        Some("core.read")
    );
    assert_eq!(
        needs("get", "/tools/reality-check").as_deref(),
        Some("inbounds.write")
    );
    assert_eq!(needs("get", "/admins").as_deref(), Some("sudo"));
    assert_eq!(needs("post", "/auth/password").as_deref(), Some("session"));
    assert_eq!(
        needs("post", "/api-keys").as_deref(),
        Some("session + api.write")
    );
    assert_eq!(needs("get", "/audit").as_deref(), Some("audit.read"));
    assert_eq!(needs("get", "/auth/me"), None);
    assert!(operations
        .iter()
        .find(|(m, p, _)| *m == "get" && p == "/users")
        .unwrap()
        .2["description"]
        .as_str()
        .unwrap()
        .starts_with("**Needs:** `users.read`"));

    // Every schema a response or request names is in the document.
    let schemas = doc["components"]["schemas"].as_object().unwrap();
    let text = doc.to_string();
    for reference in text.split("\"$ref\":\"#/components/schemas/").skip(1) {
        let name = reference.split('"').next().unwrap();
        assert!(schemas.contains_key(name), "no schema {name}");
    }
    for name in [
        "UserView",
        "UserInput",
        "ErrorBody",
        "Me",
        "KeyView",
        "AuditPage",
        "Changed_InboundView",
    ] {
        assert!(schemas.contains_key(name), "no schema {name}");
    }
    let schemes = doc["components"]["securitySchemes"].as_object().unwrap();
    assert_eq!(schemes["bearer"]["scheme"], "bearer");
    assert_eq!(schemes["session"]["name"], "z_ui_session");
    assert_eq!(schemes["api_key"]["name"], "X-API-Key");
}

#[tokio::test]
async fn no_operation_lets_in_anyone_it_should_not() {
    let (panel, owner) = world().await;
    let none = panel
        .send(
            "POST",
            "/api/roles",
            Some(&owner),
            Some(json!({"name": "Nothing", "permissions": []})),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let (_, nobody) = admin(&panel, &owner, "nobody", json!({"role_id": none})).await;
    for (method, path, operation) in operations() {
        let label = format!("{method} {path}");
        let url = concrete(&path);
        let body = || Some(json!({}));
        let open = operation["security"] == json!([{}]);
        let anonymous = panel.send(&method.to_uppercase(), &url, None, body()).await;
        if open {
            assert_ne!(
                anonymous.status,
                StatusCode::FORBIDDEN,
                "{label}: {}",
                anonymous.body
            );
            continue;
        }
        assert_eq!(
            anonymous.status,
            StatusCode::UNAUTHORIZED,
            "{label} is open to anyone: {}",
            anonymous.body
        );
        let needs = operation["x-permission"].as_str();
        let signed_in = panel
            .send(&method.to_uppercase(), &url, Some(&nobody), body())
            .await;
        match needs {
            // Only a sign-in is asked, so a sign-in gets as far as the handler.
            None | Some("session") => assert!(
                !matches!(
                    signed_in.status,
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                ),
                "{label}: {}",
                signed_in.body
            ),
            Some(needs) => {
                assert_eq!(
                    signed_in.status,
                    StatusCode::FORBIDDEN,
                    "{label} needs {needs} but an admin with no permissions got in: {}",
                    signed_in.body
                );
                assert_eq!(signed_in.json()["error"], "forbidden", "{label}");
            }
        }
    }
}

#[tokio::test]
async fn the_document_is_served_to_whoever_is_signed_in_with_this_panels_address() {
    let (panel, owner) = world().await;
    assert_eq!(
        panel.get("/api/openapi.json", None).await.status,
        StatusCode::UNAUTHORIZED
    );
    let reply = panel
        .request(
            "GET",
            "/api/openapi.json",
            &[("cookie", owner.as_str()), ("host", "panel.example:2053")],
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let doc = reply.json();
    assert_eq!(doc["servers"][0]["url"], "http://panel.example:2053/api");
    assert!(doc["paths"]["/users"]["get"].is_object());
    let (_, shop) = reseller(&panel, &owner, "shop", json!({})).await;
    assert_eq!(
        panel.get("/api/openapi.json", Some(&shop)).await.status,
        StatusCode::OK
    );
}
