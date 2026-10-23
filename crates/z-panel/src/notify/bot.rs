//! The Telegram bot.
//!
//! It listens by long polling, so it works behind NAT and needs no address of
//! its own. A chat is tied to someone before it can do anything:
//!
//!  * an **admin** gets a code in the dashboard (Settings → Telegram) and sends
//!    `/start <code>`. From then on that chat acts as the admin, with exactly
//!    their permissions and, for a reseller, only their own users: the bot
//!    calls the same functions the API does;
//!  * a **user** (when the admin allows it) sends the bot their subscription
//!    link, which only they have, and can then read their own usage.
//!
//! Anything else only gets `/id` (a chat's number, which the list of chats to
//! tell needs) and instructions. Only private chats are linked.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};

use super::channels::{self, SendError};
use super::events::{bytes, date};
use super::{take_link_code, Settings};
use crate::api::users::{self, BulkAction, BulkRequest, UserInput};
use crate::audit::{self, Actor, Audit};
use crate::auth::access::{assemble, Access, Via};
use crate::auth::permissions::{Level, Section};
use crate::error::ApiError;
use crate::server::AppState;

/// How long one request to Telegram waits for news.
const POLL_SECONDS: u64 = 25;
/// How many guesses at a link a chat gets, and over how long.
const LINK_TRIES: u32 = 5;
const LINK_WINDOW: Duration = Duration::from_secs(10 * 60);

/// Link attempts by chat.
#[derive(Default)]
pub struct Attempts {
    by_chat: HashMap<i64, (u32, Instant)>,
}

impl Attempts {
    /// Whether this chat may try again; counts the try.
    pub fn allow(&mut self, chat: i64, now: Instant) -> bool {
        if self.by_chat.len() > 5000 {
            self.by_chat
                .retain(|_, (_, first)| now.duration_since(*first) < LINK_WINDOW);
        }
        let entry = self.by_chat.entry(chat).or_insert((0, now));
        if now.duration_since(entry.1) > LINK_WINDOW {
            *entry = (0, now);
        }
        entry.0 += 1;
        entry.0 <= LINK_TRIES
    }
}

/// Start listening. It runs for as long as the panel does, waiting while the
/// bot is off.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut offset = 0i64;
        let mut backoff = Duration::from_secs(1);
        loop {
            let settings = state.notify_settings();
            if !settings.telegram_ready() {
                state.notifier().set_bot(|bot| bot.running = false);
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
            if state.notifier().bot().username.is_none() {
                if let Ok(me) = channels::telegram_call(&settings, "getMe", &json!({})).await {
                    let name = me["username"].as_str().map(str::to_string);
                    state.notifier().set_bot(|bot| bot.username = name);
                }
            }
            let polled = Instant::now();
            match channels::telegram_poll(&settings, offset, POLL_SECONDS).await {
                Ok(updates) => {
                    state.notifier().set_bot(|bot| {
                        bot.running = true;
                        bot.error = None;
                    });
                    backoff = Duration::from_secs(1);
                    for update in updates.as_array().into_iter().flatten() {
                        offset = offset.max(update["update_id"].as_i64().unwrap_or(0) + 1);
                        handle(&state, &settings, update).await;
                    }
                    // A poll that comes back at once with nothing (a relay that
                    // does not hold the request) is not asked again at once.
                    if updates.as_array().is_none_or(Vec::is_empty)
                        && polled.elapsed() < Duration::from_secs(1)
                    {
                        tokio::time::sleep(Duration::from_secs(1) - polled.elapsed()).await;
                    }
                }
                Err(error) => {
                    let message = error.message.clone();
                    state.notifier().set_bot(|bot| {
                        bot.running = false;
                        bot.error = Some(message);
                    });
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
    });
}

/// Who is speaking.
struct Chat {
    id: i64,
    private: bool,
    name: String,
}

fn chat_of(message: &Value) -> Option<Chat> {
    let chat = &message["chat"];
    let id = chat["id"].as_i64()?;
    let name = chat["title"]
        .as_str()
        .map(str::to_string)
        .or_else(|| chat["username"].as_str().map(|u| format!("@{u}")))
        .or_else(|| chat["first_name"].as_str().map(str::to_string))
        .unwrap_or_default();
    Some(Chat {
        id,
        private: chat["type"] == "private",
        name: name.chars().take(100).collect(),
    })
}

/// Deal with one update from Telegram.
pub async fn handle(state: &AppState, settings: &Settings, update: &Value) {
    if let Some(message) = update.get("message") {
        if let Some(chat) = chat_of(message) {
            let text = message["text"].as_str().unwrap_or("").trim();
            if !text.is_empty() {
                on_text(state, settings, &chat, text).await;
            }
        }
    } else if let Some(query) = update.get("callback_query") {
        on_button(state, settings, query).await;
    }
}

/// Words in the chat's language.
struct Words(bool);

impl Words {
    fn of(settings: &Settings) -> Self {
        Self(settings.language == "fa")
    }

    fn t<'a>(&self, fa: &'a str, en: &'a str) -> &'a str {
        if self.0 {
            fa
        } else {
            en
        }
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn say(
    state: &AppState,
    settings: &Settings,
    chat: i64,
    html: &str,
    keyboard: Option<Value>,
) {
    if let Err(error) = channels::telegram_text(settings, chat, html, keyboard).await {
        tracing::warn!(chat, error = %error, "the bot could not answer");
        if error.gone {
            let _ = state.db().delete_telegram_link(chat).await;
        }
    }
}

/// What a failed action says, in the chat's language.
fn failure(words: &Words, error: &ApiError) -> String {
    let known = match error.code {
        "forbidden" => Some(("اجازه‌ی این کار را ندارید.", "You may not do this.")),
        "not_found" => Some(("پیدا نشد.", "Not found.")),
        "username_taken" => Some(("این نام کاربری قبلاً گرفته شده.", "That username is taken.")),
        "invalid_username" => Some((
            "نام کاربری معتبر نیست (۳ تا ۶۴ نویسه: حرف، عدد و . _ - @).",
            "Not a valid username (3–64 letters, digits, . _ - @).",
        )),
        "user_limit_reached" => Some((
            "به سقف تعداد کاربرِ مجازتان رسیده‌اید.",
            "You have reached your limit of users.",
        )),
        "quota_exceeded" => Some((
            "سهمیه‌ی حجم شما کافی نیست.",
            "Your data quota is not enough.",
        )),
        "quota_unlimited" => Some((
            "کاربر باید حجم مشخص داشته باشد؛ سهمیه‌ی شما حجم را می‌شمارد.",
            "The user needs a data limit: your quota counts it.",
        )),
        "group_not_allowed" => Some((
            "اجازه‌ی دادن این گروه را ندارید.",
            "You may not give that group.",
        )),
        _ => None,
    };
    match known {
        Some((fa, en)) => format!("⚠️ {}", words.t(fa, en)),
        None => format!("⚠️ {}", escape(&error.message)),
    }
}

/// The admin a chat is tied to, as someone who can act, if they still can.
async fn admin_access(state: &AppState, admin_id: i64) -> Option<Access> {
    let admin = state
        .db()
        .admin_by_id(admin_id)
        .await
        .ok()
        .flatten()
        .filter(|a| a.enabled)?;
    assemble(
        state,
        admin,
        Via::Telegram,
        None,
        IpAddr::from([0, 0, 0, 0]),
    )
    .await
    .ok()
}

async fn on_text(state: &AppState, settings: &Settings, chat: &Chat, text: &str) {
    let words = Words::of(settings);
    if let Some(command) = text.strip_prefix('/') {
        let (head, rest) = command
            .split_once(char::is_whitespace)
            .unwrap_or((command, ""));
        // `/cmd@botname` in groups.
        let name = head.split('@').next().unwrap_or(head).to_ascii_lowercase();
        return command_of(state, settings, chat, &words, &name, rest.trim()).await;
    }
    // Not a command: a pasted subscription link is how a user introduces themselves.
    let link = state.db().telegram_link(chat.id).await.ok().flatten();
    if link.is_none() && chat.private && settings.telegram.user_bot {
        if let Some(token) = token_in(text) {
            return link_user(state, settings, chat, &words, &token).await;
        }
    }
    if link.is_none() {
        help(state, settings, chat, &words).await;
    }
}

/// A subscription token (32 hex characters) in `text`, which may be the
/// whole link.
pub fn token_in(text: &str) -> Option<String> {
    let mut run = String::new();
    for c in text.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_hexdigit() {
            run.push(c.to_ascii_lowercase());
        } else {
            if run.len() == 32 {
                return Some(run);
            }
            run.clear();
        }
    }
    None
}

async fn command_of(
    state: &AppState,
    settings: &Settings,
    chat: &Chat,
    words: &Words,
    name: &str,
    args: &str,
) {
    match name {
        "id" => {
            let line = format!(
                "{}: <code>{}</code>",
                words.t("شناسه‌ی این چت", "This chat's id"),
                chat.id
            );
            return say(state, settings, chat.id, &line, None).await;
        }
        "start" => return start(state, settings, chat, words, args).await,
        "help" | "" => return help(state, settings, chat, words).await,
        _ => {}
    }
    let link = state.db().telegram_link(chat.id).await.ok().flatten();
    let Some(link) = link.filter(|_| chat.private) else {
        return help(state, settings, chat, words).await;
    };
    if name == "unlink" {
        let _ = state.db().delete_telegram_link(chat.id).await;
        let text = words.t("این چت جدا شد.", "This chat is unlinked.");
        return say(state, settings, chat.id, text, None).await;
    }
    if link.kind == "admin" {
        let Some(access) = admin_access(state, link.ref_id).await else {
            let text = words.t(
                "حساب مدیر شما غیرفعال یا حذف شده است.",
                "Your admin account is off or gone.",
            );
            return say(state, settings, chat.id, text, None).await;
        };
        admin_command(state, settings, chat, words, &access, name, args).await;
    } else {
        user_command(state, settings, chat, words, link.ref_id, name).await;
    }
}

async fn help(state: &AppState, settings: &Settings, chat: &Chat, words: &Words) {
    let link = state.db().telegram_link(chat.id).await.ok().flatten();
    let text = match link.as_ref().map(|l| l.kind.as_str()) {
        Some("admin") => words
            .t(
                "<b>فرمان‌های مدیر</b>\n/stats — آمار\n/users [متن] — فهرست و جستجوی کاربران\n/user نام — کارت کاربر\n/add نام گیگ [روز] — ساخت کاربر\n/enable نام · /disable نام · /reset نام · /revoke نام · /delete نام\n/backup — بکاپ همین حالا (سودو)\n/unlink — جدا کردن این چت",
                "<b>Admin commands</b>\n/stats — figures\n/users [text] — list and search users\n/user name — a user's card\n/add name GB [days] — make a user\n/enable name · /disable name · /reset name · /revoke name · /delete name\n/backup — a backup now (sudo)\n/unlink — untie this chat",
            )
            .to_string(),
        Some(_) => words
            .t(
                "<b>فرمان‌ها</b>\n/usage — مصرف و زمان باقی‌مانده\n/link — لینک اشتراک\n/unlink — جدا کردن این چت",
                "<b>Commands</b>\n/usage — your usage and time left\n/link — your subscription link\n/unlink — untie this chat",
            )
            .to_string(),
        None => {
            let mut text = words
                .t(
                    "سلام! من ربات Z-UI هستم.\n\nاگر مدیر هستید: در پنل بخش «تنظیمات ← تلگرام» کدی بگیرید و <code>/start کد</code> را بفرستید.",
                    "Hello! I am the Z-UI bot.\n\nIf you are an admin: get a code in the panel under Settings → Telegram and send <code>/start code</code>.",
                )
                .to_string();
            if settings.telegram.user_bot {
                text.push_str(words.t(
                    "\n\nاگر کاربر هستید: لینک اشتراکتان را برای من بفرستید تا مصرفتان را نشان بدهم.",
                    "\n\nIf you are a user: send me your subscription link and I will show your usage.",
                ));
            }
            text.push_str(&format!("\n\n/id — {}", words.t("شناسه‌ی این چت", "this chat's id")));
            text
        }
    };
    say(state, settings, chat.id, &text, None).await;
}

async fn start(state: &AppState, settings: &Settings, chat: &Chat, words: &Words, code: &str) {
    if code.is_empty() {
        return help(state, settings, chat, words).await;
    }
    if !chat.private {
        let text = words.t(
            "برای وصل کردن، در چت خصوصی با من پیام بدهید.",
            "To link, write to me in a private chat.",
        );
        return say(state, settings, chat.id, text, None).await;
    }
    let tries_left = state.notifier().link_attempt(chat.id);
    let Some(admin_id) = tries_left.then(|| take_link_code(state, code)).flatten() else {
        let text = if tries_left {
            words.t(
                "این کد درست نیست یا منقضی شده است؛ در پنل کد تازه بگیرید.",
                "That code is wrong or has expired; get a new one in the panel.",
            )
        } else {
            words.t(
                "تلاش‌های زیاد؛ چند دقیقه بعد دوباره امتحان کنید.",
                "Too many tries; wait a few minutes.",
            )
        };
        return say(state, settings, chat.id, text, None).await;
    };
    let Ok(Some(admin)) = state.db().admin_by_id(admin_id).await else {
        return;
    };
    if state
        .db()
        .save_telegram_link(chat.id, "admin", admin.id, &chat.name)
        .await
        .is_err()
    {
        return;
    }
    audit::write(
        state,
        &Actor {
            admin_id: Some(admin.id),
            name: admin.username.clone(),
            via: "telegram".into(),
            address: IpAddr::from([0, 0, 0, 0]),
            quiet: false,
        },
        "auth.telegram_linked",
        &admin.username,
        &chat.name,
        200,
    )
    .await;
    let text = format!(
        "{} <b>{}</b>\n\n{}",
        words.t(
            "✅ این چت به حساب شما وصل شد:",
            "✅ This chat is linked to the account:"
        ),
        escape(&admin.username),
        words.t(
            "برای دیدن فرمان‌ها /help را بفرستید.",
            "Send /help to see what I can do."
        ),
    );
    say(state, settings, chat.id, &text, None).await;
}

async fn link_user(state: &AppState, settings: &Settings, chat: &Chat, words: &Words, token: &str) {
    if !state.notifier().link_attempt(chat.id) {
        let text = words.t(
            "تلاش‌های زیاد؛ چند دقیقه بعد دوباره امتحان کنید.",
            "Too many tries; wait a few minutes.",
        );
        return say(state, settings, chat.id, text, None).await;
    }
    match state.db().user_by_token(token).await {
        Ok(Some(user)) => {
            let _ = state
                .db()
                .save_telegram_link(chat.id, "user", user.id, &chat.name)
                .await;
            let text = format!(
                "{} <b>{}</b>\n\n{}",
                words.t("✅ وصل شد:", "✅ Linked:"),
                escape(&user.username),
                words.t(
                    "/usage — مصرف شما · /link — لینک اشتراک",
                    "/usage — your usage · /link — your subscription link"
                ),
            );
            say(state, settings, chat.id, &text, None).await;
        }
        _ => {
            let text = words.t(
                "این لینک اشتراک را نمی‌شناسم.",
                "I do not know that subscription link.",
            );
            say(state, settings, chat.id, text, None).await;
        }
    }
}

fn status_label(words: &Words, status: &str) -> String {
    let (icon, fa, en) = match status {
        "active" => ("🟢", "فعال", "Active"),
        "on_hold" => ("🟡", "در انتظار", "On hold"),
        "disabled" => ("⚫", "غیرفعال", "Disabled"),
        "limited" => ("🟠", "حجم تمام شد", "Out of data"),
        "expired" => ("🔴", "منقضی", "Expired"),
        other => ("•", other, other),
    };
    format!("{icon} {}", words.t(fa, en))
}

fn usage_line(words: &Words, user: &z_db::User) -> String {
    if user.data_limit > 0 {
        let percent = (i128::from(user.used_traffic) * 100 / i128::from(user.data_limit)) as i64;
        format!(
            "{} / {} ({percent}%)",
            bytes(user.used_traffic),
            bytes(user.data_limit)
        )
    } else {
        format!(
            "{} / {}",
            bytes(user.used_traffic),
            words.t("نامحدود", "unlimited")
        )
    }
}

fn expiry_line(words: &Words, user: &z_db::User, now: i64) -> String {
    match user.expire_at {
        Some(at) if at > now => {
            let days = (at - now + 86_399) / 86_400;
            format!(
                "{} ({} {})",
                date(at),
                days,
                words.t("روز مانده", "days left")
            )
        }
        Some(at) => format!("{} ({})", date(at), words.t("گذشته", "past")),
        None => match user.on_hold_duration {
            Some(seconds) => format!(
                "{} {}",
                seconds / 86_400,
                words.t("روز پس از اولین اتصال", "days after the first connection")
            ),
            None => words.t("بدون تاریخ", "no date").to_string(),
        },
    }
}

/// A user's card, and the buttons an admin has under it.
fn card(words: &Words, user: &z_db::User, now: i64, buttons: bool) -> (String, Option<Value>) {
    let mut text = format!(
        "<b>{}</b>  {}\n",
        escape(&user.username),
        status_label(words, &user.status)
    );
    text.push_str(&format!(
        "{}: <code>{}</code>\n",
        words.t("مصرف", "Used"),
        usage_line(words, user)
    ));
    text.push_str(&format!(
        "{}: <code>{}</code>",
        words.t("پایان", "Ends"),
        expiry_line(words, user, now)
    ));
    let keyboard = buttons.then(|| {
        let id = user.id;
        let toggle = if user.status == "disabled" {
            (
                words.t("▶️ فعال کردن", "▶️ Enable"),
                format!("u:toggle:{id}"),
            )
        } else {
            (words.t("⏸ غیرفعال", "⏸ Disable"), format!("u:toggle:{id}"))
        };
        let button = |(label, data): (&str, String)| json!({"text": label, "callback_data": data});
        json!([
            [
                button(toggle),
                button((
                    words.t("🔄 بازنشانی مصرف", "🔄 Reset usage"),
                    format!("u:reset:{id}")
                ))
            ],
            [
                button((words.t("🔗 لینک", "🔗 Link"), format!("u:link:{id}"))),
                button((
                    words.t("🔑 اعتبار تازه", "🔑 New credentials"),
                    format!("u:revoke:{id}")
                ))
            ],
            [button((
                words.t("🗑 حذف", "🗑 Delete"),
                format!("u:del:{id}")
            ))],
        ])
    });
    (text, keyboard)
}

fn subscription_text(state: &AppState, words: &Words, user: &z_db::User) -> String {
    let sub = state.subscription();
    let url = crate::subscription::url(&sub, "", &user.sub_token);
    if sub.public_url.is_empty() {
        format!(
            "{}\n<code>{}</code>\n{}",
            words.t("لینک اشتراک (آدرس عمومی پنل در تنظیمات اشتراک خالی است، پس فقط مسیر دارد):", "Subscription link (the panel's public address is empty in the subscription settings, so this is only the path):"),
            escape(&url),
            words.t("آدرس پنل را جلوی آن بگذارید.", "Put the panel's address in front of it."),
        )
    } else {
        format!(
            "{}\n<code>{}</code>",
            words.t("لینک اشتراک:", "Subscription link:"),
            escape(&url)
        )
    }
}

async fn user_command(
    state: &AppState,
    settings: &Settings,
    chat: &Chat,
    words: &Words,
    user_id: i64,
    name: &str,
) {
    let Some(user) = state.db().user(user_id).await.ok().flatten() else {
        return;
    };
    match name {
        "usage" | "me" | "status" => {
            let (text, _) = card(words, &user, z_db::now(), false);
            say(state, settings, chat.id, &text, None).await;
        }
        "link" | "sub" => {
            let text = subscription_text(state, words, &user);
            say(state, settings, chat.id, &text, None).await;
        }
        _ => help(state, settings, chat, words).await,
    }
}

/// Find the user a command names: one exactly named, or what a search finds.
enum Found {
    One(Box<z_db::User>),
    Many(Vec<z_db::User>),
    None,
}

async fn find(state: &AppState, access: &Access, query: &str) -> Found {
    let query = query.trim();
    if query.is_empty() {
        return Found::None;
    }
    let (users, _) = state
        .db()
        .users(&z_db::UserQuery {
            search: Some(query.to_string()),
            owner: access.owner_scope(),
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap_or_default();
    let users: Vec<_> = users.into_iter().filter(|u| access.sees(u)).collect();
    if let Some(exact) = users
        .iter()
        .find(|u| u.username.eq_ignore_ascii_case(query))
    {
        return Found::One(Box::new(exact.clone()));
    }
    match users.len() {
        0 => Found::None,
        1 => Found::One(Box::new(users.into_iter().next().expect("one"))),
        _ => Found::Many(users),
    }
}

async fn admin_command(
    state: &AppState,
    settings: &Settings,
    chat: &Chat,
    words: &Words,
    access: &Access,
    name: &str,
    args: &str,
) {
    let now = z_db::now();
    let read = access.require(Section::Users, Level::Read).is_ok();
    let write = access.require(Section::Users, Level::Write).is_ok();
    let denied = || {
        say(
            state,
            settings,
            chat.id,
            words.t("⚠️ اجازه‌ی این کار را ندارید.", "⚠️ You may not do this."),
            None,
        )
    };
    match name {
        "me" => {
            let who = if access.admin.is_sudo {
                words.t("سودو", "sudo").to_string()
            } else {
                access
                    .role
                    .as_ref()
                    .map(|r| r.name.clone())
                    .unwrap_or_default()
            };
            let text = format!(
                "<b>{}</b> · {}",
                escape(&access.admin.username),
                escape(&who)
            );
            say(state, settings, chat.id, &text, None).await;
        }
        "stats" => {
            if !read {
                return denied().await;
            }
            let text = stats(state, words, access).await;
            say(state, settings, chat.id, &text, None).await;
        }
        "users" | "find" => {
            if !read {
                return denied().await;
            }
            let (users, total) = state
                .db()
                .users(&z_db::UserQuery {
                    search: (!args.is_empty()).then(|| args.to_string()),
                    owner: access.owner_scope(),
                    limit: 10,
                    ..Default::default()
                })
                .await
                .unwrap_or_default();
            if users.is_empty() {
                return say(
                    state,
                    settings,
                    chat.id,
                    words.t("کاربری پیدا نشد.", "No users found."),
                    None,
                )
                .await;
            }
            list(state, settings, chat.id, words, &users, total).await;
        }
        "user" | "u" => {
            if !read {
                return denied().await;
            }
            match find(state, access, args).await {
                Found::One(user) => {
                    let (text, keyboard) = card(words, &user, now, write);
                    say(state, settings, chat.id, &text, keyboard).await;
                }
                Found::Many(users) => {
                    list(state, settings, chat.id, words, &users, users.len() as i64).await
                }
                Found::None => {
                    let text = if args.is_empty() {
                        words.t(
                            "نام کاربر را بنویسید: /user نام",
                            "Give the username: /user name",
                        )
                    } else {
                        words.t("کاربری با این نام نیست.", "No such user.")
                    };
                    say(state, settings, chat.id, text, None).await;
                }
            }
        }
        "enable" | "disable" | "reset" | "revoke" | "delete" => {
            if !write {
                return denied().await;
            }
            match find(state, access, args).await {
                Found::One(user) => {
                    if name == "delete" {
                        return confirm_delete(state, settings, chat.id, words, &user).await;
                    }
                    let action = match name {
                        "enable" => "enable",
                        "disable" => "disable",
                        "reset" => "reset",
                        _ => "revoke",
                    };
                    let outcome = act(state, access, action, user.id).await;
                    reply_action(
                        state, settings, chat.id, words, access, outcome, user.id, None,
                    )
                    .await;
                }
                Found::Many(users) => {
                    list(state, settings, chat.id, words, &users, users.len() as i64).await
                }
                Found::None => {
                    let text = words.t("کاربری با این نام نیست.", "No such user.");
                    say(state, settings, chat.id, text, None).await;
                }
            }
        }
        "add" => {
            if !write {
                return denied().await;
            }
            let outcome = add(state, access, args, now).await;
            match outcome {
                Ok(user) => {
                    let (text, keyboard) = card(words, &user, now, true);
                    say(
                        state,
                        settings,
                        chat.id,
                        &format!("✅ {}\n\n{text}", words.t("ساخته شد", "Made")),
                        keyboard,
                    )
                    .await;
                }
                Err(Failed::Usage) => {
                    let text = words.t(
                        "استفاده: <code>/add نام گیگ [روز]</code> (گیگ ۰ یعنی نامحدود)",
                        "Usage: <code>/add name GB [days]</code> (0 GB means unlimited)",
                    );
                    say(state, settings, chat.id, text, None).await;
                }
                Err(Failed::Api(error)) => {
                    say(state, settings, chat.id, &failure(words, &error), None).await
                }
            }
        }
        "backup" => {
            if access.require_sudo().is_err() {
                return denied().await;
            }
            let text = match crate::backup::run(state, Some(chat.id)).await {
                Ok(done) => format!(
                    "✅ {} <code>{}</code> ({})",
                    words.t("بکاپ ساخته و فرستاده شد:", "Backup made and sent:"),
                    escape(&done.file),
                    bytes(done.bytes as i64)
                ),
                Err(error) => format!("⚠️ {}", escape(&error)),
            };
            say(state, settings, chat.id, &text, None).await;
        }
        _ => help(state, settings, chat, words).await,
    }
}

async fn list(
    state: &AppState,
    settings: &Settings,
    chat: i64,
    words: &Words,
    users: &[z_db::User],
    total: i64,
) {
    let mut text = String::new();
    let mut rows = Vec::new();
    for user in users {
        text.push_str(&format!(
            "• <b>{}</b> — {} — <code>{}</code>\n",
            escape(&user.username),
            status_label(words, &user.status),
            usage_line(words, user)
        ));
        rows.push(json!([{"text": user.username, "callback_data": format!("u:card:{}", user.id)}]));
    }
    if total > users.len() as i64 {
        text.push_str(&format!(
            "\n{}",
            words.t(
                &format!("{total} کاربر؛ ده تای اول. جستجو کنید: /users متن"),
                &format!("{total} users; the first ten. Narrow it: /users text")
            )
        ));
    }
    say(state, settings, chat, &text, Some(Value::Array(rows))).await;
}

async fn stats(state: &AppState, words: &Words, access: &Access) -> String {
    let owner = access.owner_scope();
    let counts = state.db().user_counts(owner).await.unwrap_or_default();
    let get = |key: &str| counts.get(key).copied().unwrap_or(0);
    let total: i64 = counts.values().sum();
    let online = state
        .db()
        .all_users()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|u| owner.is_none() || u.admin_id == owner)
        .filter(|u| {
            u.online_at
                .is_some_and(|at| z_db::now() - at <= 3 * crate::jobs::INTERVAL.as_secs() as i64)
        })
        .count();
    let lifetime = state.db().lifetime_total(owner).await.unwrap_or(0);
    let mut text = format!(
        "<b>{}</b>\n{}: <code>{total}</code> · {}: <code>{online}</code>\n🟢 {}  🟡 {}  🟠 {}  🔴 {}  ⚫ {}\n{}: <code>{}</code>",
        words.t("آمار", "Figures"),
        words.t("کاربران", "Users"),
        words.t("آنلاین", "online"),
        get("active"),
        get("on_hold"),
        get("limited"),
        get("expired"),
        get("disabled"),
        words.t("کل ترافیک", "All traffic"),
        bytes(lifetime),
    );
    if owner.is_none() {
        let status = state.node().status().await;
        text.push_str(&format!(
            "\n{}: {}",
            words.t("هسته", "Core"),
            if status.running {
                words.t("🟢 روشن", "🟢 running")
            } else {
                words.t("🔴 خاموش", "🔴 stopped")
            }
        ));
        if access.can(Section::Monitor, Level::Read) {
            if let Some(now) = state.monitor().latest() {
                if let (Some(cpu), Some(used), Some(total)) = (now.cpu, now.mem_used, now.mem_total)
                {
                    text.push_str(&format!(
                        "\nCPU <code>{cpu:.0}%</code> · RAM <code>{} / {}</code>",
                        bytes(used as i64),
                        bytes(total as i64)
                    ));
                }
            }
        }
    }
    text
}

enum Failed {
    Usage,
    Api(ApiError),
}

impl From<ApiError> for Failed {
    fn from(error: ApiError) -> Self {
        Failed::Api(error)
    }
}

/// `/add name GB [days]`.
async fn add(
    state: &AppState,
    access: &Access,
    args: &str,
    now: i64,
) -> Result<z_db::User, Failed> {
    let mut parts = args.split_whitespace();
    let (Some(name), Some(gb)) = (parts.next(), parts.next()) else {
        return Err(Failed::Usage);
    };
    let gb: f64 = gb.parse().map_err(|_| Failed::Usage)?;
    let days: i64 = parts
        .next()
        .map_or(Ok(0), str::parse)
        .map_err(|_| Failed::Usage)?;
    if parts.next().is_some() || gb < 0.0 || !gb.is_finite() || !(0..=3650).contains(&days) {
        return Err(Failed::Usage);
    }
    // The first group this admin may give.
    let groups = state.db().access_groups().await.map_err(ApiError::from)?;
    let group = groups
        .iter()
        .find(|g| {
            access
                .group_scope()
                .is_none_or(|allowed| allowed.contains(&g.id))
        })
        .map(|g| g.id)
        .ok_or_else(|| ApiError::bad_request("invalid_group", "there is no group to give"))?;
    let input = UserInput::basic(
        name,
        (gb * (1u64 << 30) as f64) as i64,
        (days > 0).then_some(now + days * 86_400),
        vec![group],
    );
    let (_, said, Json(view)) =
        users::create(State(state.clone()), access.clone(), Json(input)).await?;
    write_audit(state, access, &said).await;
    state
        .db()
        .user(view.id())
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| Failed::Api(ApiError::not_found("the user was not made")))
}

async fn write_audit(state: &AppState, access: &Access, said: &Audit) {
    let (action, target, detail) = said.parts();
    audit::write(
        state,
        &Actor::of(access, false),
        action,
        target,
        detail,
        200,
    )
    .await;
}

/// Do one thing to one user through the same code the API runs.
async fn act(state: &AppState, access: &Access, action: &str, id: i64) -> Result<(), ApiError> {
    let said = match action {
        "enable" | "disable" => {
            let which = if action == "enable" {
                BulkAction::Enable
            } else {
                BulkAction::Disable
            };
            let (said, Json(result)) = users::bulk(
                State(state.clone()),
                access.clone(),
                Json(BulkRequest::one(id, which)),
            )
            .await?;
            if result.changed() == 0 {
                return Err(ApiError::not_found("there is no such user"));
            }
            said
        }
        "reset" => {
            users::reset(State(state.clone()), access.clone(), Path(id))
                .await?
                .0
        }
        "revoke" => {
            users::revoke(State(state.clone()), access.clone(), Path(id))
                .await?
                .0
        }
        "delete" => {
            users::delete(State(state.clone()), access.clone(), Path(id))
                .await?
                .1
        }
        _ => return Err(ApiError::bad_request("invalid_action", "no such action")),
    };
    write_audit(state, access, &said).await;
    Ok(())
}

/// Say how an action went, and show the user as they are now.
#[allow(clippy::too_many_arguments)]
async fn reply_action(
    state: &AppState,
    settings: &Settings,
    chat: i64,
    words: &Words,
    access: &Access,
    outcome: Result<(), ApiError>,
    id: i64,
    message: Option<i64>,
) {
    match outcome {
        Ok(()) => match state
            .db()
            .user(id)
            .await
            .ok()
            .flatten()
            .filter(|u| access.sees(u))
        {
            Some(user) => {
                let (text, keyboard) = card(words, &user, z_db::now(), true);
                let text = format!("✅ {}\n\n{text}", words.t("انجام شد", "Done"));
                show(state, settings, chat, message, &text, keyboard).await;
            }
            None => {
                say(
                    state,
                    settings,
                    chat,
                    words.t("✅ انجام شد", "✅ Done"),
                    None,
                )
                .await
            }
        },
        Err(error) => say(state, settings, chat, &failure(words, &error), None).await,
    }
}

/// Put `text` where the buttons were pressed, or in a new message.
async fn show(
    state: &AppState,
    settings: &Settings,
    chat: i64,
    message: Option<i64>,
    text: &str,
    keyboard: Option<Value>,
) {
    if let Some(message) = message {
        let mut body = json!({"chat_id": chat, "message_id": message, "text": text, "parse_mode": "HTML", "disable_web_page_preview": true});
        if let Some(keyboard) = &keyboard {
            body["reply_markup"] = json!({"inline_keyboard": keyboard});
        }
        if channels::telegram_call(settings, "editMessageText", &body)
            .await
            .is_ok()
        {
            return;
        }
    }
    say(state, settings, chat, text, keyboard).await;
}

async fn confirm_delete(
    state: &AppState,
    settings: &Settings,
    chat: i64,
    words: &Words,
    user: &z_db::User,
) {
    let text = format!(
        "🗑 {} <b>{}</b>؟",
        words.t("حذف", "Delete"),
        escape(&user.username)
    );
    let text = if words.0 {
        text
    } else {
        text.replace('؟', "?")
    };
    let keyboard = json!([[
        {"text": words.t("بله، حذف شود", "Yes, delete"), "callback_data": format!("u:delok:{}", user.id)},
        {"text": words.t("انصراف", "Cancel"), "callback_data": format!("u:card:{}", user.id)},
    ]]);
    say(state, settings, chat, &text, Some(keyboard)).await;
}

async fn on_button(state: &AppState, settings: &Settings, query: &Value) {
    let words = Words::of(settings);
    let Some(id) = query["id"].as_str() else {
        return;
    };
    let answer = |text: &str| {
        let body = json!({"callback_query_id": id, "text": text});
        let settings = settings.clone();
        async move {
            let _ = channels::telegram_call(&settings, "answerCallbackQuery", &body).await;
        }
    };
    let Some(chat) = query.get("message").and_then(chat_of) else {
        return answer("").await;
    };
    let message = query["message"]["message_id"].as_i64();
    let link = state.db().telegram_link(chat.id).await.ok().flatten();
    let Some(access) = (match link.filter(|l| l.kind == "admin" && chat.private) {
        Some(link) => admin_access(state, link.ref_id).await,
        None => None,
    }) else {
        return answer(words.t("اجازه ندارید", "Not allowed")).await;
    };
    let mut parts = query["data"].as_str().unwrap_or("").splitn(3, ':');
    let (Some("u"), Some(what), Some(user)) = (
        parts.next(),
        parts.next(),
        parts.next().and_then(|u| u.parse::<i64>().ok()),
    ) else {
        return answer("").await;
    };
    let write = access.require(Section::Users, Level::Write).is_ok();
    let user_row = state
        .db()
        .user(user)
        .await
        .ok()
        .flatten()
        .filter(|u| access.sees(u));
    let Some(user_row) = user_row else {
        return answer(words.t("پیدا نشد", "Not found")).await;
    };
    match what {
        "card" => {
            let (text, keyboard) = card(&words, &user_row, z_db::now(), write);
            answer("").await;
            show(state, settings, chat.id, message, &text, keyboard).await;
        }
        "link" => {
            answer("").await;
            let text = subscription_text(state, &words, &user_row);
            say(state, settings, chat.id, &text, None).await;
        }
        "del" if write => {
            answer("").await;
            confirm_delete(state, settings, chat.id, &words, &user_row).await;
        }
        "toggle" | "reset" | "revoke" | "delok" if write => {
            let action = match what {
                "toggle" if user_row.status == "disabled" => "enable",
                "toggle" => "disable",
                "delok" => "delete",
                other => other,
            };
            let outcome = act(state, &access, action, user).await;
            answer(if outcome.is_ok() {
                words.t("انجام شد", "Done")
            } else {
                words.t("ناموفق", "Failed")
            })
            .await;
            if action == "delete" {
                let text = if outcome.is_ok() {
                    words.t("🗑 حذف شد", "🗑 Deleted").to_string()
                } else {
                    failure(&words, outcome.as_ref().expect_err("failed"))
                };
                show(state, settings, chat.id, message, &text, None).await;
            } else {
                reply_action(
                    state, settings, chat.id, &words, &access, outcome, user, message,
                )
                .await;
            }
        }
        _ => answer(words.t("اجازه ندارید", "Not allowed")).await,
    }
}

impl From<SendError> for ApiError {
    fn from(error: SendError) -> Self {
        ApiError::bad_request("delivery_failed", error.message)
    }
}

#[cfg(test)]
mod tests {
    use super::super::http::testing::{mock, ok, Mock};
    use super::*;
    use crate::server::testing::{panel, Panel};

    const GB: i64 = 1 << 30;

    async fn world(telegram: &Mock) -> Panel {
        let panel = panel("/").await;
        let mut settings = Settings {
            language: "en".into(),
            ..Default::default()
        };
        settings.telegram.enabled = true;
        settings.telegram.token = "123:abc".into();
        settings.telegram.api_base = telegram.url();
        settings.telegram.user_bot = true;
        panel.state.set_notify_settings(settings.checked().unwrap());
        panel.state.notifier().retry_after(vec![]);
        panel
    }

    fn text(chat: i64, text: &str) -> Value {
        json!({"update_id": 1, "message": {"message_id": 5, "chat": {"id": chat, "type": "private", "first_name": "Ali"}, "text": text}})
    }

    fn group(chat: i64, text: &str) -> Value {
        json!({"update_id": 1, "message": {"message_id": 5, "chat": {"id": chat, "type": "supergroup", "title": "Team"}, "text": text}})
    }

    fn button(chat: i64, data: &str) -> Value {
        json!({"update_id": 2, "callback_query": {"id": "cb1", "data": data, "message": {"message_id": 9, "chat": {"id": chat, "type": "private"}}}})
    }

    /// Say `update` to the bot and return what it sent back, as `(chat, text, buttons)`.
    async fn hear(panel: &Panel, telegram: &Mock, update: Value) -> Vec<(i64, String, Value)> {
        let before = telegram.requests().len();
        Box::pin(handle(
            &panel.state,
            &panel.state.notify_settings(),
            &update,
        ))
        .await;
        telegram.requests()[before..]
            .iter()
            .filter(|r| r.path.ends_with("/sendMessage") || r.path.ends_with("/editMessageText"))
            .map(|r| {
                let body = r.json();
                (
                    body["chat_id"].as_i64().unwrap(),
                    body["text"].as_str().unwrap().to_string(),
                    body["reply_markup"]["inline_keyboard"].clone(),
                )
            })
            .collect()
    }

    async fn admin(panel: &Panel, name: &str, sudo: bool) -> z_db::Admin {
        let db = panel.state.db();
        if sudo {
            return db.create_admin(name, "h", true).await.unwrap();
        }
        let role = match db
            .roles()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.name == "Reseller")
        {
            Some(role) => role,
            None => unreachable!("the default roles exist"),
        };
        db.create_admin_with(
            name,
            "h",
            &z_db::AdminFields {
                role_id: Some(role.id),
                user_limit: 1,
                data_quota: 2 * GB,
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    async fn user(
        panel: &Panel,
        name: &str,
        owner: Option<i64>,
        limit: i64,
        used: i64,
    ) -> z_db::User {
        let default = panel.state.db().access_groups().await.unwrap()[0].id;
        let fields = z_db::UserFields {
            username: name.into(),
            status: "active".into(),
            data_limit: limit,
            reset_strategy: "no_reset".into(),
            group_ids: vec![default],
            ..Default::default()
        };
        let made = panel
            .state
            .db()
            .create_user(
                &fields,
                name,
                name,
                &hex::encode(rand::random::<[u8; 16]>()),
                owner,
            )
            .await
            .unwrap();
        if used > 0 {
            panel
                .state
                .db()
                .add_traffic(&[z_db::TrafficDelta {
                    user_id: made.id,
                    uplink: used,
                    downlink: 0,
                }])
                .await
                .unwrap();
        }
        made
    }

    #[test]
    fn a_token_is_found_in_a_link_or_alone_and_nothing_else_is() {
        let token = "0123456789abcdef0123456789ABCDEF";
        assert_eq!(
            token_in(token).as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            token_in(&format!("https://panel.example/sub/{token}")).as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            token_in(&format!("my link: {token}?format=json")).as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(token_in("0123456789abcdef"), None, "too short");
        assert_eq!(
            token_in(&format!("{token}0")),
            None,
            "too long: a longer run is not a token"
        );
        assert_eq!(token_in("hello there"), None);
    }

    #[test]
    fn a_chat_gets_a_few_tries_at_linking() {
        let mut tries = Attempts::default();
        let now = Instant::now();
        for _ in 0..LINK_TRIES {
            assert!(tries.allow(7, now));
        }
        assert!(!tries.allow(7, now), "then it waits");
        assert!(tries.allow(8, now), "another chat is not held");
        assert!(
            tries.allow(7, now + LINK_WINDOW + Duration::from_secs(1)),
            "and the wait ends"
        );
    }

    #[tokio::test]
    async fn an_admin_links_a_chat_with_a_code_that_works_once() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        let boss = admin(&panel, "boss", true).await;
        let code = crate::notify::issue_link_code(&panel.state, boss.id);

        let said = hear(
            &panel,
            &telegram,
            text(111, &format!("/start {}", code.to_lowercase())),
        )
        .await;
        assert!(
            said[0].1.contains("linked to the account") && said[0].1.contains("<b>boss</b>"),
            "{said:?}"
        );
        let link = panel.state.db().telegram_link(111).await.unwrap().unwrap();
        assert_eq!(
            (link.kind.as_str(), link.ref_id, link.name.as_str()),
            ("admin", boss.id, "Ali")
        );

        let again = hear(&panel, &telegram, text(222, &format!("/start {code}"))).await;
        assert!(again[0].1.contains("wrong or has expired"), "{again:?}");
        assert!(panel.state.db().telegram_link(222).await.unwrap().is_none());
        let wrong = hear(&panel, &telegram, text(222, "/start NOTACODE")).await;
        assert!(wrong[0].1.contains("wrong or has expired"));

        // The link shows in the audit log as the admin's own act, by Telegram.
        let log = panel
            .state
            .db()
            .audit(&z_db::AuditQuery {
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(log.iter().any(|e| e.action == "auth.telegram_linked"
            && e.via == "telegram"
            && e.admin_name == "boss"));
    }

    #[tokio::test]
    async fn a_chat_that_is_nobody_gets_its_id_and_how_to_link_and_nothing_more() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        let boss = admin(&panel, "boss", true).await;
        user(&panel, "ali", None, GB, 0).await;

        let id = hear(&panel, &telegram, text(333, "/id")).await;
        assert!(id[0].1.contains("<code>333</code>"));
        for command in [
            "/stats",
            "/users",
            "/user ali",
            "/disable ali",
            "/add x 1",
            "/backup",
            "hello",
        ] {
            let said = hear(&panel, &telegram, text(333, command)).await;
            assert_eq!(said.len(), 1, "{command}");
            assert!(
                said[0].1.contains("I am the Z-UI bot") && said[0].1.contains("subscription link"),
                "{command}: {}",
                said[0].1
            );
        }
        assert_eq!(
            panel.state.db().user(1).await.unwrap().unwrap().status,
            "active"
        );
        // A code does nothing in a group, and the chat is not tied there.
        let code = crate::notify::issue_link_code(&panel.state, boss.id);
        let said = hear(&panel, &telegram, group(-500, &format!("/start {code}"))).await;
        assert!(said[0].1.contains("private chat"));
        assert!(panel
            .state
            .db()
            .telegram_link(-500)
            .await
            .unwrap()
            .is_none());
        // With the user bot off, users are not asked for their link.
        let mut settings = panel.state.notify_settings();
        settings.telegram.user_bot = false;
        panel.state.set_notify_settings(settings);
        let said = hear(&panel, &telegram, text(334, "/start")).await;
        assert!(!said[0].1.contains("subscription link"));
    }

    #[tokio::test]
    async fn an_admin_runs_the_panel_from_the_chat() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        let boss = admin(&panel, "boss", true).await;
        panel
            .state
            .db()
            .save_telegram_link(111, "admin", boss.id, "Ali")
            .await
            .unwrap();
        let ali = user(&panel, "ali", None, 10 * GB, 4 * GB).await;
        user(&panel, "alice", None, 0, 0).await;

        let stats = hear(&panel, &telegram, text(111, "/stats")).await;
        assert!(
            stats[0].1.contains("Users: <code>2</code>") && stats[0].1.contains("Core"),
            "{}",
            stats[0].1
        );
        let list = hear(&panel, &telegram, text(111, "/users ali")).await;
        assert!(
            list[0].1.contains("<b>ali</b>") && list[0].1.contains("<b>alice</b>"),
            "{}",
            list[0].1
        );
        assert_eq!(list[0].2.as_array().unwrap().len(), 2, "a button for each");

        let card = hear(&panel, &telegram, text(111, "/user ali")).await;
        assert!(
            card[0].1.contains("<b>ali</b>") && card[0].1.contains("4.0 GB / 10.0 GB (40%)"),
            "{}",
            card[0].1
        );
        assert_eq!(
            card[0].2[0][0]["callback_data"],
            format!("u:toggle:{}", ali.id)
        );

        // Off, and on again, through the same code the API runs.
        let off = hear(&panel, &telegram, text(111, "/disable ali")).await;
        assert!(
            off[0].1.contains("Done") && off[0].1.contains("Disabled"),
            "{}",
            off[0].1
        );
        assert_eq!(
            panel.state.db().user(ali.id).await.unwrap().unwrap().status,
            "disabled"
        );
        hear(&panel, &telegram, text(111, "/enable ali")).await;
        assert_eq!(
            panel.state.db().user(ali.id).await.unwrap().unwrap().status,
            "active"
        );
        hear(&panel, &telegram, text(111, "/reset ali")).await;
        assert_eq!(
            panel
                .state
                .db()
                .user(ali.id)
                .await
                .unwrap()
                .unwrap()
                .used_traffic,
            0
        );
        hear(&panel, &telegram, text(111, "/revoke ali")).await;
        assert_ne!(
            panel.state.db().user(ali.id).await.unwrap().unwrap().uuid,
            ali.uuid
        );
        let log = panel
            .state
            .db()
            .audit(&z_db::AuditQuery {
                limit: 20,
                ..Default::default()
            })
            .await
            .unwrap();
        for action in ["user.bulk", "user.reset", "user.revoke"] {
            assert!(
                log.iter()
                    .any(|e| e.action == action && e.via == "telegram"),
                "{action} is in the log"
            );
        }
    }

    #[tokio::test]
    async fn an_admin_makes_and_deletes_users_from_the_chat() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        let boss = admin(&panel, "boss", true).await;
        panel
            .state
            .db()
            .save_telegram_link(111, "admin", boss.id, "Ali")
            .await
            .unwrap();

        let made = hear(&panel, &telegram, text(111, "/add bob 5 30")).await;
        assert!(
            made[0].1.contains("Made") && made[0].1.contains("<b>bob</b>"),
            "{}",
            made[0].1
        );
        let bob = panel
            .state
            .db()
            .users(&z_db::UserQuery {
                search: Some("bob".into()),
                limit: 5,
                ..Default::default()
            })
            .await
            .unwrap()
            .0
            .remove(0);
        assert_eq!(bob.data_limit, 5 * GB);
        assert!(bob.expire_at.unwrap() - z_db::now() > 29 * 86_400);
        assert_eq!(bob.admin_id, Some(boss.id));
        let usage = hear(&panel, &telegram, text(111, "/add bob")).await;
        assert!(usage[0].1.contains("Usage"));
        let taken = hear(&panel, &telegram, text(111, "/add bob 1")).await;
        assert!(taken[0].1.contains("taken"), "{}", taken[0].1);

        let ask = hear(&panel, &telegram, text(111, "/delete bob")).await;
        assert!(
            ask[0].1.contains("Delete")
                && ask[0].2[0][0]["callback_data"] == format!("u:delok:{}", bob.id)
        );
        assert!(
            panel.state.db().user(bob.id).await.unwrap().is_some(),
            "asked, not done"
        );
        let sure = hear(
            &panel,
            &telegram,
            button(111, &format!("u:delok:{}", bob.id)),
        )
        .await;
        assert!(sure[0].1.contains("Deleted"));
        assert!(panel.state.db().user(bob.id).await.unwrap().is_none());
        assert!(telegram
            .requests()
            .iter()
            .any(|r| r.path.ends_with("/answerCallbackQuery")));

        let none = hear(&panel, &telegram, text(111, "/user nobody")).await;
        assert!(none[0].1.contains("No such user"));
        let unlinked = hear(&panel, &telegram, text(111, "/unlink")).await;
        assert!(
            unlinked[0].1.contains("unlinked")
                && panel.state.db().telegram_link(111).await.unwrap().is_none()
        );
    }

    #[tokio::test]
    async fn a_reseller_in_the_chat_is_held_to_their_users_their_quota_and_their_rights() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        let boss = admin(&panel, "boss", true).await;
        let shop = admin(&panel, "shop", false).await;
        panel
            .state
            .db()
            .save_telegram_link(444, "admin", shop.id, "Shop")
            .await
            .unwrap();
        user(&panel, "bossuser", Some(boss.id), GB, 0).await;

        let find = hear(&panel, &telegram, text(444, "/user bossuser")).await;
        assert!(
            find[0].1.contains("No such user"),
            "another admin's user is not there: {}",
            find[0].1
        );
        let listed = hear(&panel, &telegram, text(444, "/users")).await;
        assert!(listed[0].1.contains("No users found"));
        let off = hear(&panel, &telegram, text(444, "/disable bossuser")).await;
        assert!(off[0].1.contains("No such user"));

        let made = hear(&panel, &telegram, text(444, "/add mine 1")).await;
        assert!(made[0].1.contains("Made"), "{}", made[0].1);
        let over = hear(&panel, &telegram, text(444, "/add second 1")).await;
        assert!(
            over[0].1.contains("limit of users"),
            "the shop may own one user: {}",
            over[0].1
        );
        let unlimited = hear(&panel, &telegram, text(444, "/add third 0")).await;
        assert!(
            unlimited[0].1.contains("limit of users") || unlimited[0].1.contains("data limit"),
            "{}",
            unlimited[0].1
        );
        let stats = hear(&panel, &telegram, text(444, "/stats")).await;
        assert!(
            stats[0].1.contains("Users: <code>1</code>") && !stats[0].1.contains("Core"),
            "{}",
            stats[0].1
        );
        let backup = hear(&panel, &telegram, text(444, "/backup")).await;
        assert!(backup[0].1.contains("may not"), "{}", backup[0].1);

        // A button for a user that is not theirs does nothing.
        let before = panel.state.db().user(1).await.unwrap().unwrap().status;
        hear(&panel, &telegram, button(444, "u:toggle:1")).await;
        assert_eq!(
            panel.state.db().user(1).await.unwrap().unwrap().status,
            before
        );
        // And a disabled admin's chat is shut.
        panel
            .state
            .db()
            .update_admin(
                shop.id,
                &z_db::AdminFields {
                    enabled: false,
                    role_id: shop.role_id,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let shut = hear(&panel, &telegram, text(444, "/stats")).await;
        assert!(shut[0].1.contains("off or gone"));
    }

    #[tokio::test]
    async fn a_user_links_with_their_subscription_link_and_sees_only_their_own_usage() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        let mut sub = panel.state.subscription();
        sub.public_url = "https://sub.example".into();
        panel.state.set_subscription(sub);
        let sara = user(&panel, "sara", None, 10 * GB, 3 * GB).await;
        user(&panel, "other", None, GB, 0).await;

        let wrong = hear(
            &panel,
            &telegram,
            text(
                555,
                "https://sub.example/sub/00000000000000000000000000000000",
            ),
        )
        .await;
        assert!(wrong[0].1.contains("do not know"), "{}", wrong[0].1);
        let linked = hear(
            &panel,
            &telegram,
            text(555, &format!("https://sub.example/sub/{}", sara.sub_token)),
        )
        .await;
        assert!(
            linked[0].1.contains("Linked") && linked[0].1.contains("<b>sara</b>"),
            "{}",
            linked[0].1
        );
        assert_eq!(
            panel
                .state
                .db()
                .telegram_link(555)
                .await
                .unwrap()
                .unwrap()
                .kind,
            "user"
        );

        let usage = hear(&panel, &telegram, text(555, "/usage")).await;
        assert!(
            usage[0].1.contains("3.0 GB / 10.0 GB (30%)") && usage[0].2.is_null(),
            "no buttons for a user: {usage:?}"
        );
        let link = hear(&panel, &telegram, text(555, "/link")).await;
        assert!(link[0]
            .1
            .contains(&format!("https://sub.example/sub/{}", sara.sub_token)));
        // Admin things are not theirs, and they cannot see anyone else.
        for command in [
            "/stats",
            "/users",
            "/user other",
            "/disable sara",
            "/add x 1",
            "/backup",
        ] {
            let said = hear(&panel, &telegram, text(555, command)).await;
            assert!(said[0].1.contains("/usage"), "{command}: {}", said[0].1);
        }
        assert_eq!(
            panel
                .state
                .db()
                .user(sara.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "active"
        );
        let pressed = hear(
            &panel,
            &telegram,
            button(555, &format!("u:toggle:{}", sara.id)),
        )
        .await;
        assert!(pressed.is_empty());
        assert_eq!(
            panel
                .state
                .db()
                .user(sara.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "active"
        );
    }

    #[tokio::test]
    async fn guessing_links_is_slowed_and_with_the_user_bot_off_nobody_can_link() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        for _ in 0..LINK_TRIES {
            hear(
                &panel,
                &telegram,
                text(666, "00000000000000000000000000000000"),
            )
            .await;
        }
        let sara = user(&panel, "sara", None, GB, 0).await;
        let blocked = hear(&panel, &telegram, text(666, &sara.sub_token)).await;
        assert!(
            blocked[0].1.contains("Too many tries"),
            "a right guess after wrong ones still waits: {}",
            blocked[0].1
        );
        assert!(panel.state.db().telegram_link(666).await.unwrap().is_none());

        let mut settings = panel.state.notify_settings();
        settings.telegram.user_bot = false;
        panel.state.set_notify_settings(settings);
        hear(&panel, &telegram, text(667, &sara.sub_token)).await;
        assert!(panel.state.db().telegram_link(667).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_bot_listens_answers_and_moves_its_place_forward() {
        let polls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<i64>::new()));
        let seen = std::sync::Arc::clone(&polls);
        let telegram = mock(move |request| {
            if request.path.ends_with("/getMe") {
                return (200, r#"{"ok":true,"result":{"username":"zui_bot"}}"#.into());
            }
            if request.path.ends_with("/getUpdates") {
                let offset = request.json()["offset"].as_i64().unwrap_or(0);
                let mut seen = seen.lock().unwrap();
                seen.push(offset);
                let updates = if seen.len() == 1 {
                    r#"[{"update_id":41,"message":{"message_id":1,"chat":{"id":888,"type":"private","first_name":"Z"},"text":"/id"}}]"#
                } else {
                    "[]"
                };
                return (200, format!(r#"{{"ok":true,"result":{updates}}}"#));
            }
            (200, r#"{"ok":true,"result":{}}"#.into())
        })
        .await;
        let panel = world(&telegram).await;
        spawn(panel.state.clone());
        for _ in 0..80 {
            if polls.lock().unwrap().len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let polls = polls.lock().unwrap().clone();
        assert!(polls.len() >= 2, "it kept listening: {polls:?}");
        assert_eq!(
            (polls[0], polls[1]),
            (0, 42),
            "and asked for what came after the last it heard"
        );
        let reply = telegram
            .requests()
            .into_iter()
            .find(|r| r.path.ends_with("/sendMessage"))
            .expect("it answered");
        assert_eq!(reply.json()["chat_id"], 888);
        assert!(reply.json()["text"]
            .as_str()
            .unwrap()
            .contains("<code>888</code>"));
        let bot = panel.state.notifier().bot();
        assert_eq!(bot.username.as_deref(), Some("zui_bot"));
        assert!(bot.running && bot.error.is_none());
    }
}
