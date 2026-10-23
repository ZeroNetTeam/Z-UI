//! Telling people what happens: the sudo admins, a user's owner, the user,
//! and chats, a Discord channel or a webhook the admin names.
//!
//! Anything in the panel may [`emit`](Notifier::emit) an [`Event`]; it is put
//! in a queue and a worker delivers it, so a slow or filtered Telegram never
//! holds up the thing that happened. Each channel remembers how its last
//! delivery went, for the dashboard to show.

pub mod bot;
pub mod channels;
pub mod events;
pub mod http;
pub mod settings;
pub mod watch;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::{mpsc, Notify};

pub use channels::{Channel, SendError};
pub use events::Event;
pub use settings::Settings;

use crate::server::AppState;

/// How a channel's last delivery went.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct Status {
    pub ok_at: Option<i64>,
    pub failed_at: Option<i64>,
    pub error: Option<String>,
}

/// What is known of the bot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct BotStatus {
    /// Its `@name`, once Telegram has said.
    pub username: Option<String>,
    /// It is listening for messages.
    pub running: bool,
    pub error: Option<String>,
}

pub struct Notifier {
    tx: mpsc::UnboundedSender<Event>,
    rx: Mutex<Option<mpsc::UnboundedReceiver<Event>>>,
    pending: AtomicUsize,
    idle: Notify,
    status: Mutex<[Status; 3]>,
    bot: Mutex<BotStatus>,
    /// How long to wait before each new attempt at a delivery that could not
    /// be made.
    retry: RwLock<Vec<Duration>>,
    logins: Mutex<watch::Logins>,
    link_tries: Mutex<bot::Attempts>,
}

impl Notifier {
    pub fn new() -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            tx,
            rx: Mutex::new(Some(rx)),
            pending: AtomicUsize::new(0),
            idle: Notify::new(),
            status: Mutex::new(Default::default()),
            bot: Mutex::new(BotStatus::default()),
            retry: RwLock::new(vec![Duration::from_secs(2), Duration::from_secs(10)]),
            logins: Mutex::new(watch::Logins::default()),
            link_tries: Mutex::new(bot::Attempts::default()),
        })
    }

    /// Tell it, without waiting.
    pub fn emit(&self, event: Event) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        if self.tx.send(event).is_err() {
            self.finished();
        }
    }

    fn finished(&self) {
        if self.pending.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.idle.notify_waiters();
        }
    }

    /// Wait until everything emitted so far is delivered (or given up on).
    #[cfg(test)]
    pub async fn settle(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.pending.load(Ordering::SeqCst) == 0 {
                return;
            }
            notified.await;
        }
    }

    pub fn status(&self) -> [Status; 3] {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    #[cfg(test)]
    pub fn status_of(&self, channel: Channel) -> Status {
        self.status()[channel.index()].clone()
    }

    fn record(&self, channel: Channel, result: Result<(), &SendError>) {
        let mut status = self.status.lock().unwrap_or_else(|p| p.into_inner());
        let entry = &mut status[channel.index()];
        match result {
            Ok(()) => {
                entry.ok_at = Some(z_db::now());
                entry.error = None;
            }
            Err(error) => {
                entry.failed_at = Some(z_db::now());
                entry.error = Some(error.message.clone());
            }
        }
    }

    /// A sign-in failed: tell the admins when it keeps happening. `admin` is
    /// the account (id and name), when there is one.
    pub fn login_failed(&self, admin: Option<(i64, &str)>, address: &str) {
        let (id, name) = admin.unwrap_or((0, "(unknown user)"));
        let count = self
            .logins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .failed(name, address, std::time::Instant::now());
        if let Some(attempts) = count {
            self.emit(Event::LoginFailed {
                admin: name.to_string(),
                admin_id: id,
                address: address.to_string(),
                attempts,
            });
        }
    }

    /// Whether a chat may try to link itself again (it gets a few tries).
    pub fn link_attempt(&self, chat: i64) -> bool {
        self.link_tries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .allow(chat, std::time::Instant::now())
    }

    /// An address was locked out of signing in.
    pub fn login_locked(&self, address: &str, minutes: u64) {
        let news = self
            .logins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .locked(address, std::time::Instant::now());
        if news {
            self.emit(Event::LoginLocked {
                address: address.to_string(),
                minutes,
            });
        }
    }

    pub fn bot(&self) -> BotStatus {
        self.bot.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set_bot(&self, change: impl FnOnce(&mut BotStatus)) {
        change(&mut self.bot.lock().unwrap_or_else(|p| p.into_inner()));
    }

    /// Shorten the waits between attempts (for tests).
    #[cfg(test)]
    pub fn retry_after(&self, delays: Vec<Duration>) {
        *self.retry.write().unwrap_or_else(|p| p.into_inner()) = delays;
    }

    fn delays(&self) -> Vec<Duration> {
        self.retry.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Start delivering. Once only; a second call does nothing.
    pub fn start(self: &Arc<Self>, state: AppState) {
        let Some(mut rx) = self.rx.lock().unwrap_or_else(|p| p.into_inner()).take() else {
            return;
        };
        let notifier = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                deliver(&state, &event).await;
                notifier.finished();
            }
        });
    }
}

/// How long a code for linking a chat stays good.
pub const LINK_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// A short code for `admin` to send the bot, which ties that chat to them.
pub fn issue_link_code(state: &AppState, admin: i64) -> String {
    const LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let code: String = (0..8)
        .map(|_| LETTERS[rand::random::<usize>() % LETTERS.len()] as char)
        .collect();
    let mut codes = state.link_codes();
    codes.retain(|_, (_, at)| at.elapsed() < LINK_LIFETIME);
    // One live code per admin: asking again replaces the last.
    codes.retain(|_, (owner, _)| *owner != admin);
    codes.insert(code.clone(), (admin, std::time::Instant::now()));
    code
}

/// The admin a code was issued to, if it is still good. Good once.
pub fn take_link_code(state: &AppState, code: &str) -> Option<i64> {
    let mut codes = state.link_codes();
    codes.retain(|_, (_, at)| at.elapsed() < LINK_LIFETIME);
    codes
        .remove(&code.trim().to_ascii_uppercase())
        .map(|(admin, _)| admin)
}

/// The stored settings, or the defaults.
pub async fn load(db: &z_db::Database) -> Settings {
    match db.setting(settings::SETTING).await {
        Ok(Some(text)) => serde_json::from_str::<Settings>(&text)
            .ok()
            .and_then(|s| s.checked().ok())
            .unwrap_or_default(),
        _ => Settings::default(),
    }
}

pub async fn save(db: &z_db::Database, settings: &Settings) -> Result<(), z_db::DbError> {
    db.set_setting(
        settings::SETTING,
        &serde_json::to_string(settings).unwrap_or_default(),
    )
    .await
}

/// The chats of the sudo admins and of `also`, an admin it concerns.
pub async fn admin_chats(state: &AppState, also: Option<i64>) -> Vec<i64> {
    let mut ids: Vec<i64> = match state.db().admins().await {
        Ok(admins) => admins
            .into_iter()
            .filter(|admin| admin.is_sudo && admin.enabled)
            .map(|admin| admin.id)
            .collect(),
        Err(_) => Vec::new(),
    };
    ids.extend(also);
    ids.sort_unstable();
    ids.dedup();
    let mut chats = Vec::new();
    for id in ids {
        if let Ok(links) = state.db().telegram_links_of("admin", id).await {
            chats.extend(links.into_iter().map(|link| link.chat_id));
        }
    }
    chats
}

/// Try a delivery, again after each wait in the notifier's list while the
/// failure is the kind that may pass.
async fn attempt<F, Fut>(state: &AppState, mut send: F) -> Result<(), SendError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), SendError>>,
{
    let mut delays = state.notifier().delays().into_iter();
    loop {
        match send().await {
            Err(error) if error.transient => match delays.next() {
                Some(wait) => tokio::time::sleep(wait).await,
                None => return Err(error),
            },
            other => return other,
        }
    }
}

/// Tell one Telegram chat; a chat that has shut the bot out loses its link.
async fn telegram_to(
    state: &AppState,
    settings: &Settings,
    chat: i64,
    linked: bool,
    message: &events::Message,
) -> Result<(), SendError> {
    let result = attempt(state, || {
        channels::telegram_message(settings, chat, message)
    })
    .await;
    if let Err(error) = &result {
        if error.gone && linked {
            let _ = state.db().delete_telegram_link(chat).await;
        }
    }
    result
}

/// Tell everyone `event` is for, through every channel that is on.
pub async fn deliver(state: &AppState, event: &Event) {
    let settings = state.notify_settings();
    if !event.wanted(&settings.events) {
        return;
    }
    let message = event.message(&settings.language);
    let audience = event.audience();
    let notifier = state.notifier();

    if settings.telegram_ready() {
        let mut outcome: Option<Result<(), SendError>> = None;
        let mut note = |result: Result<(), SendError>| {
            outcome = match (outcome.take(), result) {
                (_, Err(error)) => Some(Err(error)),
                (Some(Err(error)), Ok(())) => Some(Err(error)),
                (_, Ok(())) => Some(Ok(())),
            };
        };
        let mut chats: Vec<(i64, bool)> = settings
            .telegram
            .chat_ids
            .iter()
            .map(|id| (*id, false))
            .collect();
        for chat in admin_chats(state, audience.admin).await {
            if !chats.iter().any(|(id, _)| *id == chat) {
                chats.push((chat, true));
            }
        }
        for (chat, linked) in chats {
            note(telegram_to(state, &settings, chat, linked, &message).await);
        }
        // The user it is about hears it in their own words.
        if settings.telegram.notify_users {
            if let (Some(user), Some(own)) =
                (audience.user, event.message_for_user(&settings.language))
            {
                if let Ok(links) = state.db().telegram_links_of("user", user).await {
                    for link in links {
                        note(telegram_to(state, &settings, link.chat_id, true, &own).await);
                    }
                }
            }
        }
        if let Some(result) = outcome {
            notifier.record(Channel::Telegram, result.as_ref().map(|_| ()));
        }
    }
    if settings.discord.enabled {
        let result = attempt(state, || channels::discord(&settings, event, &message)).await;
        notifier.record(Channel::Discord, result.as_ref().map(|_| ()));
    }
    if settings.webhook.enabled {
        let result = attempt(state, || channels::webhook(&settings, event, &message)).await;
        notifier.record(Channel::Webhook, result.as_ref().map(|_| ()));
    }
}

/// What a test of a channel found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct Tried {
    /// How many chats (for Telegram) got the message; 1 for the others.
    pub delivered: usize,
    /// The bot's name, when Telegram has said.
    pub bot: Option<String>,
}

/// Send a test through one channel and say how it went, with the real error
/// when it did not. `admin` is who asked: their own chat gets it too.
pub async fn test(state: &AppState, channel: Channel, admin: i64) -> Result<Tried, SendError> {
    let settings = state.notify_settings();
    let event = Event::Test;
    let message = event.message(&settings.language);
    let notifier = state.notifier();
    let result = match channel {
        Channel::Telegram => {
            if !settings.telegram_ready() {
                return Err(SendError {
                    transient: false,
                    gone: false,
                    message: "turn the bot on and give it its token first".into(),
                });
            }
            // The token and the way there are checked before any message.
            let me = channels::telegram_call(&settings, "getMe", &serde_json::json!({})).await?;
            let username = me["username"].as_str().map(str::to_string);
            notifier.set_bot(|bot| bot.username = username.clone());
            let mut chats: Vec<i64> = settings.telegram.chat_ids.clone();
            chats.extend(admin_chats(state, Some(admin)).await);
            chats.sort_unstable();
            chats.dedup();
            if chats.is_empty() {
                let error = SendError {
                    transient: false,
                    gone: false,
                    message:
                        "there is no chat to send to: link yours under Settings, or add a chat id"
                            .into(),
                };
                notifier.record(Channel::Telegram, Err(&error));
                return Err(error);
            }
            let mut delivered = 0;
            let mut last = None;
            for chat in chats {
                match channels::telegram_message(&settings, chat, &message).await {
                    Ok(()) => delivered += 1,
                    Err(error) => last = Some(error),
                }
            }
            match (delivered, last) {
                (0, Some(error)) => Err(error),
                _ => Ok(Tried {
                    delivered,
                    bot: username,
                }),
            }
        }
        Channel::Discord => {
            if !settings.discord.enabled {
                return Err(SendError {
                    transient: false,
                    gone: false,
                    message: "turn Discord on and give it the webhook address first".into(),
                });
            }
            channels::discord(&settings, &event, &message)
                .await
                .map(|()| Tried {
                    delivered: 1,
                    bot: None,
                })
        }
        Channel::Webhook => {
            if !settings.webhook.enabled {
                return Err(SendError {
                    transient: false,
                    gone: false,
                    message: "turn the webhook on and give it an address first".into(),
                });
            }
            channels::webhook(&settings, &event, &message)
                .await
                .map(|()| Tried {
                    delivered: 1,
                    bot: None,
                })
        }
    };
    notifier.record(channel, result.as_ref().map(|_| ()));
    result
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::http::testing::{http_proxy, mock, ok};
    use super::*;
    use crate::server::testing::{panel, Panel};

    /// A panel whose Telegram, Discord and webhook are mocks, with `chat_ids`
    /// listed. Retries are instant.
    async fn world(
        telegram: &super::http::testing::Mock,
        change: impl FnOnce(&mut Settings),
    ) -> Panel {
        let panel = panel("/").await;
        let mut settings = Settings::default();
        settings.telegram.enabled = true;
        settings.telegram.token = "123:abc".into();
        settings.telegram.api_base = telegram.url();
        settings.telegram.chat_ids = vec![-1001];
        settings.language = "en".into();
        change(&mut settings);
        panel.state.set_notify_settings(settings.checked().unwrap());
        panel
            .state
            .notifier()
            .retry_after(vec![Duration::ZERO, Duration::ZERO]);
        panel
    }

    #[tokio::test]
    async fn an_event_reaches_the_listed_chat_the_sudo_chats_and_the_owners() {
        let telegram = ok().await;
        let panel = world(&telegram, |_| {}).await;
        let db = panel.state.db();
        let boss = db.create_admin("boss", "h", true).await.unwrap();
        let role = db.create_role("shop", "users.write", true).await.unwrap();
        let shop = db
            .create_admin_with(
                "shop",
                "h",
                &z_db::AdminFields {
                    role_id: Some(role.id),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let other = db
            .create_admin_with(
                "other",
                "h",
                &z_db::AdminFields {
                    role_id: Some(role.id),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        db.save_telegram_link(11, "admin", boss.id, "boss")
            .await
            .unwrap();
        db.save_telegram_link(22, "admin", shop.id, "shop")
            .await
            .unwrap();
        db.save_telegram_link(33, "admin", other.id, "other")
            .await
            .unwrap();
        db.save_telegram_link(44, "user", 9, "ali").await.unwrap();

        panel.state.notifier().emit(Event::UserDataLow {
            user: "ali".into(),
            user_id: 9,
            owner: Some(shop.id),
            used: 900,
            limit: 1000,
            percent: 90,
        });
        panel.state.notifier().settle().await;
        let sent: Vec<(i64, String)> = telegram
            .requests()
            .iter()
            .map(|r| {
                (
                    r.json()["chat_id"].as_i64().unwrap(),
                    r.json()["text"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let mut chats: Vec<i64> = sent.iter().map(|(chat, _)| *chat).collect();
        chats.sort_unstable();
        assert_eq!(
            chats,
            [-1001, 11, 22, 44],
            "the list, the sudo, the owner and the user — not another reseller"
        );
        assert!(telegram
            .requests()
            .iter()
            .all(|r| r.path == "/bot123:abc/sendMessage"));
        let admin_text = &sent.iter().find(|(chat, _)| *chat == 22).unwrap().1;
        assert!(
            admin_text.contains("<b>⚠️ A user is running low on data</b>")
                && admin_text.contains("<code>ali</code>")
        );
        let user_text = &sent.iter().find(|(chat, _)| *chat == 44).unwrap().1;
        assert!(
            user_text.contains("Your data is running low") && !user_text.contains("ali"),
            "{user_text}"
        );
        let status = panel.state.notifier().status_of(Channel::Telegram);
        assert!(status.ok_at.is_some() && status.error.is_none());
    }

    #[tokio::test]
    async fn what_the_settings_do_not_want_is_not_told_and_users_can_be_left_out() {
        let telegram = ok().await;
        let panel = world(&telegram, |s| {
            s.events.core = false;
            s.telegram.notify_users = false;
        })
        .await;
        panel
            .state
            .db()
            .save_telegram_link(44, "user", 9, "ali")
            .await
            .unwrap();
        panel.state.notifier().emit(Event::CoreDown);
        panel.state.notifier().emit(Event::UserLimited {
            user: "ali".into(),
            user_id: 9,
            owner: None,
            limit: 1000,
        });
        panel.state.notifier().settle().await;
        let chats: Vec<i64> = telegram
            .requests()
            .iter()
            .map(|r| r.json()["chat_id"].as_i64().unwrap())
            .collect();
        assert_eq!(chats, [-1001], "no core news, and the user is not told");
    }

    #[tokio::test]
    async fn a_delivery_that_fails_is_tried_again_and_then_given_up() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let telegram = mock(move |_| {
            if counter.fetch_add(1, Ordering::SeqCst) < 2 {
                (502, "bad gateway".into())
            } else {
                (200, r#"{"ok":true,"result":{}}"#.into())
            }
        })
        .await;
        let panel = world(&telegram, |_| {}).await;
        panel.state.notifier().emit(Event::CoreUp);
        panel.state.notifier().settle().await;
        assert_eq!(
            telegram.requests().len(),
            3,
            "two failures, then it went through"
        );
        assert!(panel
            .state
            .notifier()
            .status_of(Channel::Telegram)
            .error
            .is_none());

        let broken = mock(|_| (500, "no".into())).await;
        let panel = world(&broken, |_| {}).await;
        panel.state.notifier().emit(Event::CoreUp);
        panel.state.notifier().settle().await;
        assert_eq!(broken.requests().len(), 3, "the first try and two more");
        let status = panel.state.notifier().status_of(Channel::Telegram);
        assert!(status.error.unwrap().contains("HTTP 500") && status.failed_at.is_some());
    }

    #[tokio::test]
    async fn a_refusal_is_not_pressed_and_a_chat_that_blocked_the_bot_loses_its_link() {
        let telegram = mock(|seen| {
            if seen.json()["chat_id"] == 55 {
                (
                    403,
                    r#"{"ok":false,"description":"Forbidden: bot was blocked by the user"}"#.into(),
                )
            } else {
                (200, r#"{"ok":true,"result":{}}"#.into())
            }
        })
        .await;
        let panel = world(&telegram, |s| s.telegram.chat_ids = vec![]).await;
        let boss = panel
            .state
            .db()
            .create_admin("boss", "h", true)
            .await
            .unwrap();
        panel
            .state
            .db()
            .save_telegram_link(55, "admin", boss.id, "gone")
            .await
            .unwrap();
        panel
            .state
            .db()
            .save_telegram_link(66, "admin", boss.id, "here")
            .await
            .unwrap();
        panel.state.notifier().emit(Event::CoreDown);
        panel.state.notifier().settle().await;
        assert_eq!(
            telegram.requests().len(),
            2,
            "one try each, no pressing a refusal"
        );
        assert!(panel.state.db().telegram_link(55).await.unwrap().is_none());
        assert!(panel.state.db().telegram_link(66).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn discord_and_a_signed_webhook_are_told_too() {
        let discord = ok().await;
        let hook = ok().await;
        let panel = panel("/").await;
        let mut settings = Settings {
            language: "en".into(),
            ..Default::default()
        };
        settings.discord.enabled = true;
        settings.discord.url = "https://discord.com/api/webhooks/1/x".into();
        settings.webhook.enabled = true;
        settings.webhook.url = format!("{}/z-ui", hook.url());
        settings.webhook.secret = "s3cret".into();
        let mut settings = settings.checked().unwrap();
        // Discord is only reachable by its own name; stand it in with a mock.
        settings.discord.url = format!("{}/api/webhooks/1/x", discord.url());
        panel.state.set_notify_settings(settings);
        panel.state.notifier().retry_after(vec![]);

        panel.state.notifier().emit(Event::BackupFailed {
            error: "disk full".into(),
        });
        panel.state.notifier().settle().await;

        let embed = &discord.requests()[0].json()["embeds"][0];
        assert_eq!(embed["title"], "❌ A backup failed");
        assert_eq!(embed["fields"][0]["value"], "disk full");
        let seen = &hook.requests()[0];
        assert_eq!(seen.path, "/z-ui");
        assert_eq!(seen.header("x-z-ui-event"), Some("backup.failed"));
        assert_eq!(
            seen.header("x-z-ui-signature"),
            Some(channels::sign("s3cret", &seen.body).as_str())
        );
        assert_eq!(seen.json()["data"], json!({"error": "disk full"}));
        assert!(panel
            .state
            .notifier()
            .status_of(Channel::Discord)
            .ok_at
            .is_some());
        assert!(panel
            .state
            .notifier()
            .status_of(Channel::Webhook)
            .ok_at
            .is_some());
    }

    #[tokio::test]
    async fn a_proxy_carries_the_message_when_the_address_is_out_of_reach() {
        let telegram = ok().await;
        let proxy = http_proxy(None).await;
        let panel = world(&telegram, |s| {
            s.proxy = format!("http://{}", proxy.address);
        })
        .await;
        panel.state.notifier().emit(Event::CoreUp);
        panel.state.notifier().settle().await;
        assert_eq!(telegram.requests().len(), 1);
        assert_eq!(
            proxy.asked.lock().unwrap().as_slice(),
            [telegram.address.to_string()]
        );
    }

    #[tokio::test]
    async fn a_test_of_a_channel_that_is_off_says_to_turn_it_on() {
        let panel = panel("/").await;
        let admin = panel
            .state
            .db()
            .create_admin("boss", "h", true)
            .await
            .unwrap();
        let off = test(&panel.state, Channel::Telegram, admin.id)
            .await
            .unwrap_err();
        assert!(off.message.contains("token"));
        assert!(test(&panel.state, Channel::Discord, admin.id)
            .await
            .unwrap_err()
            .message
            .contains("Discord"));
        assert!(test(&panel.state, Channel::Webhook, admin.id)
            .await
            .unwrap_err()
            .message
            .contains("webhook"));
    }

    #[tokio::test]
    async fn a_test_of_telegram_checks_the_token_then_sends_to_the_chats() {
        let telegram = mock(|seen| {
            if seen.path.ends_with("/getMe") {
                (200, r#"{"ok":true,"result":{"username":"zui_bot"}}"#.into())
            } else {
                (200, r#"{"ok":true,"result":{}}"#.into())
            }
        })
        .await;
        let panel = world(&telegram, |s| s.telegram.chat_ids = vec![]).await;
        let admin = panel
            .state
            .db()
            .create_admin("boss", "h", true)
            .await
            .unwrap();
        let none = test(&panel.state, Channel::Telegram, admin.id)
            .await
            .unwrap_err();
        assert!(none.message.contains("no chat"), "{}", none.message);
        panel
            .state
            .db()
            .save_telegram_link(77, "admin", admin.id, "boss")
            .await
            .unwrap();
        let tried = test(&panel.state, Channel::Telegram, admin.id)
            .await
            .unwrap();
        assert_eq!(
            (tried.delivered, tried.bot.as_deref()),
            (1, Some("zui_bot"))
        );
        assert_eq!(
            panel.state.notifier().bot().username.as_deref(),
            Some("zui_bot")
        );
        let last = telegram.requests().pop().unwrap();
        assert!(last.json()["text"]
            .as_str()
            .unwrap()
            .contains("Test message from Z-UI"));
    }

    #[tokio::test]
    async fn a_test_with_a_wrong_token_says_so() {
        let wrong = mock(|_| (401, r#"{"ok":false}"#.into())).await;
        let panel = world(&wrong, |_| {}).await;
        let admin = panel
            .state
            .db()
            .create_admin("boss", "h", true)
            .await
            .unwrap();
        let bad = test(&panel.state, Channel::Telegram, admin.id)
            .await
            .unwrap_err();
        assert!(bad.message.contains("token is wrong"), "{}", bad.message);
        assert_eq!(
            wrong.requests().len(),
            1,
            "nothing is sent after the token fails"
        );
    }

    #[tokio::test]
    async fn wrong_passwords_a_lockout_and_a_new_address_are_told() {
        let telegram = ok().await;
        let panel = world(&telegram, |_| {}).await;
        panel.set_up("").await;
        let wrong = || {
            panel.send(
                "POST",
                "/api/auth/login",
                None,
                Some(json!({"username": "owner", "password": "wrong-password"})),
            )
        };
        wrong().await;
        wrong().await;
        panel.state.notifier().settle().await;
        assert!(telegram.requests().is_empty(), "two slips are not news");
        wrong().await;
        panel.state.notifier().settle().await;
        let texts: Vec<String> = telegram
            .requests()
            .iter()
            .map(|r| r.json()["text"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(
            texts[0].contains("Failed sign-ins")
                && texts[0].contains("<code>owner</code>")
                && texts[0].contains("<code>3</code>")
        );

        wrong().await;
        wrong().await; // the fifth locks the address
        let locked = wrong().await;
        assert_eq!(locked.status, axum::http::StatusCode::TOO_MANY_REQUESTS);
        panel.state.notifier().settle().await;
        let all: Vec<String> = telegram
            .requests()
            .iter()
            .map(|r| r.json()["text"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            all.iter().filter(|t| t.contains("locked out")).count(),
            1,
            "told once: {all:?}"
        );

        // A new address: the owner's very first sign-in (setup) is not news, a
        // later one from an address the log has not seen is.
        let panel = world(&ok().await, |_| {}).await;
        let telegram = ok().await;
        let mut settings = panel.state.notify_settings();
        settings.telegram.api_base = telegram.url();
        panel.state.set_notify_settings(settings);
        panel.set_up("").await;
        panel.sign_in("owner", "a-long-password").await;
        panel.state.notifier().settle().await;
        assert!(
            telegram.requests().is_empty(),
            "the same address as before is not news"
        );
        sqlx::query("DELETE FROM audit_log")
            .execute(panel.state.db().pool())
            .await
            .unwrap();
        panel.sign_in("owner", "a-long-password").await;
        panel.state.notifier().settle().await;
        let texts: Vec<String> = telegram
            .requests()
            .iter()
            .map(|r| r.json()["text"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].contains("new address") && texts[0].contains("<code>owner</code>"));
    }
}
