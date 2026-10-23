//! The things worth telling, and how each reads in Persian and in English.

use serde_json::{json, Value};

use super::settings::Events;

/// Something that happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A message to see that a channel works.
    Test,
    UserDataLow {
        user: String,
        user_id: i64,
        owner: Option<i64>,
        used: i64,
        limit: i64,
        percent: u32,
    },
    UserExpiring {
        user: String,
        user_id: i64,
        owner: Option<i64>,
        days: i64,
        expire_at: i64,
    },
    UserLimited {
        user: String,
        user_id: i64,
        owner: Option<i64>,
        limit: i64,
    },
    UserExpired {
        user: String,
        user_id: i64,
        owner: Option<i64>,
        expire_at: i64,
    },
    CoreDown,
    CoreUp,
    LoginFailed {
        admin: String,
        admin_id: i64,
        address: String,
        attempts: u32,
    },
    LoginLocked {
        address: String,
        minutes: u64,
    },
    NewLogin {
        admin: String,
        admin_id: i64,
        address: String,
    },
    BackupDone {
        file: String,
        bytes: u64,
        sent: u32,
    },
    BackupFailed {
        error: String,
    },
}

/// A message, before it is dressed for a channel: a title and some
/// `label: value` lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub title: String,
    pub fields: Vec<(String, String)>,
}

/// Who an event is for, beyond the sudo admins and the listed chats.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Audience {
    /// An admin whose business it is (a user's owner, the one signing in).
    pub admin: Option<i64>,
    /// A user it is about, who may be told in the bot.
    pub user: Option<i64>,
}

impl Event {
    /// A short stable name, in the webhook and the logs.
    pub fn kind(&self) -> &'static str {
        match self {
            Event::Test => "test",
            Event::UserDataLow { .. } => "user.data_low",
            Event::UserExpiring { .. } => "user.expiring",
            Event::UserLimited { .. } => "user.limited",
            Event::UserExpired { .. } => "user.expired",
            Event::CoreDown => "core.down",
            Event::CoreUp => "core.up",
            Event::LoginFailed { .. } => "login.failed",
            Event::LoginLocked { .. } => "login.locked",
            Event::NewLogin { .. } => "login.new",
            Event::BackupDone { .. } => "backup.done",
            Event::BackupFailed { .. } => "backup.failed",
        }
    }

    /// Whether the settings want it told.
    pub fn wanted(&self, events: &Events) -> bool {
        match self {
            Event::Test => true,
            Event::UserDataLow { .. } => events.data_low,
            Event::UserExpiring { .. } => events.expiring,
            Event::UserLimited { .. } => events.limited,
            Event::UserExpired { .. } => events.expired,
            Event::CoreDown | Event::CoreUp => events.core,
            Event::LoginFailed { .. } | Event::LoginLocked { .. } => events.login_failed,
            Event::NewLogin { .. } => events.login_new,
            Event::BackupDone { .. } | Event::BackupFailed { .. } => events.backup,
        }
    }

    pub fn audience(&self) -> Audience {
        match self {
            Event::UserDataLow { user_id, owner, .. }
            | Event::UserExpiring { user_id, owner, .. }
            | Event::UserLimited { user_id, owner, .. }
            | Event::UserExpired { user_id, owner, .. } => Audience {
                admin: *owner,
                user: Some(*user_id),
            },
            Event::LoginFailed { admin_id, .. } | Event::NewLogin { admin_id, .. } => Audience {
                admin: Some(*admin_id),
                user: None,
            },
            _ => Audience::default(),
        }
    }

    /// The facts, for a webhook to read.
    pub fn data(&self) -> Value {
        match self {
            Event::Test | Event::CoreDown | Event::CoreUp => json!({}),
            Event::UserDataLow {
                user,
                user_id,
                used,
                limit,
                percent,
                ..
            } => {
                json!({"user": user, "user_id": user_id, "used": used, "limit": limit, "percent": percent})
            }
            Event::UserExpiring {
                user,
                user_id,
                days,
                expire_at,
                ..
            } => {
                json!({"user": user, "user_id": user_id, "days": days, "expire_at": expire_at})
            }
            Event::UserLimited {
                user,
                user_id,
                limit,
                ..
            } => {
                json!({"user": user, "user_id": user_id, "limit": limit})
            }
            Event::UserExpired {
                user,
                user_id,
                expire_at,
                ..
            } => {
                json!({"user": user, "user_id": user_id, "expire_at": expire_at})
            }
            Event::LoginFailed {
                admin,
                address,
                attempts,
                ..
            } => {
                json!({"admin": admin, "address": address, "attempts": attempts})
            }
            Event::LoginLocked { address, minutes } => {
                json!({"address": address, "minutes": minutes})
            }
            Event::NewLogin { admin, address, .. } => json!({"admin": admin, "address": address}),
            Event::BackupDone { file, bytes, sent } => {
                json!({"file": file, "bytes": bytes, "sent": sent})
            }
            Event::BackupFailed { error } => json!({"error": error}),
        }
    }

    /// The message for the admins, in `language` (`fa` or anything else for
    /// English).
    pub fn message(&self, language: &str) -> Message {
        let fa = language == "fa";
        let pick = |fa_text: &str, en_text: &str| if fa { fa_text } else { en_text }.to_string();
        let field =
            |fa_label: &str, en_label: &str, value: String| (pick(fa_label, en_label), value);
        match self {
            Event::Test => Message {
                title: pick("پیام آزمایشی از Z-UI ✅", "Test message from Z-UI ✅"),
                fields: vec![field(
                    "وضعیت",
                    "Status",
                    pick("این کانال درست کار می‌کند", "this channel works"),
                )],
            },
            Event::UserDataLow {
                user,
                used,
                limit,
                percent,
                ..
            } => Message {
                title: pick(
                    "⚠️ حجم یک کاربر رو به پایان است",
                    "⚠️ A user is running low on data",
                ),
                fields: vec![
                    field("کاربر", "User", user.clone()),
                    field(
                        "مصرف",
                        "Used",
                        format!("{} / {} ({percent}%)", bytes(*used), bytes(*limit)),
                    ),
                ],
            },
            Event::UserExpiring {
                user,
                days,
                expire_at,
                ..
            } => Message {
                title: pick(
                    "⏳ اشتراک یک کاربر رو به پایان است",
                    "⏳ A user's time is running out",
                ),
                fields: vec![
                    field("کاربر", "User", user.clone()),
                    field("روز مانده", "Days left", days.to_string()),
                    field("پایان", "Ends", date(*expire_at)),
                ],
            },
            Event::UserLimited { user, limit, .. } => Message {
                title: pick("⛔ حجم یک کاربر تمام شد", "⛔ A user is out of data"),
                fields: vec![
                    field("کاربر", "User", user.clone()),
                    field("حجم", "Limit", bytes(*limit)),
                ],
            },
            Event::UserExpired {
                user, expire_at, ..
            } => Message {
                title: pick("⌛ اشتراک یک کاربر تمام شد", "⌛ A user's time has ended"),
                fields: vec![
                    field("کاربر", "User", user.clone()),
                    field("پایان", "Ended", date(*expire_at)),
                ],
            },
            Event::CoreDown => Message {
                title: pick("🔴 هسته از کار افتاد", "🔴 The core has stopped"),
                fields: vec![field(
                    "نکته",
                    "Note",
                    pick(
                        "ورودی فعال هست ولی هسته روشن نیست؛ لاگ پنل را ببینید",
                        "an inbound is enabled but the core is not running; see the panel's log",
                    ),
                )],
            },
            Event::CoreUp => Message {
                title: pick("🟢 هسته دوباره روشن شد", "🟢 The core is running again"),
                fields: vec![],
            },
            Event::LoginFailed {
                admin,
                address,
                attempts,
                ..
            } => Message {
                title: pick(
                    "🚨 ورودهای ناموفق به پنل",
                    "🚨 Failed sign-ins to the panel",
                ),
                fields: vec![
                    field("حساب", "Account", admin.clone()),
                    field("آدرس", "Address", address.clone()),
                    field("تعداد", "Attempts", attempts.to_string()),
                ],
            },
            Event::LoginLocked { address, minutes } => Message {
                title: pick("🔒 یک آدرس قفل شد", "🔒 An address was locked out"),
                fields: vec![
                    field("آدرس", "Address", address.clone()),
                    field(
                        "مدت",
                        "For",
                        pick(&format!("{minutes} دقیقه"), &format!("{minutes} minutes")),
                    ),
                ],
            },
            Event::NewLogin { admin, address, .. } => Message {
                title: pick("🔑 ورود از آدرس تازه", "🔑 Sign-in from a new address"),
                fields: vec![
                    field("حساب", "Account", admin.clone()),
                    field("آدرس", "Address", address.clone()),
                ],
            },
            Event::BackupDone {
                file,
                bytes: size,
                sent,
            } => Message {
                title: pick("💾 بکاپ ساخته شد", "💾 A backup was made"),
                fields: vec![
                    field("فایل", "File", file.clone()),
                    field("حجم", "Size", bytes(*size as i64)),
                    field("ارسال‌شده به تلگرام", "Sent to Telegram", sent.to_string()),
                ],
            },
            Event::BackupFailed { error } => Message {
                title: pick("❌ ساخت بکاپ ناموفق بود", "❌ A backup failed"),
                fields: vec![field("خطا", "Error", error.clone())],
            },
        }
    }

    /// The message for the user it is about, when there is one: in the first
    /// person, with no one else's name in it.
    pub fn message_for_user(&self, language: &str) -> Option<Message> {
        let fa = language == "fa";
        let pick = |fa_text: &str, en_text: &str| if fa { fa_text } else { en_text }.to_string();
        let field =
            |fa_label: &str, en_label: &str, value: String| (pick(fa_label, en_label), value);
        Some(match self {
            Event::UserDataLow {
                used,
                limit,
                percent,
                ..
            } => Message {
                title: pick(
                    "⚠️ حجم اشتراک شما رو به پایان است",
                    "⚠️ Your data is running low",
                ),
                fields: vec![field(
                    "مصرف",
                    "Used",
                    format!("{} / {} ({percent}%)", bytes(*used), bytes(*limit)),
                )],
            },
            Event::UserExpiring {
                days, expire_at, ..
            } => Message {
                title: pick(
                    "⏳ اشتراک شما رو به پایان است",
                    "⏳ Your subscription is ending soon",
                ),
                fields: vec![
                    field("روز مانده", "Days left", days.to_string()),
                    field("پایان", "Ends", date(*expire_at)),
                ],
            },
            Event::UserLimited { limit, .. } => Message {
                title: pick("⛔ حجم اشتراک شما تمام شد", "⛔ You are out of data"),
                fields: vec![field("حجم", "Limit", bytes(*limit))],
            },
            Event::UserExpired { expire_at, .. } => Message {
                title: pick("⌛ اشتراک شما تمام شد", "⌛ Your subscription has ended"),
                fields: vec![field("پایان", "Ended", date(*expire_at))],
            },
            _ => return None,
        })
    }
}

/// Bytes as people write them: `1.5 GB`.
pub fn bytes(value: i64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut size = value.max(0) as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < units.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 || size >= 100.0 {
        format!("{size:.0} {}", units[unit])
    } else {
        format!("{size:.1} {}", units[unit])
    }
}

/// A Unix time as `2026-10-23` (UTC): enough to say when, with no time zone
/// to get wrong.
pub fn date(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn low() -> Event {
        Event::UserDataLow {
            user: "ali".into(),
            user_id: 3,
            owner: Some(2),
            used: 900 << 20,
            limit: 1 << 30,
            percent: 88,
        }
    }

    #[test]
    fn numbers_and_dates_read_plainly() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(5 << 30), "5.0 GB");
        assert_eq!(bytes(250 << 30), "250 GB");
        assert_eq!(bytes(-5), "0 B");
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_793_000_000), "2026-10-26");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(-86_400), "1969-12-31");
    }

    #[test]
    fn every_event_has_a_message_in_each_language_and_a_stable_name() {
        let events = [
            Event::Test,
            low(),
            Event::UserExpiring {
                user: "ali".into(),
                user_id: 3,
                owner: None,
                days: 2,
                expire_at: 1_793_000_000,
            },
            Event::UserLimited {
                user: "ali".into(),
                user_id: 3,
                owner: None,
                limit: 1 << 30,
            },
            Event::UserExpired {
                user: "ali".into(),
                user_id: 3,
                owner: None,
                expire_at: 1_793_000_000,
            },
            Event::CoreDown,
            Event::CoreUp,
            Event::LoginFailed {
                admin: "root".into(),
                admin_id: 1,
                address: "203.0.113.9".into(),
                attempts: 3,
            },
            Event::LoginLocked {
                address: "203.0.113.9".into(),
                minutes: 15,
            },
            Event::NewLogin {
                admin: "root".into(),
                admin_id: 1,
                address: "203.0.113.9".into(),
            },
            Event::BackupDone {
                file: "z-ui.zbk".into(),
                bytes: 2048,
                sent: 1,
            },
            Event::BackupFailed {
                error: "disk full".into(),
            },
        ];
        let mut kinds: Vec<&str> = events.iter().map(Event::kind).collect();
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(kinds.len(), events.len(), "names are unique");
        for event in &events {
            let (fa, en) = (event.message("fa"), event.message("en"));
            assert!(!fa.title.is_empty() && !en.title.is_empty());
            assert_ne!(fa.title, en.title, "{}", event.kind());
            assert!(event.data().is_object());
        }
        assert!(low().message("en").fields[1].1.contains("88%"));
    }

    #[test]
    fn a_user_is_told_in_the_first_person_and_only_of_their_own() {
        let mine = low().message_for_user("en").unwrap();
        assert!(mine.title.contains("Your data"));
        assert!(
            mine.fields.iter().all(|(_, value)| !value.contains("ali")),
            "no name needed: {mine:?}"
        );
        assert!(low().message_for_user("fa").unwrap().title.contains("شما"));
        assert!(Event::CoreDown.message_for_user("en").is_none());
        assert!(Event::Test.message_for_user("en").is_none());
    }

    #[test]
    fn settings_decide_which_events_are_told_and_who_hears_them() {
        let mut events = Events::default();
        assert!(low().wanted(&events) && Event::CoreDown.wanted(&events));
        events.data_low = false;
        events.core = false;
        assert!(!low().wanted(&events) && !Event::CoreUp.wanted(&events));
        assert!(Event::Test.wanted(&events), "a test is always told");
        assert_eq!(
            low().audience(),
            Audience {
                admin: Some(2),
                user: Some(3)
            }
        );
        let login = Event::NewLogin {
            admin: "root".into(),
            admin_id: 1,
            address: "x".into(),
        };
        assert_eq!(
            login.audience(),
            Audience {
                admin: Some(1),
                user: None
            }
        );
        assert_eq!(Event::CoreDown.audience(), Audience::default());
    }
}
