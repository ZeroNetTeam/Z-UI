//! The ways of telling: Telegram, Discord and a signed webhook.

use std::time::Duration;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;

use super::events::{Event, Message};
use super::http::{self, HttpError};
use super::settings::Settings;

/// How long a delivery waits for the other side.
pub const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Telegram,
    Discord,
    Webhook,
}

impl Channel {
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Why a delivery failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendError {
    /// Worth trying again: the other side was unreachable or busy.
    pub transient: bool,
    /// The chat will not take messages from this bot (blocked, removed).
    pub gone: bool,
    pub message: String,
}

impl SendError {
    fn config(message: impl Into<String>) -> Self {
        Self {
            transient: false,
            gone: false,
            message: message.into(),
        }
    }

    fn from_http(error: HttpError, what: &str) -> Self {
        match error {
            HttpError::Status(code) => Self {
                transient: code == 429 || code >= 500,
                gone: code == 403,
                message: match (what, code) {
                    ("Telegram", 401 | 404) => "Telegram says the bot token is wrong".to_string(),
                    ("Telegram", 403) => "the bot was blocked, or is not in that chat".to_string(),
                    ("Telegram", 400) => "Telegram refused it: is the chat id right, and has the bot been started in that chat?".to_string(),
                    ("Telegram", 429) => "Telegram says to slow down".to_string(),
                    (_, 404) => format!("{what} says there is no such address (HTTP 404)"),
                    _ => format!("{what} answered HTTP {code}"),
                },
            },
            HttpError::Failed(message) => Self {
                transient: true,
                gone: false,
                message: format!("{what} could not be reached: {message}"),
            },
        }
    }
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A message as Telegram's HTML: a bold title and a line for each field.
pub fn telegram_html(message: &Message) -> String {
    let mut text = format!("<b>{}</b>", escape(&message.title));
    for (label, value) in &message.fields {
        text.push_str(&format!(
            "\n{}: <code>{}</code>",
            escape(label),
            escape(value)
        ));
    }
    text
}

/// A message as plain text.
pub fn plain(message: &Message) -> String {
    let mut text = message.title.clone();
    for (label, value) in &message.fields {
        text.push_str(&format!("\n{label}: {value}"));
    }
    text
}

/// One call of the Telegram Bot API; the `result` of the answer.
pub async fn telegram_call(
    settings: &Settings,
    method: &str,
    body: &Value,
) -> Result<Value, SendError> {
    if settings.telegram.token.is_empty() {
        return Err(SendError::config("the bot has no token yet"));
    }
    let url = format!(
        "{}/bot{}/{method}",
        settings.telegram_api(),
        settings.telegram.token
    );
    let proxy = settings.proxy();
    let reply = http::post(
        &url,
        "application/json",
        &[],
        body.to_string().as_bytes(),
        proxy.as_ref(),
        TIMEOUT,
    )
    .await
    .map_err(|error| SendError::from_http(error, "Telegram"))?;
    parsed(&reply)
}

/// `result` of a Bot API answer, or why it is `ok: false`.
fn parsed(reply: &[u8]) -> Result<Value, SendError> {
    let answer: Value = serde_json::from_slice(reply).map_err(|_| {
        SendError::config("Telegram's answer made no sense (is the API address right?)")
    })?;
    if answer["ok"] == true {
        Ok(answer["result"].clone())
    } else {
        Err(SendError::config(format!(
            "Telegram refused it: {}",
            answer["description"].as_str().unwrap_or("no reason given")
        )))
    }
}

/// Like [`telegram_call`] for a long poll, which waits longer than the rest.
pub async fn telegram_poll(
    settings: &Settings,
    offset: i64,
    seconds: u64,
) -> Result<Value, SendError> {
    let url = format!(
        "{}/bot{}/getUpdates",
        settings.telegram_api(),
        settings.telegram.token
    );
    let proxy = settings.proxy();
    let body = json!({"offset": offset, "timeout": seconds, "allowed_updates": ["message", "callback_query"]});
    let reply = http::post(
        &url,
        "application/json",
        &[],
        body.to_string().as_bytes(),
        proxy.as_ref(),
        Duration::from_secs(seconds + 15),
    )
    .await
    .map_err(|error| SendError::from_http(error, "Telegram"))?;
    parsed(&reply)
}

pub async fn telegram_message(
    settings: &Settings,
    chat_id: i64,
    message: &Message,
) -> Result<(), SendError> {
    telegram_text(settings, chat_id, &telegram_html(message), None).await
}

/// Say `html` in a chat, with buttons if there are any.
pub async fn telegram_text(
    settings: &Settings,
    chat_id: i64,
    html: &str,
    keyboard: Option<Value>,
) -> Result<(), SendError> {
    let mut body = json!({
        "chat_id": chat_id,
        "text": html,
        "parse_mode": "HTML",
        "disable_web_page_preview": true,
    });
    if let Some(keyboard) = keyboard {
        body["reply_markup"] = json!({"inline_keyboard": keyboard});
    }
    telegram_call(settings, "sendMessage", &body)
        .await
        .map(|_| ())
}

/// Send a file to a chat, with a caption.
pub async fn telegram_file(
    settings: &Settings,
    chat_id: i64,
    caption: &str,
    filename: &str,
    bytes: &[u8],
) -> Result<(), SendError> {
    let url = format!(
        "{}/bot{}/sendDocument",
        settings.telegram_api(),
        settings.telegram.token
    );
    let chat = chat_id.to_string();
    let (content_type, body) = http::multipart(
        &[("chat_id", &chat), ("caption", caption)],
        ("document", filename, bytes),
    );
    let proxy = settings.proxy();
    let reply = http::post(
        &url,
        &content_type,
        &[],
        &body,
        proxy.as_ref(),
        Duration::from_secs(120),
    )
    .await
    .map_err(|error| SendError::from_http(error, "Telegram"))?;
    parsed(&reply).map(|_| ())
}

fn colour(event: &Event) -> u32 {
    match event {
        Event::CoreDown
        | Event::BackupFailed { .. }
        | Event::UserLimited { .. }
        | Event::LoginLocked { .. } => 0xd9_4f_4f,
        Event::CoreUp | Event::BackupDone { .. } | Event::Test => 0x4f_a8_6b,
        _ => 0xd8_a8_1f,
    }
}

pub async fn discord(
    settings: &Settings,
    event: &Event,
    message: &Message,
) -> Result<(), SendError> {
    let fields: Vec<Value> = message
        .fields
        .iter()
        .map(|(name, value)| json!({"name": name, "value": value, "inline": true}))
        .collect();
    let body = json!({
        "username": "Z-UI",
        "embeds": [{"title": message.title, "color": colour(event), "fields": fields}],
    });
    let proxy = settings.proxy();
    http::post(
        &settings.discord.url,
        "application/json",
        &[],
        body.to_string().as_bytes(),
        proxy.as_ref(),
        TIMEOUT,
    )
    .await
    .map(|_| ())
    .map_err(|error| SendError::from_http(error, "Discord"))
}

/// The body a webhook is sent, and its signature when there is a secret.
pub fn webhook_body(event: &Event, message: &Message, at: i64) -> Vec<u8> {
    json!({
        "event": event.kind(),
        "at": at,
        "title": message.title,
        "text": plain(message),
        "data": event.data(),
    })
    .to_string()
    .into_bytes()
}

pub fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes a key of any length");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

pub async fn webhook(
    settings: &Settings,
    event: &Event,
    message: &Message,
) -> Result<(), SendError> {
    let body = webhook_body(event, message, z_db::now());
    let signature = sign(&settings.webhook.secret, &body);
    let kind = event.kind();
    let mut headers = vec![("X-Z-UI-Event", kind)];
    if !settings.webhook.secret.is_empty() {
        headers.push(("X-Z-UI-Signature", signature.as_str()));
    }
    let proxy = settings.proxy();
    http::post(
        &settings.webhook.url,
        "application/json",
        &headers,
        &body,
        proxy.as_ref(),
        TIMEOUT,
    )
    .await
    .map(|_| ())
    .map_err(|error| SendError::from_http(error, "the webhook"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> Message {
        Message {
            title: "Low <data> & more".into(),
            fields: vec![
                ("User".into(), "a&b".into()),
                ("Used".into(), "1 GB".into()),
            ],
        }
    }

    #[test]
    fn telegram_gets_html_that_cannot_break_out() {
        let html = telegram_html(&message());
        assert_eq!(html, "<b>Low &lt;data&gt; &amp; more</b>\nUser: <code>a&amp;b</code>\nUsed: <code>1 GB</code>");
        assert_eq!(
            plain(&message()),
            "Low <data> & more\nUser: a&b\nUsed: 1 GB"
        );
    }

    #[test]
    fn a_signature_is_the_hmac_of_the_body_and_a_known_one_matches() {
        // RFC 4231 test case 2.
        assert_eq!(
            sign("Jefe", b"what do ya want for nothing?"),
            "sha256=5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        let body = webhook_body(
            &Event::CoreDown,
            &Event::CoreDown.message("en"),
            1_700_000_000,
        );
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            (parsed["event"].as_str(), parsed["at"].as_i64()),
            (Some("core.down"), Some(1_700_000_000))
        );
        assert!(parsed["text"].as_str().unwrap().contains("stopped"));
        assert!(parsed["data"].is_object());
    }

    #[test]
    fn errors_say_what_to_do_and_whether_to_try_again() {
        let refused = |code| SendError::from_http(HttpError::Status(code), "Telegram");
        assert!(refused(401).message.contains("token") && !refused(401).transient);
        assert!(refused(403).gone && !refused(403).transient);
        assert!(refused(400).message.contains("chat id"));
        assert!(refused(429).transient && refused(502).transient);
        let down = SendError::from_http(HttpError::Failed("refused".into()), "Telegram");
        assert!(down.transient && down.message.contains("could not be reached"));
        assert!(!SendError::from_http(HttpError::Status(404), "the webhook").transient);
    }

    #[test]
    fn a_telegram_answer_is_read_for_its_result_or_its_reason() {
        assert_eq!(
            parsed(br#"{"ok":true,"result":{"id":7}}"#).unwrap()["id"],
            7
        );
        let refused =
            parsed(br#"{"ok":false,"description":"Bad Request: chat not found"}"#).unwrap_err();
        assert!(refused.message.contains("chat not found"));
        assert!(parsed(b"<html>")
            .unwrap_err()
            .message
            .contains("made no sense"));
    }
}
