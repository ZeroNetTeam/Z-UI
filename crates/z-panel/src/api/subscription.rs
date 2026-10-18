//! Subscription settings, and each user's subscription and devices as the
//! dashboard shows them.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Serialize;

use crate::auth::CurrentAdmin;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::subscription::{self, Settings};

pub async fn settings(State(state): State<AppState>, _: CurrentAdmin) -> Json<Settings> {
    Json(state.subscription())
}

pub async fn save_settings(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Json(input): Json<Settings>,
) -> ApiResult<Json<Settings>> {
    let settings = input
        .checked(&state.config().base_path)
        .map_err(|message| ApiError::bad_request("invalid_subscription", message))?;
    let text = serde_json::to_string(&settings).map_err(ApiError::internal)?;
    state.db().set_setting(subscription::SETTING, &text).await?;
    state.set_subscription(settings.clone());
    Ok(Json(settings))
}

#[derive(Serialize)]
pub struct Link {
    remark: String,
    link: String,
}

#[derive(Serialize)]
pub struct UserSubscription {
    url: String,
    links: Vec<Link>,
}

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "there is no such user")
}

/// The address the dashboard was reached at, which subscription links share
/// unless a public address is set, and the name in it.
fn origin(state: &AppState, headers: &HeaderMap) -> (String, String) {
    let host = headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost");
    let scheme = if state.tls() { "https" } else { "http" };
    (
        format!("{scheme}://{host}"),
        subscription::request_host(headers),
    )
}

/// A user's subscription address and the links in it, as its app would get
/// them now.
pub async fn user_subscription(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> ApiResult<Json<UserSubscription>> {
    let user = state.db().user(id).await?.ok_or_else(not_found)?;
    let settings = state.subscription();
    let (origin, host) = origin(&state, &headers);
    let server = if settings.server_address.is_empty() {
        host
    } else {
        settings.server_address.clone()
    };
    let endpoints = subscription::user_endpoints(&state, &user, &server).await?;
    Ok(Json(UserSubscription {
        url: subscription::url(&settings, &origin, &user.sub_token),
        links: endpoints
            .iter()
            .map(|e| Link {
                remark: e.remark.clone(),
                link: subscription::formats::link(e),
            })
            .collect(),
    }))
}

#[derive(Serialize)]
pub struct DeviceView {
    id: i64,
    hwid: String,
    platform: String,
    os_version: String,
    model: String,
    user_agent: String,
    first_seen: i64,
    last_seen: i64,
}

pub async fn devices(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<DeviceView>>> {
    state.db().user(id).await?.ok_or_else(not_found)?;
    Ok(Json(
        state
            .db()
            .user_devices(id)
            .await?
            .into_iter()
            .map(|d| DeviceView {
                id: d.id,
                hwid: d.hwid,
                platform: d.platform,
                os_version: d.os_version,
                model: d.model,
                user_agent: d.user_agent,
                first_seen: d.first_seen,
                last_seen: d.last_seen,
            })
            .collect(),
    ))
}

/// Forget one device, which frees its place under the user's limit.
pub async fn delete_device(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path((id, device)): Path<(i64, i64)>,
) -> ApiResult<StatusCode> {
    if state.db().delete_devices(id, Some(device)).await? == 0 {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "there is no such device",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_devices(
    State(state): State<AppState>,
    _: CurrentAdmin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    state.db().delete_devices(id, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::StatusCode;
    use base64::Engine as _;
    use serde_json::{json, Value};

    use crate::server::testing::{connect, echo_sink, free_port, panel, Panel};

    const SS_SERVER_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZg==";

    async fn run(config: zero_config::RuntimeConfig) {
        let server = Arc::new(zero_runtime::Server::new(zero_runtime::ServerConfig {
            config: Arc::new(config),
            generation: zero_core::GenerationId(1),
        }));
        let running = Arc::clone(&server);
        tokio::spawn(async move { running.run().await });
        server.wait_until_listening().await;
    }

    /// A zray client whose SOCKS5 port goes out through `link`.
    async fn client_from(link: &zero_config::share_link::ShareLink) -> u16 {
        let socks = free_port();
        let base = json!({
            "inbounds": [{"tag": "socks", "listen": "127.0.0.1", "port": socks, "protocol": "socks"}],
            "outbounds": [{"tag": "proxy", "protocol": "freedom"}],
        });
        let (generation, _) =
            zero_config::compile_config(&base, zero_core::GenerationId(1)).unwrap();
        let mut config = (*generation.config).clone();
        let mut outbound = link.outbound.clone();
        outbound.tag = Arc::from("proxy");
        config.outbounds = vec![outbound].into_boxed_slice();
        run(config).await;
        socks
    }

    /// A panel under `/panel` with a plain VLESS and a Shadowsocks 2022
    /// inbound, and a user in the default group.
    async fn rig() -> (Panel, String, Value) {
        let panel = panel("/panel").await;
        let session = panel.set_up("/panel").await;
        panel
            .state
            .db()
            .set_setting(crate::core_config::ALLOW_PRIVATE, "true")
            .await
            .unwrap();
        for (remark, protocol, settings) in [
            ("plain", "vless", json!({"decryption": "none"})),
            (
                "ss",
                "shadowsocks",
                json!({"method": "2022-blake3-aes-128-gcm", "password": SS_SERVER_KEY}),
            ),
        ] {
            let made = panel
                .send("POST", "/panel/api/inbounds", Some(&session), Some(json!({
                    "remark": remark, "listen": "127.0.0.1", "port": free_port(), "protocol": protocol,
                    "settings": settings,
                    "stream": if protocol == "vless" { json!({"network": "raw", "security": "none"}) } else { json!({}) },
                })))
                .await;
            assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
        }
        let default = panel.get("/panel/api/groups", Some(&session)).await.json()[0]["id"].clone();
        let user = panel
            .send(
                "POST",
                "/panel/api/users",
                Some(&session),
                Some(json!({
                    "username": "alice", "group_ids": [default], "data_limit": 1_000_000_000,
                    "expire_at": z_db::now() + 86_400 * 10,
                })),
            )
            .await;
        assert_eq!(user.status, StatusCode::CREATED, "{}", user.body);
        (panel, session, user.json())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_user_connects_with_what_its_subscription_gives() {
        let (panel, session, alice) = rig().await;
        let token = alice["sub_token"].as_str().unwrap();
        let path = format!("/sub/{token}");
        let reply = panel
            .fetch(
                &path,
                &[("host", "127.0.0.1:2053"), ("user-agent", "v2rayNG/1.9.30")],
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        let info = reply.headers["subscription-userinfo"].to_str().unwrap();
        assert!(
            info.starts_with("upload=0; download=0; total=1000000000; expire="),
            "{info}"
        );
        assert_eq!(reply.headers["profile-update-interval"], "12");
        let links = zero_config::share_link::parse_subscription(&reply.body);
        assert_eq!(links.len(), 2, "{}", reply.body);
        let fetched = panel
            .get(&format!("/panel/api/users/{}", alice["id"]), Some(&session))
            .await
            .json();
        assert_eq!(fetched["sub_user_agent"], "v2rayNG/1.9.30");
        assert!(fetched["sub_updated_at"].is_i64());

        // Every link connects through the panel's own core.
        let sink = echo_sink().await;
        for link in &links {
            let link = link.as_ref().unwrap();
            let socks = client_from(link).await;
            assert!(
                connect(socks, sink, b"hello").await.is_some(),
                "{} does not connect: {}",
                link.remark,
                link.link
            );
        }

        // The zray JSON connects too, once its local ports are free ones and
        // the local sink is not sent direct.
        let reply = panel
            .fetch(&format!("{path}?format=zray"), &[("host", "127.0.0.1")])
            .await;
        let mut config = reply.json()[0].clone();
        let socks = free_port();
        config["inbounds"] =
            json!([{"tag": "socks", "listen": "127.0.0.1", "port": socks, "protocol": "socks"}]);
        config["routing"] = json!({"rules": []});
        let (generation, _) = zero_config::compile_config(&config, zero_core::GenerationId(1))
            .unwrap_or_else(|error| panic!("{error}\n{config:#}"));
        run((*generation.config).clone()).await;
        assert!(connect(socks, sink, b"json").await.is_some());

        // What the dashboard shows is the same.
        let shown = panel
            .send(
                "GET",
                &format!("/panel/api/users/{}/subscription", alice["id"]),
                Some(&session),
                None,
            )
            .await
            .json();
        assert_eq!(shown["links"].as_array().unwrap().len(), 2);
        assert!(shown["url"].as_str().unwrap().ends_with(&path), "{shown}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn each_client_gets_its_format_and_a_browser_the_page() {
        let (panel, session, alice) = rig().await;
        let path = format!("/sub/{}", alice["sub_token"].as_str().unwrap());
        let clash = panel
            .fetch(
                &path,
                &[("user-agent", "ClashMetaForAndroid/2.11"), ("host", "h")],
            )
            .await;
        assert!(clash.headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/yaml"));
        assert!(
            clash.body.contains("\"proxies\":") && clash.body.contains("\"type\": \"ss\""),
            "{}",
            clash.body
        );
        let singbox = panel
            .fetch(&format!("{path}/sing-box"), &[("host", "h")])
            .await
            .json();
        assert_eq!(singbox["route"]["final"], "proxy");
        let page = panel
            .fetch(
                &path,
                &[
                    ("user-agent", "Mozilla/5.0 (X11; Linux) Firefox/130"),
                    ("accept", "text/html"),
                    ("accept-language", "fa-IR"),
                    ("host", "h"),
                ],
            )
            .await;
        assert!(page.body.contains("alice") && page.body.contains("dir=\"rtl\""));
        assert!(page.body.contains("<svg"), "QR codes are drawn");
        assert_eq!(
            panel.fetch("/sub/not-a-token", &[]).await.status,
            StatusCode::NOT_FOUND
        );

        // A disabled user still sees its usage, and gets no configs.
        let id = alice["id"].clone();
        panel
            .send(
                "POST",
                "/panel/api/users/bulk",
                Some(&session),
                Some(json!({"action": "disable", "ids": [id]})),
            )
            .await;
        let off = panel.fetch(&path, &[("host", "h")]).await;
        assert_eq!(off.status, StatusCode::OK);
        assert!(off.headers.contains_key("subscription-userinfo"));
        assert!(zero_config::share_link::parse_subscription(&off.body).is_empty());

        // Moved to another path, the old one is the panel's 404.
        let moved = panel
            .send("PUT", "/panel/api/settings/subscription", Some(&session), Some(json!({
                "path": "/s/feed", "title": "My VPN", "remark_template": "{remark} · {username}",
            })))
            .await;
        assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
        assert_eq!(panel.fetch(&path, &[]).await.status, StatusCode::NOT_FOUND);
        let new_path = format!("/s/feed/{}", alice["sub_token"].as_str().unwrap());
        let reply = panel.fetch(&new_path, &[("host", "h")]).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(
            reply.headers["profile-title"].to_str().unwrap(),
            format!(
                "base64:{}",
                base64::engine::general_purpose::STANDARD.encode("My VPN")
            )
        );
        let inside = panel
            .send(
                "PUT",
                "/panel/api/settings/subscription",
                Some(&session),
                Some(json!({"path": "/panel/sub"})),
            )
            .await;
        assert_eq!(inside.json()["error"], "invalid_subscription");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_device_limit_holds_by_the_hwid_the_app_sends() {
        let (panel, session, alice) = rig().await;
        let id = alice["id"].as_i64().unwrap();
        let mut edit = json!({
            "username": "alice", "group_ids": alice["group_ids"], "hwid_limit": 1,
            "data_limit": alice["data_limit"], "expire_at": alice["expire_at"],
        });
        let limited = panel
            .send(
                "PUT",
                &format!("/panel/api/users/{id}"),
                Some(&session),
                Some(edit.clone()),
            )
            .await;
        assert_eq!(limited.json()["hwid_limit"], 1, "{}", limited.body);
        let path = format!("/sub/{}", alice["sub_token"].as_str().unwrap());
        let on = |hwid: &'static str| {
            let panel = &panel;
            let path = path.clone();
            async move {
                panel
                    .fetch(
                        &path,
                        &[
                            ("host", "h"),
                            ("user-agent", "Happ/1.0"),
                            ("x-hwid", hwid),
                            ("x-device-os", "Android"),
                        ],
                    )
                    .await
                    .status
            }
        };
        assert_eq!(on("phone").await, StatusCode::OK);
        assert_eq!(on("laptop").await, StatusCode::FORBIDDEN, "a second device");
        assert_eq!(on("phone").await, StatusCode::OK, "the first device again");
        let devices = panel
            .get(&format!("/panel/api/users/{id}/devices"), Some(&session))
            .await
            .json();
        assert_eq!(devices.as_array().unwrap().len(), 1);
        assert_eq!(devices[0]["platform"], "Android");

        // An app that does not say which device: let in, unless required.
        assert_eq!(
            panel.fetch(&path, &[("host", "h")]).await.status,
            StatusCode::OK
        );
        panel
            .send(
                "PUT",
                "/panel/api/settings/subscription",
                Some(&session),
                Some(json!({"hwid_required": true})),
            )
            .await;
        assert_eq!(
            panel.fetch(&path, &[("host", "h")]).await.status,
            StatusCode::FORBIDDEN
        );

        // Forgetting a device frees its place.
        let forget = panel
            .send(
                "DELETE",
                &format!("/panel/api/users/{id}/devices/{}", devices[0]["id"]),
                Some(&session),
                None,
            )
            .await;
        assert_eq!(forget.status, StatusCode::NO_CONTENT);
        assert_eq!(on("laptop").await, StatusCode::OK);

        // No limit: any device.
        edit["hwid_limit"] = json!(0);
        panel
            .send(
                "PUT",
                &format!("/panel/api/users/{id}"),
                Some(&session),
                Some(edit),
            )
            .await;
        assert_eq!(on("tablet").await, StatusCode::OK);
    }
}
