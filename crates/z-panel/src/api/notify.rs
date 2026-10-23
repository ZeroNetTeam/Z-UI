//! Notifications, the bot, and tying a Telegram chat to oneself.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit::Audit;
use crate::auth::Access;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::notify::{self, BotStatus, Channel, Settings, Status};
use crate::server::AppState;

#[derive(Serialize, utoipa::ToSchema)]
pub struct ChannelStatuses {
    telegram: Status,
    discord: Status,
    webhook: Status,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct NotifyView {
    settings: notify::settings::View,
    /// How the last delivery through each channel went.
    status: ChannelStatuses,
    bot: BotStatus,
}

fn view(state: &AppState) -> NotifyView {
    let [telegram, discord, webhook] = state.notifier().status();
    NotifyView {
        settings: state.notify_settings().view(),
        status: ChannelStatuses {
            telegram,
            discord,
            webhook,
        },
        bot: state.notifier().bot(),
    }
}

#[utoipa::path(
    get,
    path = "/notifications",
    operation_id = "notify_get",
    tag = "notifications",
    summary = "Read the notification, bot and backup settings",
    responses((status = 200, description = "The settings, without their secrets, and how each channel is doing", body = NotifyView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn get(State(state): State<AppState>, _: Access) -> Json<NotifyView> {
    Json(view(&state))
}

#[utoipa::path(
    put,
    path = "/notifications",
    operation_id = "notify_save",
    tag = "notifications",
    summary = "Change the notification, bot and backup settings",
    request_body = Settings,
    responses((status = 200, description = "Saved. A secret left empty is kept as it was", body = NotifyView), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn save(
    State(state): State<AppState>,
    _: Access,
    Json(incoming): Json<Settings>,
) -> ApiResult<(Audit, Json<NotifyView>)> {
    let settings = state
        .notify_settings()
        .accept(incoming)
        .map_err(|message| ApiError::bad_request("invalid_notify", message))?;
    notify::save(state.db(), &settings).await?;
    state.set_notify_settings(settings);
    Ok((Audit::new("notifications.save", ""), Json(view(&state))))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct TestInput {
    channel: Channel,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Tested {
    /// How many chats got the message (1 for Discord and webhooks).
    delivered: usize,
    /// The bot's name, when Telegram said.
    bot: Option<String>,
}

#[utoipa::path(
    post,
    path = "/notifications/test",
    operation_id = "notify_test",
    tag = "notifications",
    summary = "Send a test message through one channel",
    request_body = TestInput,
    responses((status = 200, description = "Delivered", body = Tested), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn test(
    State(state): State<AppState>,
    access: Access,
    Json(input): Json<TestInput>,
) -> ApiResult<(Audit, Json<Tested>)> {
    let tried = notify::test(&state, input.channel, access.admin.id)
        .await
        .map_err(|error| ApiError::bad_request("delivery_failed", error.message))?;
    Ok((
        Audit::new(
            "notifications.test",
            format!("{:?}", input.channel).to_lowercase(),
        ),
        Json(Tested {
            delivered: tried.delivered,
            bot: tried.bot,
        }),
    ))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ChatView {
    chat_id: i64,
    name: String,
    created_at: i64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TelegramInfo {
    /// The bot is on and has a token.
    enabled: bool,
    /// The bot's name, once it has been heard from.
    bot: Option<String>,
    /// The chats tied to the signed-in admin.
    chats: Vec<ChatView>,
}

#[utoipa::path(
    get,
    path = "/auth/telegram",
    operation_id = "auth_telegram",
    tag = "auth",
    summary = "My Telegram chats and whether the bot is on",
    responses((status = 200, description = "The chats tied to me", body = TelegramInfo), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn telegram(
    State(state): State<AppState>,
    access: Access,
) -> ApiResult<Json<TelegramInfo>> {
    let chats = state
        .db()
        .telegram_links_of("admin", access.admin.id)
        .await?
        .into_iter()
        .map(|link| ChatView {
            chat_id: link.chat_id,
            name: link.name,
            created_at: link.created_at,
        })
        .collect();
    Ok(Json(TelegramInfo {
        enabled: state.notify_settings().telegram_ready(),
        bot: state.notifier().bot().username,
        chats,
    }))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct LinkCode {
    /// Send the bot `/start <code>` from the chat to tie.
    code: String,
    /// The same as a link that opens the chat, when the bot's name is known.
    link: Option<String>,
    /// Seconds the code stays good.
    expires_in: u64,
}

#[utoipa::path(
    post,
    path = "/auth/telegram/link",
    operation_id = "auth_telegram_link",
    tag = "auth",
    summary = "Get a code to tie a Telegram chat to me",
    responses((status = 200, description = "A code to send the bot", body = LinkCode), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn telegram_link(
    State(state): State<AppState>,
    access: Access,
) -> ApiResult<(Audit, Json<LinkCode>)> {
    if !state.notify_settings().telegram_ready() {
        return Err(ApiError::bad_request(
            "telegram_off",
            "the Telegram bot is not on; a sudo admin turns it on under Notifications",
        ));
    }
    let code = notify::issue_link_code(&state, access.admin.id);
    let link = state
        .notifier()
        .bot()
        .username
        .map(|bot| format!("https://t.me/{bot}?start={code}"));
    Ok((
        Audit::new("auth.telegram_link", &access.admin.username),
        Json(LinkCode {
            code,
            link,
            expires_in: notify::LINK_LIFETIME.as_secs(),
        }),
    ))
}

#[utoipa::path(
    delete,
    path = "/auth/telegram/{chat_id}",
    operation_id = "auth_telegram_unlink",
    tag = "auth",
    summary = "Untie one of my Telegram chats",
    params(("chat_id" = i64, Path, description = "the chat's id")),
    responses((status = 204, description = "Untied"), (status = "default", description = "Failed: `error` is a stable code, `message` is for people", body = ErrorBody)),
)]
pub async fn telegram_unlink(
    State(state): State<AppState>,
    access: Access,
    Path(chat_id): Path<i64>,
) -> ApiResult<(StatusCode, Audit, ())> {
    let mine = state
        .db()
        .telegram_link(chat_id)
        .await?
        .filter(|link| link.kind == "admin" && link.ref_id == access.admin.id)
        .ok_or_else(|| ApiError::not_found("that chat is not tied to you"))?;
    state.db().delete_telegram_link(mine.chat_id).await?;
    Ok((
        StatusCode::NO_CONTENT,
        Audit::new("auth.telegram_unlink", &access.admin.username),
        (),
    ))
}
