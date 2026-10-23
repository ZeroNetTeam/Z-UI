//! Notifications, the bot's link and backups, through the API.

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::notify::http::testing::{mock, ok, Mock};
use crate::server::testing::{panel, Panel, Reply};

async fn world() -> (Panel, String) {
    let panel = panel("/").await;
    let owner = panel.set_up("").await;
    panel.state.notifier().retry_after(vec![]);
    (panel, owner)
}

async fn error(reply: Reply, status: StatusCode, code: &str) -> Value {
    assert_eq!(reply.status, status, "{}", reply.body);
    assert_eq!(reply.json()["error"], code, "{}", reply.body);
    reply.json()
}

/// Turn the bot on against a mock Telegram, through the API.
async fn telegram_on(panel: &Panel, owner: &str, telegram: &Mock) {
    let saved = panel
        .send(
            "PUT",
            "/api/notifications",
            Some(owner),
            Some(json!({
                "language": "en",
                "telegram": {"enabled": true, "token": "123:abc", "api_base": telegram.url(), "user_bot": true},
            })),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
}

#[tokio::test]
async fn the_settings_are_saved_checked_and_never_show_their_secrets() {
    let (panel, owner) = world().await;
    let got = panel.get("/api/notifications", Some(&owner)).await.json();
    assert_eq!(got["settings"]["language"], "fa");
    assert_eq!(got["settings"]["telegram"]["token_set"], false);
    assert_eq!(got["bot"]["running"], false);

    let saved = panel
        .send(
            "PUT",
            "/api/notifications",
            Some(&owner),
            Some(json!({
                "language": "en",
                "proxy": "socks5://bot:hunter2@127.0.0.1:10808",
                "telegram": {"enabled": true, "token": "123:abc", "chat_ids": [-1001]},
                "webhook": {"enabled": true, "url": "https://hooks.example/z", "secret": "s3cret"},
                "events": {"data_low_percent": 70},
            })),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    for secret in ["123:abc", "s3cret", "hunter2"] {
        assert!(
            !saved.body.contains(secret),
            "{secret} leaked: {}",
            saved.body
        );
    }
    let view = saved.json();
    assert_eq!(view["settings"]["telegram"]["token_set"], true);
    assert_eq!(view["settings"]["webhook"]["secret_set"], true);
    assert_eq!(view["settings"]["events"]["data_low_percent"], 70);
    assert_eq!(view["settings"]["telegram"]["chat_ids"], json!([-1001]));

    // What the dashboard sends back as shown keeps every secret.
    let again = panel
        .send(
            "PUT",
            "/api/notifications",
            Some(&owner),
            Some(json!({
                "language": "en",
                "proxy": view["settings"]["proxy"],
                "telegram": {"enabled": true, "chat_ids": [-1001]},
                "webhook": {"enabled": true, "url": "https://hooks.example/z"},
            })),
        )
        .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    let kept = panel.state.notify_settings();
    assert_eq!(kept.telegram.token, "123:abc");
    assert_eq!(kept.webhook.secret, "s3cret");
    assert_eq!(kept.proxy, "socks5://bot:hunter2@127.0.0.1:10808");
    // It is stored, so a restart reads it back.
    assert_eq!(crate::notify::load(panel.state.db()).await, kept);

    error(
        panel
            .send(
                "PUT",
                "/api/notifications",
                Some(&owner),
                Some(json!({"language": "de"})),
            )
            .await,
        StatusCode::BAD_REQUEST,
        "invalid_notify",
    )
    .await;
    let log = panel
        .get("/api/audit?action=notifications.", Some(&owner))
        .await
        .json();
    let entries = log["entries"].as_array().unwrap();
    assert!(entries
        .iter()
        .any(|e| e["action"] == "notifications.save" && e["status"] == 200));
    assert!(
        entries.iter().any(|e| e["action"] == "notifications.put"
            && e["status"] == 400
            && e["detail"] == "invalid_notify"),
        "the refused one is written too: {log}"
    );
}

#[tokio::test]
async fn a_test_of_each_channel_says_how_it_went() {
    let (panel, owner) = world().await;
    let telegram = mock(|seen| {
        if seen.path.ends_with("/getMe") {
            (200, r#"{"ok":true,"result":{"username":"zui_bot"}}"#.into())
        } else {
            (200, r#"{"ok":true,"result":{}}"#.into())
        }
    })
    .await;
    error(
        panel
            .send(
                "POST",
                "/api/notifications/test",
                Some(&owner),
                Some(json!({"channel": "telegram"})),
            )
            .await,
        StatusCode::BAD_REQUEST,
        "delivery_failed",
    )
    .await;
    telegram_on(&panel, &owner, &telegram).await;
    let none = error(
        panel
            .send(
                "POST",
                "/api/notifications/test",
                Some(&owner),
                Some(json!({"channel": "telegram"})),
            )
            .await,
        StatusCode::BAD_REQUEST,
        "delivery_failed",
    )
    .await;
    assert!(
        none["message"].as_str().unwrap().contains("no chat"),
        "{none}"
    );

    // The signed-in admin ties their chat, and the test reaches it.
    let linked = panel
        .send("POST", "/api/auth/telegram/link", Some(&owner), None)
        .await;
    assert_eq!(linked.status, StatusCode::OK, "{}", linked.body);
    let code = linked.json()["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 8);
    let update = json!({"update_id": 1, "message": {"message_id": 1, "chat": {"id": 901, "type": "private", "first_name": "Ali"}, "text": format!("/start {code}")}});
    Box::pin(crate::notify::bot::handle(
        &panel.state,
        &panel.state.notify_settings(),
        &update,
    ))
    .await;
    let chats = panel.get("/api/auth/telegram", Some(&owner)).await.json();
    assert_eq!(chats["chats"][0]["chat_id"], 901);
    assert_eq!(chats["enabled"], true);

    let tried = panel
        .send(
            "POST",
            "/api/notifications/test",
            Some(&owner),
            Some(json!({"channel": "telegram"})),
        )
        .await;
    assert_eq!(tried.status, StatusCode::OK, "{}", tried.body);
    assert_eq!(
        (
            tried.json()["delivered"].as_i64(),
            tried.json()["bot"].as_str()
        ),
        (Some(1), Some("zui_bot"))
    );
    let status = panel.get("/api/notifications", Some(&owner)).await.json();
    assert!(
        status["status"]["telegram"]["ok_at"].is_number(),
        "{status}"
    );
    assert_eq!(status["bot"]["username"], "zui_bot");

    // Another admin cannot untie it; its owner can.
    let role = panel.get("/api/roles", Some(&owner)).await.json()["roles"][0]["id"].clone();
    panel
        .send(
            "POST",
            "/api/admins",
            Some(&owner),
            Some(json!({"username": "shop", "password": "a-reseller-password", "role_id": role})),
        )
        .await;
    let shop = panel.sign_in("shop", "a-reseller-password").await;
    error(
        panel
            .send("DELETE", "/api/auth/telegram/901", Some(&shop), None)
            .await,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    let gone = panel
        .send("DELETE", "/api/auth/telegram/901", Some(&owner), None)
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT);
    assert!(
        panel.get("/api/auth/telegram", Some(&owner)).await.json()["chats"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // With the bot off there is nothing to link to.
    panel
        .send(
            "PUT",
            "/api/notifications",
            Some(&owner),
            Some(json!({"language": "en"})),
        )
        .await;
    error(
        panel
            .send("POST", "/api/auth/telegram/link", Some(&owner), None)
            .await,
        StatusCode::BAD_REQUEST,
        "telegram_off",
    )
    .await;
    let discord = ok().await;
    let _ = discord;
}

#[tokio::test]
async fn a_backup_is_made_listed_downloaded_and_deleted() {
    let (panel, owner) = world().await;
    let made = panel.send("POST", "/api/backups", Some(&owner), None).await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let name = made.json()["file"].as_str().unwrap().to_string();
    assert_eq!(
        (
            made.json()["encrypted"].clone(),
            made.json()["sent"].clone()
        ),
        (json!(false), json!(0))
    );

    let listed = panel.get("/api/backups", Some(&owner)).await.json();
    assert_eq!(listed["files"][0]["name"], name.as_str());
    assert!(listed["last_at"].is_number());
    assert_eq!(listed["settings"]["enabled"], false);

    let file = panel
        .get(&format!("/api/backups/{name}"), Some(&owner))
        .await;
    assert_eq!(file.status, StatusCode::OK);
    assert_eq!(
        file.headers.get("content-type").unwrap(),
        "application/octet-stream"
    );
    assert!(file
        .headers
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .contains(&name));
    assert!(file.body.len() > 100);
    for bad in ["x.zbk", "..%2F..%2Fetc%2Fpasswd", "panel.db"] {
        error(
            panel
                .get(&format!("/api/backups/{bad}"), Some(&owner))
                .await,
            StatusCode::NOT_FOUND,
            "not_found",
        )
        .await;
    }
    assert_eq!(
        panel
            .send(
                "DELETE",
                &format!("/api/backups/{name}"),
                Some(&owner),
                None
            )
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    error(
        panel
            .send(
                "DELETE",
                &format!("/api/backups/{name}"),
                Some(&owner),
                None,
            )
            .await,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    assert!(
        panel.get("/api/backups", Some(&owner)).await.json()["files"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn the_schedule_and_the_passphrase_are_set_checked_kept_and_cleared() {
    let (panel, owner) = world().await;
    let put = |body: Value| panel.send("PUT", "/api/backups/settings", Some(&owner), Some(body));
    let base = |extra: Value| {
        let mut body = json!({"enabled": true, "interval_hours": 12, "keep": 5, "send_telegram": false, "include_usage": true});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    };
    let saved = put(base(json!({"passphrase": "correct horse"}))).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    assert!(!saved.body.contains("correct horse"));
    let view = saved.json()["settings"].clone();
    assert_eq!(
        (
            view["interval_hours"].clone(),
            view["keep"].clone(),
            view["passphrase_set"].clone()
        ),
        (json!(12), json!(5), json!(true))
    );
    // Left empty it is kept; made encrypted; cleared on request.
    put(base(json!({}))).await;
    assert_eq!(
        panel.state.notify_settings().backup.passphrase,
        "correct horse"
    );
    let sealed = panel
        .send("POST", "/api/backups", Some(&owner), None)
        .await
        .json();
    assert!(sealed["encrypted"] == true && sealed["file"].as_str().unwrap().ends_with(".zbk.enc"));
    put(base(json!({"clear_passphrase": true}))).await;
    assert_eq!(panel.state.notify_settings().backup.passphrase, "");
    for bad in [
        json!({"interval_hours": 0}),
        json!({"keep": 0}),
        json!({"passphrase": "short"}),
    ] {
        error(
            put(base(bad)).await,
            StatusCode::BAD_REQUEST,
            "invalid_notify",
        )
        .await;
    }
}

#[tokio::test]
async fn a_backup_uploaded_replaces_the_panel_and_signs_everyone_out() {
    // A panel with its own admin and users to take the backup from.
    let (source, source_owner) = world().await;
    let role = source.get("/api/roles", Some(&source_owner)).await.json()["roles"][0]["id"].clone();
    source
        .send(
            "POST",
            "/api/admins",
            Some(&source_owner),
            Some(json!({"username": "shop", "password": "a-reseller-password", "role_id": role})),
        )
        .await;
    let default = source.get("/api/groups", Some(&source_owner)).await.json()[0]["id"].clone();
    source
        .send(
            "POST",
            "/api/users",
            Some(&source_owner),
            Some(json!({"username": "ali", "group_ids": [default], "data_limit": 1000})),
        )
        .await;
    let made = source
        .send("POST", "/api/backups", Some(&source_owner), None)
        .await
        .json();
    let backup = source
        .get(
            &format!("/api/backups/{}", made["file"].as_str().unwrap()),
            Some(&source_owner),
        )
        .await;
    assert_eq!(backup.status, StatusCode::OK);
    let bytes = crate::backup::read(&source.state, made["file"].as_str().unwrap()).unwrap();

    // Another panel, with an admin of its own, takes it.
    let (target, owner) = world().await;
    let anonymous = target
        .upload("/api/backups/restore", &[], bytes.clone())
        .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    let cookie = ("cookie", owner.as_str());
    let junk = target
        .upload("/api/backups/restore", &[cookie], b"not a backup".to_vec())
        .await;
    error(junk, StatusCode::BAD_REQUEST, "invalid_backup").await;
    assert!(
        target
            .get("/api/auth/me", Some(&owner))
            .await
            .status
            .is_success(),
        "a refused file changes nothing"
    );

    let done = target
        .upload("/api/backups/restore", &[cookie], bytes)
        .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let report = done.json();
    assert!(report["tables"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "users" && t["rows"] == 1));
    assert!(report["safety"]
        .as_str()
        .unwrap()
        .contains("before-restore"));
    // The restoring admin's session is over: the admins are those of the backup now.
    assert_eq!(
        target.get("/api/auth/me", Some(&owner)).await.status,
        StatusCode::UNAUTHORIZED
    );
    let shop = target.sign_in("shop", "a-reseller-password").await;
    assert_eq!(
        target.get("/api/auth/me", Some(&shop)).await.json()["username"],
        "shop"
    );
    let again = target.sign_in("owner", "a-long-password").await;
    let users = target.get("/api/users", Some(&again)).await.json();
    assert_eq!(users["total"], 1);
    assert_eq!(users["users"][0]["username"], "ali");
    // The way back is among the files, and the restore is in the log.
    let files = target.get("/api/backups", Some(&again)).await.json();
    assert!(files["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["before_restore"] == true));
    let log = target
        .get("/api/audit?action=backup.", Some(&again))
        .await
        .json();
    assert_eq!(log["entries"][0]["action"], "backup.restore");
}

#[tokio::test]
async fn a_sealed_backup_needs_its_passphrase_to_be_put_back() {
    let (source, source_owner) = world().await;
    source
        .send("PUT", "/api/backups/settings", Some(&source_owner), Some(json!({"enabled": false, "interval_hours": 24, "keep": 7, "send_telegram": false, "include_usage": false, "passphrase": "correct horse"})))
        .await;
    let made = source
        .send("POST", "/api/backups", Some(&source_owner), None)
        .await
        .json();
    let bytes = crate::backup::read(&source.state, made["file"].as_str().unwrap()).unwrap();

    let (target, owner) = world().await;
    let cookie = ("cookie", owner.as_str());
    error(
        target
            .upload("/api/backups/restore", &[cookie], bytes.clone())
            .await,
        StatusCode::BAD_REQUEST,
        "passphrase_required",
    )
    .await;
    error(
        target
            .upload(
                "/api/backups/restore",
                &[cookie, ("x-backup-passphrase", "wrong horse")],
                bytes.clone(),
            )
            .await,
        StatusCode::BAD_REQUEST,
        "wrong_passphrase",
    )
    .await;
    assert!(target
        .get("/api/auth/me", Some(&owner))
        .await
        .status
        .is_success());
    let done = target
        .upload(
            "/api/backups/restore",
            &[cookie, ("x-backup-passphrase", "correct horse")],
            bytes,
        )
        .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
}
