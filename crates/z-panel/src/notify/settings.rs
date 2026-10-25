//! What an admin decides about notifications, the bot and backups. Kept as one
//! JSON setting; the secrets in it (the bot token, the webhook secret, the
//! Discord URL, the backup passphrase) are never sent back to the dashboard:
//! it learns only whether each is set, and an empty one sent in is "keep".

use serde::{Deserialize, Serialize};

use super::http::Proxy;

/// The setting these are kept under.
pub const SETTING: &str = "notify.settings";

pub const TELEGRAM_API: &str = "https://api.telegram.org";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct Settings {
    /// `fa` or `en`: the language messages are written in.
    pub language: String,
    /// `http://host:port` or `socks5://host:port` (with `user:pass@` if it
    /// asks) that Telegram, Discord and webhooks are reached through; empty
    /// for none.
    pub proxy: String,
    pub telegram: Telegram,
    pub discord: Discord,
    pub webhook: Webhook,
    pub events: Events,
    pub backup: Backup,
    pub relief: Relief,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct Telegram {
    pub enabled: bool,
    /// The bot's token from @BotFather.
    pub token: String,
    /// Where the Bot API is: Telegram's own address, or a relay of it.
    pub api_base: String,
    /// Chats, groups and channels that hear everything (a channel's id starts
    /// with -100).
    pub chat_ids: Vec<i64>,
    /// Users may link their chat to see their own usage.
    pub user_bot: bool,
    /// Tell users in the bot when their own data or time runs out.
    pub notify_users: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct Discord {
    pub enabled: bool,
    /// The channel's incoming-webhook URL.
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct Webhook {
    pub enabled: bool,
    pub url: String,
    /// Signs each body (HMAC-SHA256, in `X-Z-UI-Signature`), so the receiver
    /// can tell it came from here.
    pub secret: String,
}

/// Which things are told, and when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct Events {
    pub data_low: bool,
    /// Tell when this much of the data limit is used.
    pub data_low_percent: u8,
    pub expiring: bool,
    /// Tell when this many days are left.
    pub expiring_days: u32,
    pub limited: bool,
    pub expired: bool,
    /// The core stopping, and starting again.
    pub core: bool,
    /// Wrong passwords, and an address being locked out.
    pub login_failed: bool,
    /// An admin signing in from an address not seen before.
    pub login_new: bool,
    pub backup: bool,
    /// A certificate renewed, one that could not be, one about to expire.
    pub certificate: bool,
    /// A newer version of the panel is published.
    pub update: bool,
    /// The server's processor over `cpu_percent` for `cpu_minutes` in a row.
    pub server_cpu: bool,
    pub cpu_percent: u8,
    pub cpu_minutes: u32,
    /// Its memory, and its disk, over a percent.
    pub server_memory: bool,
    pub memory_percent: u8,
    pub server_disk: bool,
    pub disk_percent: u8,
    /// Its hottest sensor over a number of degrees (where it has one).
    pub server_temperature: bool,
    pub temperature_celsius: u8,
    /// Each time the traffic the panel has carried grows by `traffic_step_gb`.
    pub traffic: bool,
    pub traffic_step_gb: u32,
    /// The machine starting again.
    pub reboot: bool,
}

/// What the panel does by itself to lighten the load when the server's
/// processor or temperature stays over the limits of [`Events`]: the idle
/// users leave for a few minutes, and the busiest ones are slowed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct Relief {
    pub enabled: bool,
    /// Users connected but with no traffic for `idle_minutes` leave the core
    /// for `pause_minutes`, then come back by themselves.
    pub pause_idle: bool,
    pub idle_minutes: u32,
    pub pause_minutes: u32,
    /// The `busiest_count` users that carried the most are held to
    /// `throttle_mbit` Mbit/s until the load is down (or `max_minutes`).
    pub throttle_busiest: bool,
    pub busiest_count: u32,
    pub throttle_mbit: u32,
    pub max_minutes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct Backup {
    pub enabled: bool,
    pub interval_hours: u32,
    /// How many to keep in the panel's data directory.
    pub keep: u32,
    /// Send each to the Telegram chats of the sudo admins and the listed ones.
    pub send_telegram: bool,
    /// Include the hourly usage (large).
    pub include_usage: bool,
    /// Encrypts each backup; empty for none.
    pub passphrase: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "fa".into(),
            proxy: String::new(),
            telegram: Telegram::default(),
            discord: Discord::default(),
            webhook: Webhook::default(),
            events: Events::default(),
            backup: Backup::default(),
            relief: Relief::default(),
        }
    }
}

impl Default for Telegram {
    fn default() -> Self {
        Self {
            enabled: false,
            token: String::new(),
            api_base: TELEGRAM_API.into(),
            chat_ids: Vec::new(),
            user_bot: false,
            notify_users: true,
        }
    }
}

impl Default for Events {
    fn default() -> Self {
        Self {
            data_low: true,
            data_low_percent: 85,
            expiring: true,
            expiring_days: 3,
            limited: true,
            expired: true,
            core: true,
            login_failed: true,
            login_new: true,
            backup: true,
            certificate: true,
            update: true,
            server_cpu: true,
            cpu_percent: 90,
            cpu_minutes: 3,
            server_memory: true,
            memory_percent: 90,
            server_disk: true,
            disk_percent: 90,
            server_temperature: true,
            temperature_celsius: 85,
            traffic: true,
            traffic_step_gb: 1024,
            reboot: true,
        }
    }
}

impl Default for Relief {
    fn default() -> Self {
        Self {
            enabled: false,
            pause_idle: true,
            idle_minutes: 5,
            pause_minutes: 5,
            throttle_busiest: true,
            busiest_count: 10,
            throttle_mbit: 5,
            max_minutes: 30,
        }
    }
}

impl Default for Backup {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_hours: 24,
            keep: 7,
            send_telegram: true,
            include_usage: false,
            passphrase: String::new(),
        }
    }
}

impl Settings {
    /// The proxy, already read. Checked when the settings were saved.
    pub fn proxy(&self) -> Option<Proxy> {
        Proxy::parse(&self.proxy).ok().flatten()
    }

    /// The Telegram Bot API address without a trailing slash.
    pub fn telegram_api(&self) -> &str {
        self.telegram.api_base.trim_end_matches('/')
    }

    /// Whether the bot can be spoken to at all.
    pub fn telegram_ready(&self) -> bool {
        self.telegram.enabled && !self.telegram.token.is_empty()
    }

    /// These settings, with every secret that `incoming` left empty kept as it
    /// is here, and the whole checked. What a save is made of.
    pub fn accept(&self, mut incoming: Settings) -> Result<Settings, String> {
        let keep = |new: &mut String, old: &str| {
            if new.trim().is_empty() {
                *new = old.to_string();
            } else {
                *new = new.trim().to_string();
            }
        };
        // The proxy came back as it was shown, its password hidden: keep the stored one.
        if incoming.proxy.trim() == redact_proxy(&self.proxy) {
            incoming.proxy = self.proxy.clone();
        } else if is_redacted(&incoming.proxy) {
            return Err("type the proxy's password again".into());
        }
        keep(&mut incoming.telegram.token, &self.telegram.token);
        keep(&mut incoming.discord.url, &self.discord.url);
        keep(&mut incoming.webhook.secret, &self.webhook.secret);
        keep(&mut incoming.backup.passphrase, &self.backup.passphrase);
        incoming.checked()
    }

    /// These settings if they make sense, with what can be tidied tidied.
    pub fn checked(mut self) -> Result<Settings, String> {
        if !matches!(self.language.as_str(), "fa" | "en") {
            return Err("the language is fa or en".into());
        }
        self.proxy = self.proxy.trim().to_string();
        Proxy::parse(&self.proxy)?;

        let base = self
            .telegram
            .api_base
            .trim()
            .trim_end_matches('/')
            .to_string();
        let api = if base.is_empty() {
            TELEGRAM_API.to_string()
        } else {
            base
        };
        if !(api.starts_with("https://") || api.starts_with("http://")) {
            return Err("the Telegram API address starts with https:// or http://".into());
        }
        self.telegram.api_base = api;
        self.telegram.chat_ids.sort_unstable();
        self.telegram.chat_ids.dedup();
        if self.telegram.chat_ids.contains(&0) {
            return Err("0 is not a chat id".into());
        }
        if self.telegram.enabled && self.telegram.token.is_empty() {
            return Err("the bot needs its token".into());
        }
        if self.telegram.token.chars().any(char::is_whitespace) {
            return Err("a bot token has no spaces".into());
        }

        if self.discord.enabled {
            let url = url::Url::parse(&self.discord.url)
                .map_err(|_| "Discord's webhook address is not a URL")?;
            let host = url.host_str().unwrap_or_default();
            let discord = host == "discord.com"
                || host.ends_with(".discord.com")
                || host == "discordapp.com"
                || host.ends_with(".discordapp.com");
            if url.scheme() != "https" || !discord || !url.path().starts_with("/api/webhooks/") {
                return Err(
                    "that is not a Discord webhook (https://discord.com/api/webhooks/…)".into(),
                );
            }
        }
        if self.webhook.enabled {
            let url = url::Url::parse(&self.webhook.url)
                .map_err(|_| "the webhook address is not a URL")?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err("the webhook address starts with https:// or http://".into());
            }
        }

        if !(1..=99).contains(&self.events.data_low_percent) {
            return Err("the data warning is a percent from 1 to 99".into());
        }
        if !(1..=60).contains(&self.events.expiring_days) {
            return Err("the expiry warning is 1 to 60 days".into());
        }
        let e = &self.events;
        if !(50..=100).contains(&e.cpu_percent)
            || !(50..=100).contains(&e.memory_percent)
            || !(50..=100).contains(&e.disk_percent)
        {
            return Err("a server warning is a percent from 50 to 100".into());
        }
        if !(1..=60).contains(&e.cpu_minutes) {
            return Err("the processor must stay over its limit for 1 to 60 minutes".into());
        }
        if !(50..=120).contains(&e.temperature_celsius) {
            return Err("the temperature warning is 50 to 120 degrees".into());
        }
        if !(1..=1_000_000).contains(&e.traffic_step_gb) {
            return Err("tell the traffic every 1 to 1000000 GB".into());
        }
        let r = &self.relief;
        if !(1..=240).contains(&r.idle_minutes) || !(1..=240).contains(&r.pause_minutes) {
            return Err("idle and pause times are 1 to 240 minutes".into());
        }
        if !(1..=1000).contains(&r.busiest_count) || !(1..=1000).contains(&r.throttle_mbit) {
            return Err("the busiest users are 1 to 1000, and the speed 1 to 1000 Mbit/s".into());
        }
        if !(1..=720).contains(&r.max_minutes) {
            return Err("slowing lasts 1 to 720 minutes at most".into());
        }
        if !(1..=720).contains(&self.backup.interval_hours) {
            return Err("a backup every 1 to 720 hours".into());
        }
        if !(1..=100).contains(&self.backup.keep) {
            return Err("keep 1 to 100 backups".into());
        }
        if !self.backup.passphrase.is_empty() && self.backup.passphrase.chars().count() < 8 {
            return Err("a backup passphrase is at least 8 characters".into());
        }
        Ok(self)
    }
}

/// What the dashboard is told: no secret, only whether each is set.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct View {
    pub language: String,
    pub proxy: String,
    pub telegram: TelegramView,
    pub discord: DiscordView,
    pub webhook: WebhookView,
    pub events: Events,
    pub backup: BackupView,
    pub relief: Relief,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct TelegramView {
    pub enabled: bool,
    pub token_set: bool,
    pub api_base: String,
    pub chat_ids: Vec<i64>,
    pub user_bot: bool,
    pub notify_users: bool,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DiscordView {
    pub enabled: bool,
    pub url_set: bool,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct WebhookView {
    pub enabled: bool,
    pub url: String,
    pub secret_set: bool,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct BackupView {
    pub enabled: bool,
    pub interval_hours: u32,
    pub keep: u32,
    pub send_telegram: bool,
    pub include_usage: bool,
    pub passphrase_set: bool,
}

impl Settings {
    pub fn view(&self) -> View {
        View {
            language: self.language.clone(),
            proxy: redact_proxy(&self.proxy),
            telegram: TelegramView {
                enabled: self.telegram.enabled,
                token_set: !self.telegram.token.is_empty(),
                api_base: self.telegram.api_base.clone(),
                chat_ids: self.telegram.chat_ids.clone(),
                user_bot: self.telegram.user_bot,
                notify_users: self.telegram.notify_users,
            },
            discord: DiscordView {
                enabled: self.discord.enabled,
                url_set: !self.discord.url.is_empty(),
            },
            webhook: WebhookView {
                enabled: self.webhook.enabled,
                url: self.webhook.url.clone(),
                secret_set: !self.webhook.secret.is_empty(),
            },
            events: self.events.clone(),
            backup: BackupView {
                enabled: self.backup.enabled,
                interval_hours: self.backup.interval_hours,
                keep: self.backup.keep,
                send_telegram: self.backup.send_telegram,
                include_usage: self.backup.include_usage,
                passphrase_set: !self.backup.passphrase.is_empty(),
            },
            relief: self.relief.clone(),
        }
    }
}

/// The proxy address without its password: the dashboard shows where it goes,
/// not how to get in. Saving the redacted form back keeps the stored one.
fn redact_proxy(proxy: &str) -> String {
    match url::Url::parse(proxy) {
        Ok(mut url) if url.password().is_some() => {
            let _ = url.set_password(Some("•••"));
            url.to_string()
                .replace("%E2%80%A2%E2%80%A2%E2%80%A2", "•••")
        }
        _ => proxy.to_string(),
    }
}

/// Whether `proxy` is what [`redact_proxy`] made of the stored one.
pub fn is_redacted(proxy: &str) -> bool {
    proxy.contains(":•••@")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(change: impl FnOnce(&mut Settings)) -> Settings {
        let mut settings = Settings::default();
        change(&mut settings);
        settings
    }

    #[test]
    fn the_defaults_are_sound_and_tell_everything() {
        let settings = Settings::default().checked().unwrap();
        assert_eq!(settings.language, "fa");
        assert!(
            !settings.telegram.enabled && !settings.discord.enabled && !settings.webhook.enabled
        );
        assert_eq!(settings.telegram_api(), "https://api.telegram.org");
        assert!(settings.events.data_low && settings.events.core && settings.events.login_new);
        assert!(settings.proxy().is_none());
        // An old setting with fewer keys still reads.
        let old: Settings =
            serde_json::from_str(r#"{"language":"en","telegram":{"enabled":false}}"#).unwrap();
        assert_eq!(old.events.data_low_percent, 85);
        assert_eq!(old.telegram.api_base, TELEGRAM_API);
    }

    #[test]
    fn what_makes_no_sense_is_refused() {
        let bad = [
            with(|s| s.language = "de".into()),
            with(|s| s.proxy = "ftp://x:1".into()),
            with(|s| s.telegram.api_base = "t.me".into()),
            with(|s| s.telegram.enabled = true),
            with(|s| s.telegram.token = "12 34".into()),
            with(|s| s.telegram.chat_ids = vec![0]),
            with(|s| {
                s.discord.enabled = true;
                s.discord.url = "https://evil.example/api/webhooks/1/x".into();
            }),
            with(|s| {
                s.discord.enabled = true;
                s.discord.url = "http://discord.com/api/webhooks/1/x".into();
            }),
            with(|s| {
                s.webhook.enabled = true;
                s.webhook.url = "ftp://hook".into();
            }),
            with(|s| s.events.data_low_percent = 100),
            with(|s| s.events.expiring_days = 0),
            with(|s| s.backup.interval_hours = 0),
            with(|s| s.backup.keep = 101),
            with(|s| s.backup.passphrase = "short".into()),
        ];
        for (n, settings) in bad.into_iter().enumerate() {
            assert!(settings.checked().is_err(), "case {n}");
        }
        let good = with(|s| {
            s.telegram.enabled = true;
            s.telegram.token = "123:abc".into();
            s.telegram.chat_ids = vec![-1001, 5, 5];
            s.telegram.api_base = " https://relay.example/ ".into();
            s.discord.enabled = true;
            s.discord.url = "https://discord.com/api/webhooks/1/x".into();
            s.webhook.enabled = true;
            s.webhook.url = "https://hooks.example/z".into();
            s.proxy = " socks5://127.0.0.1:10808 ".into();
        })
        .checked()
        .unwrap();
        assert_eq!(good.telegram.chat_ids, [-1001, 5]);
        assert_eq!(good.telegram_api(), "https://relay.example");
        assert_eq!(good.proxy, "socks5://127.0.0.1:10808");
    }

    #[test]
    fn a_secret_left_empty_is_kept_and_none_is_shown() {
        let stored = with(|s| {
            s.telegram.token = "123:abc".into();
            s.discord.url = "https://discord.com/api/webhooks/1/x".into();
            s.webhook.secret = "s3cret".into();
            s.backup.passphrase = "correct horse".into();
        });
        let view = serde_json::to_string(&stored.view()).unwrap();
        for secret in ["123:abc", "webhooks/1/x", "s3cret", "correct horse"] {
            assert!(!view.contains(secret), "{secret} leaked: {view}");
        }
        let v = stored.view();
        assert!(
            v.telegram.token_set
                && v.discord.url_set
                && v.webhook.secret_set
                && v.backup.passphrase_set
        );

        // The dashboard sends back what it was shown, with a change.
        let incoming = Settings {
            language: "en".into(),
            ..Default::default()
        };
        let kept = stored.accept(incoming).unwrap();
        assert_eq!(kept.language, "en");
        assert_eq!(kept.telegram.token, "123:abc");
        assert_eq!(kept.discord.url, stored.discord.url);
        assert_eq!(kept.webhook.secret, "s3cret");
        assert_eq!(kept.backup.passphrase, "correct horse");
        let mut changed = Settings::default();
        changed.telegram.token = " 999:new ".into();
        assert_eq!(stored.accept(changed).unwrap().telegram.token, "999:new");
    }

    #[test]
    fn a_proxy_password_is_not_shown() {
        let stored = with(|s| s.proxy = "socks5://bot:hunter2@10.0.0.1:1080".into());
        let shown = stored.view().proxy;
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("bot") && shown.contains("10.0.0.1"));
        assert!(is_redacted(&shown));
        assert!(!is_redacted("socks5://10.0.0.1:1080"));

        // Saved back as shown, the stored proxy stays; a half-edited one is refused.
        let same = Settings {
            proxy: shown,
            ..Default::default()
        };
        assert_eq!(
            stored.accept(same).unwrap().proxy,
            "socks5://bot:hunter2@10.0.0.1:1080"
        );
        let moved = Settings {
            proxy: "socks5://bot:•••@10.9.9.9:1080".into(),
            ..Default::default()
        };
        assert!(stored.accept(moved).is_err());
        let cleared = Settings {
            proxy: String::new(),
            ..Default::default()
        };
        assert_eq!(stored.accept(cleared).unwrap().proxy, "");
    }
}
