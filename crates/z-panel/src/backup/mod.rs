//! Backups: making one, keeping the last few in the data directory, sending
//! it to Telegram, doing it on a schedule, and putting one back.
//!
//! A backup is the panel's whole database (see `z_db::Database::export_json`)
//! gzipped, and sealed with a passphrase if one is set. The audit log and the
//! record of what was already notified are not in it, and neither is the
//! hourly usage unless asked for: they are the bulk, and the panel works
//! without them.

pub mod crypt;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Serialize;

use crate::notify::channels;
use crate::notify::events::{bytes as human, date};
use crate::notify::Event;
use crate::server::AppState;

/// The setting the time of the last backup is kept under.
const LAST_AT: &str = "backup.last_at";
/// How often the schedule looks at the clock.
const TICK: Duration = Duration::from_secs(5 * 60);
/// What a Telegram bot may send as a file.
const TELEGRAM_LIMIT: usize = 49 * 1024 * 1024;
/// How much a backup may inflate to when it is put back.
const INFLATED_LIMIT: u64 = 4 << 30;
/// Tables a backup leaves out always: records of what happened.
const NEVER: [&str; 2] = ["audit_log", "notify_sent"];
/// Tables a restore leaves alone when the backup lacks them.
const KEEP_TABLES: [&str; 2] = NEVER;
/// Settings a restore leaves as they are: the key sessions are signed with,
/// so signing in carries on working, and when this machine last made a
/// backup, so an old file does not make the schedule fire at once.
const KEEP_SETTINGS: [&str; 2] = ["auth.session_secret", LAST_AT];

/// What a backup came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub file: String,
    pub bytes: u64,
    /// Telegram chats it was sent to.
    pub sent: u32,
    pub encrypted: bool,
}

pub fn dir(state: &AppState) -> PathBuf {
    state.config().data_dir.join("backups")
}

/// `20261023-054012`, UTC.
fn stamp(at: i64) -> String {
    let seconds = at.rem_euclid(86_400);
    format!(
        "{}-{:02}{:02}{:02}",
        date(at).replace('-', ""),
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// Whether `name` is a file this module made: nothing else is ever read,
/// written or deleted by name, so a name from outside cannot reach other files.
pub fn valid_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("z-ui-backup-") else {
        return false;
    };
    let rest = rest
        .strip_suffix(".zbk.enc")
        .or_else(|| rest.strip_suffix(".zbk"));
    let Some(rest) = rest else { return false };
    let rest = rest
        .strip_suffix("-before-restore")
        .or_else(|| rest.strip_suffix("-before-import"))
        .unwrap_or(rest);
    let mut parts = rest.split('-');
    let (Some(day), Some(time), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    day.len() == 8
        && time.len() == 6
        && day.bytes().all(|b| b.is_ascii_digit())
        && time.bytes().all(|b| b.is_ascii_digit())
}

/// A backup file in the data directory.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct FileInfo {
    pub name: String,
    pub bytes: u64,
    /// When it was made (Unix seconds).
    pub at: i64,
    pub encrypted: bool,
    /// Made by a restore, of what the panel held before it.
    pub before_restore: bool,
    /// Made by an import from another panel, of what the panel held before it.
    pub before_import: bool,
}

pub fn list(state: &AppState) -> Vec<FileInfo> {
    let mut files: Vec<FileInfo> = std::fs::read_dir(dir(state))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !valid_name(&name) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            let at = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            Some(FileInfo {
                encrypted: name.ends_with(".enc"),
                before_restore: name.contains("-before-restore"),
                before_import: name.contains("-before-import"),
                bytes: meta.len(),
                at,
                name,
            })
        })
        .collect();
    // Names carry the time, newest last when sorted; the other way up is wanted.
    files.sort_by(|a, b| b.name.cmp(&a.name));
    files
}

pub fn read(state: &AppState, name: &str) -> Option<Vec<u8>> {
    valid_name(name)
        .then(|| std::fs::read(dir(state).join(name)).ok())
        .flatten()
}

pub fn delete(state: &AppState, name: &str) -> bool {
    valid_name(name) && std::fs::remove_file(dir(state).join(name)).is_ok()
}

/// The newest `keep` files stay.
fn prune(state: &AppState, keep: usize) {
    for file in list(state).into_iter().skip(keep) {
        let _ = std::fs::remove_file(dir(state).join(file.name));
    }
}

/// The backup itself: gzip of the database as JSON, sealed if `passphrase` is set.
pub async fn make(
    state: &AppState,
    include_usage: bool,
    passphrase: &str,
) -> Result<Vec<u8>, String> {
    let mut skip: Vec<&str> = NEVER.to_vec();
    if !include_usage {
        skip.push("user_usage");
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(6));
    state
        .db()
        .export_json(&mut encoder, z_common::VERSION, &skip)
        .await
        .map_err(|error| error.to_string())?;
    let packed = encoder.finish().map_err(|error| error.to_string())?;
    if passphrase.is_empty() {
        return Ok(packed);
    }
    let passphrase = passphrase.to_string();
    tokio::task::spawn_blocking(move || crypt::encrypt(&passphrase, &packed))
        .await
        .map_err(|error| error.to_string())
}

/// Write `bytes` as `name` in the backups directory, whole or not at all.
fn store(state: &AppState, name: &str, bytes: &[u8]) -> Result<(), String> {
    let dir = dir(state);
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("creating {}: {error}", dir.display()))?;
    let temp = dir.join(format!(".{name}.part"));
    let mut file = std::fs::File::create(&temp).map_err(|error| error.to_string())?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("writing {name}: {error}"))?;
    std::fs::rename(&temp, dir.join(name)).map_err(|error| error.to_string())
}

fn file_name(encrypted: bool, suffix: &str) -> String {
    format!(
        "z-ui-backup-{}{suffix}.zbk{}",
        stamp(z_db::now()),
        if encrypted { ".enc" } else { "" }
    )
}

/// Make a backup, keep it, and send it: to `to_chat` alone when given, else
/// to Telegram as the settings say. The result is told to the admins, except
/// when a chat asked: that chat is answered with the file or the error.
pub async fn run(state: &AppState, to_chat: Option<i64>) -> Result<Done, String> {
    let result = run_now(state, to_chat).await;
    if to_chat.is_none() {
        match &result {
            Ok(done) => state.notify(Event::BackupDone {
                file: done.file.clone(),
                bytes: done.bytes,
                sent: done.sent,
            }),
            Err(error) => state.notify(Event::BackupFailed {
                error: error.clone(),
            }),
        }
    }
    result
}

async fn run_now(state: &AppState, to_chat: Option<i64>) -> Result<Done, String> {
    let settings = state.notify_settings();
    let data = make(
        state,
        settings.backup.include_usage,
        &settings.backup.passphrase,
    )
    .await?;
    let encrypted = !settings.backup.passphrase.is_empty();
    let name = file_name(encrypted, "");
    store(state, &name, &data)?;
    prune(state, settings.backup.keep as usize);
    let _ = state
        .db()
        .set_setting(LAST_AT, &z_db::now().to_string())
        .await;

    let mut sent = 0;
    let chats: Vec<i64> = match to_chat {
        Some(chat) => vec![chat],
        None if settings.backup.send_telegram && settings.telegram_ready() => {
            let mut chats = settings.telegram.chat_ids.clone();
            chats.extend(crate::notify::admin_chats(state, None).await);
            chats.sort_unstable();
            chats.dedup();
            chats
        }
        None => Vec::new(),
    };
    let mut note = None;
    if !chats.is_empty() && !settings.telegram_ready() {
        note = Some("Telegram is not on, so it was only kept in the data directory".to_string());
    } else if data.len() > TELEGRAM_LIMIT {
        note = Some(format!(
            "{} is too big for Telegram's 50 MB; it was kept in the data directory",
            human(data.len() as i64)
        ));
    } else {
        let caption = format!(
            "Z-UI backup · {} · {}",
            stamp(z_db::now()),
            human(data.len() as i64)
        );
        for chat in chats {
            match channels::telegram_file(&settings, chat, &caption, &name, &data).await {
                Ok(()) => sent += 1,
                Err(error) => note = Some(error.message),
            }
        }
    }
    if let (Some(note), 0, Some(_)) = (&note, sent, to_chat) {
        return Err(note.clone());
    }
    if let Some(note) = note {
        tracing::warn!(%note, "a backup was made but not sent everywhere");
    }
    Ok(Done {
        file: name,
        bytes: data.len() as u64,
        sent,
        encrypted,
    })
}

/// Save everything the panel holds as a file of its own, the way back from
/// something that is about to replace or add to it (`suffix` says what).
pub async fn save_before(state: &AppState, suffix: &str) -> Result<String, String> {
    let settings = state.notify_settings();
    let before = make(state, true, &settings.backup.passphrase).await?;
    let name = file_name(!settings.backup.passphrase.is_empty(), suffix);
    store(state, &name, &before)?;
    Ok(name)
}

/// Why a restore did not happen.
#[derive(Debug)]
pub struct Refused {
    pub code: &'static str,
    pub message: String,
}

fn refused(code: &'static str, message: impl Into<String>) -> Refused {
    Refused {
        code,
        message: message.into(),
    }
}

/// What a restore did.
#[derive(Debug)]
pub struct Restored {
    pub report: z_db::ImportReport,
    /// The file made of what the panel held just before, to go back to.
    pub safety: String,
    /// Something that did not go quite right afterwards, such as the core
    /// not starting on the restored inbounds.
    pub warning: Option<String>,
}

/// Put a backup file back: everything the panel keeps is replaced by it.
/// What the panel held before is first saved as a file of its own.
pub async fn restore(
    state: &AppState,
    upload: &[u8],
    passphrase: &str,
) -> Result<Restored, Refused> {
    let packed;
    let data: &[u8] = if crypt::is_encrypted(upload) {
        if passphrase.is_empty() {
            return Err(refused(
                "passphrase_required",
                "this backup is encrypted: give its passphrase",
            ));
        }
        let (secret, bytes) = (passphrase.to_string(), upload.to_vec());
        packed = tokio::task::spawn_blocking(move || crypt::decrypt(&secret, &bytes))
            .await
            .map_err(|error| refused("invalid_backup", error.to_string()))?
            .map_err(|_| {
                refused(
                    "wrong_passphrase",
                    "the passphrase is wrong, or the file is damaged",
                )
            })?;
        &packed
    } else {
        upload
    };
    let gzip = data.starts_with(&[0x1f, 0x8b]);
    if !gzip && !data.starts_with(b"{") {
        return Err(refused("invalid_backup", "this is not a Z-UI backup"));
    }

    let guard = state.core_lock().lock().await;
    // The way back, before anything is replaced; without it nothing is.
    let safety = save_before(state, "-before-restore")
        .await
        .map_err(|error| {
            refused(
                "backup_failed",
                format!("could not save the current state first: {error}"),
            )
        })?;

    let reader: Box<dyn Read + Send + '_> = if gzip {
        Box::new(GzDecoder::new(data).take(INFLATED_LIMIT))
    } else {
        Box::new(data)
    };
    let report = state
        .db()
        .import_json(reader, &KEEP_SETTINGS, &KEEP_TABLES)
        .await
        .map_err(|error| match error {
            z_db::DbError::Backup(message) => refused("invalid_backup", message),
            other => refused("restore_failed", other.to_string()),
        })?;
    // Whoever had a session had it under the admins that were replaced.
    state
        .db()
        .end_all_admin_sessions()
        .await
        .map_err(|error| refused("restore_failed", error.to_string()))?;
    drop(guard);

    // What the panel holds in memory is read again from what it now stores.
    state.set_subscription(crate::subscription::load(state.db()).await);
    state.set_notify_settings(crate::notify::load(state.db()).await);
    if let Ok(Some(level)) = state.db().setting(crate::api::LOG_LEVEL_SETTING).await {
        let _ = state.monitor().logs.set_level(&level);
    }
    let warning = crate::core_config::sync(state)
        .await
        .err()
        .map(|error| error.message);
    Ok(Restored {
        report,
        safety,
        warning,
    })
}

/// Back up on the schedule the settings give.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            let settings = state.notify_settings();
            if !settings.backup.enabled {
                continue;
            }
            let last = state
                .db()
                .setting(LAST_AT)
                .await
                .ok()
                .flatten()
                .and_then(|text| text.parse::<i64>().ok())
                .unwrap_or(0);
            if z_db::now() - last >= i64::from(settings.backup.interval_hours) * 3600 {
                if let Err(error) = run(&state, None).await {
                    tracing::warn!(%error, "the scheduled backup failed");
                    // Not again at once: the next try is an interval from now.
                    let _ = state
                        .db()
                        .set_setting(LAST_AT, &z_db::now().to_string())
                        .await;
                }
            }
        }
    });
}

/// When the last backup was made, if ever.
pub async fn last_at(state: &AppState) -> Option<i64> {
    state
        .db()
        .setting(LAST_AT)
        .await
        .ok()
        .flatten()
        .and_then(|text| text.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::http::testing::ok;
    use crate::notify::Settings;
    use crate::server::testing::{panel, Panel};

    #[test]
    fn only_names_this_module_makes_are_files_it_will_touch() {
        for good in [
            "z-ui-backup-20261023-054012.zbk",
            "z-ui-backup-20261023-054012.zbk.enc",
            "z-ui-backup-20261023-054012-before-restore.zbk",
        ] {
            assert!(valid_name(good), "{good}");
        }
        for bad in [
            "",
            "../z-ui-backup-20261023-054012.zbk",
            "z-ui-backup-20261023-054012.zbk/../../etc/passwd",
            "z-ui-backup-2026-054012.zbk",
            "z-ui-backup-20261023-054012.zip",
            "z-ui-backup-20261023-054012-extra.zbk",
            "z-ui-backup-20261023-05401a.zbk",
            "panel.db",
            ".z-ui-backup-20261023-054012.zbk.part",
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
        assert_eq!(stamp(1_793_000_000), "20261026-073320");
    }

    async fn world() -> Panel {
        let panel = panel("/").await;
        let boss = panel
            .state
            .db()
            .create_admin("boss", "hash", true)
            .await
            .unwrap();
        let groups = panel.state.db().access_groups().await.unwrap();
        for name in ["ali", "sara"] {
            let fields = z_db::UserFields {
                username: name.into(),
                status: "active".into(),
                reset_strategy: "no_reset".into(),
                group_ids: vec![groups[0].id],
                ..Default::default()
            };
            panel
                .state
                .db()
                .create_user(&fields, name, "pw", &format!("{name}-token"), Some(boss.id))
                .await
                .unwrap();
        }
        panel
    }

    #[tokio::test]
    async fn a_backup_is_kept_in_the_data_directory_and_the_oldest_go() {
        let panel = world().await;
        let mut settings = Settings::default();
        settings.backup.keep = 2;
        panel.state.set_notify_settings(settings);
        let first = run(&panel.state, None).await.unwrap();
        assert!(valid_name(&first.file) && !first.encrypted && first.sent == 0);
        assert!(
            read(&panel.state, &first.file)
                .unwrap()
                .starts_with(&[0x1f, 0x8b]),
            "gzip"
        );
        // Files are told apart by the second they were made in.
        for second in 1..=3 {
            let name = format!("z-ui-backup-20300101-00000{second}.zbk");
            store(&panel.state, &name, b"x").unwrap();
        }
        prune(&panel.state, 2);
        let names: Vec<String> = list(&panel.state).into_iter().map(|f| f.name).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(
            names[0].ends_with("000003.zbk") && names[1].ends_with("000002.zbk"),
            "the newest stay: {names:?}"
        );
        assert!(last_at(&panel.state).await.is_some());
        assert!(delete(&panel.state, &names[0]) && !delete(&panel.state, "../../etc/passwd"));
        assert_eq!(read(&panel.state, "../x"), None);
    }

    #[tokio::test]
    async fn a_backup_goes_to_the_telegram_chats_as_a_file() {
        let telegram = ok().await;
        let panel = world().await;
        let mut settings = Settings {
            language: "en".into(),
            ..Default::default()
        };
        settings.telegram.enabled = true;
        settings.telegram.token = "123:abc".into();
        settings.telegram.api_base = telegram.url();
        settings.telegram.chat_ids = vec![-1001];
        settings.backup.send_telegram = true;
        settings.backup.passphrase = "correct horse".into();
        panel.state.set_notify_settings(settings.checked().unwrap());
        panel.state.notifier().retry_after(vec![]);

        let boss = panel
            .state
            .db()
            .admin_by_username("boss")
            .await
            .unwrap()
            .unwrap();
        panel
            .state
            .db()
            .save_telegram_link(77, "admin", boss.id, "Ali")
            .await
            .unwrap();
        let done = run(&panel.state, None).await.unwrap();
        assert!(
            done.encrypted && done.file.ends_with(".zbk.enc") && done.sent == 2,
            "{done:?}"
        );
        panel.state.notifier().settle().await;
        let uploads: Vec<_> = telegram
            .requests()
            .into_iter()
            .filter(|r| r.path.ends_with("/sendDocument"))
            .collect();
        assert_eq!(uploads.len(), 2);
        let body = &uploads[0].body;
        assert!(
            body.windows(5).any(|w| w == b"ZUIE1"),
            "what Telegram gets is sealed"
        );
        assert!(String::from_utf8_lossy(&uploads[0].body)
            .contains(&format!("filename=\"{}\"", done.file)));
        assert!(uploads[0]
            .header("content-type")
            .unwrap()
            .starts_with("multipart/form-data; boundary="));
        // The admins are told it was made; that message is a text, not a file.
        let told: Vec<String> = telegram
            .requests()
            .iter()
            .filter(|r| r.path.ends_with("/sendMessage"))
            .map(|r| r.json()["text"].as_str().unwrap().to_string())
            .collect();
        assert!(
            told.iter().any(|t| t.contains("A backup was made")),
            "{told:?}"
        );
    }

    #[tokio::test]
    async fn a_backup_from_a_chat_goes_to_that_chat_and_a_failure_is_told() {
        let telegram = crate::notify::http::testing::mock(|_| (403, "{}".into())).await;
        let panel = world().await;
        let mut settings = Settings::default();
        settings.telegram.enabled = true;
        settings.telegram.token = "123:abc".into();
        settings.telegram.api_base = telegram.url();
        panel.state.set_notify_settings(settings.checked().unwrap());
        panel.state.notifier().retry_after(vec![]);
        let refused = run(&panel.state, Some(5)).await.unwrap_err();
        assert!(refused.contains("blocked"), "{refused}");
        assert_eq!(list(&panel.state).len(), 1, "it was still kept");
        panel.state.notifier().settle().await;
        let told: Vec<String> = telegram
            .requests()
            .iter()
            .filter(|r| r.path.ends_with("/sendMessage"))
            .map(|r| r.json()["text"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(
            told.is_empty(),
            "the chat that asked is answered, not broadcast to: {told:?}"
        );
    }

    async fn restored_from(source: &Panel, passphrase: &str, include_usage: bool) -> Vec<u8> {
        make(&source.state, include_usage, passphrase)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_restore_replaces_the_panel_and_leaves_a_way_back() {
        let source = world().await;
        let backup = restored_from(&source, "", false).await;

        let target = panel("/").await;
        target
            .state
            .db()
            .create_admin("someone", "h", true)
            .await
            .unwrap();
        target
            .state
            .db()
            .set_setting("auth.session_secret", "LIVE-SECRET")
            .await
            .unwrap();
        target
            .state
            .db()
            .set_setting(LAST_AT, "1234567890")
            .await
            .unwrap();
        target
            .state
            .db()
            .add_audit(&z_db::NewAudit {
                admin_name: "someone".into(),
                action: "user.create".into(),
                via: "session".into(),
                status: 201,
                ..Default::default()
            })
            .await
            .unwrap();
        let result = restore(&target.state, &backup, "").await.unwrap();
        assert!(result
            .report
            .tables
            .iter()
            .any(|(t, n)| t == "users" && *n == 2));
        assert!(target
            .state
            .db()
            .admin_by_username("someone")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            target
                .state
                .db()
                .user_by_token("ali-token")
                .await
                .unwrap()
                .unwrap()
                .username,
            "ali"
        );
        assert_eq!(
            target
                .state
                .db()
                .setting("auth.session_secret")
                .await
                .unwrap()
                .as_deref(),
            Some("LIVE-SECRET"),
            "sessions keep their key"
        );
        assert_eq!(
            last_at(&target.state).await,
            Some(1_234_567_890),
            "an old file does not rewind the schedule"
        );
        assert_eq!(
            target
                .state
                .db()
                .audit(&z_db::AuditQuery {
                    limit: 5,
                    ..Default::default()
                })
                .await
                .unwrap()
                .len(),
            1,
            "the log is not lost"
        );
        let boss = target
            .state
            .db()
            .admin_by_username("boss")
            .await
            .unwrap()
            .unwrap();
        assert!(
            boss.token_version >= 1_000_000,
            "every old session is ended"
        );

        // The way back is a file, and it puts the panel as it was.
        assert!(valid_name(&result.safety) && result.safety.contains("before-restore"));
        let before = read(&target.state, &result.safety).unwrap();
        restore(&target.state, &before, "").await.unwrap();
        assert!(target
            .state
            .db()
            .admin_by_username("someone")
            .await
            .unwrap()
            .is_some());
        assert!(target
            .state
            .db()
            .user_by_token("ali-token")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn an_encrypted_backup_needs_its_passphrase_and_a_wrong_file_changes_nothing() {
        let source = world().await;
        let sealed = restored_from(&source, "correct horse", false).await;
        assert!(crypt::is_encrypted(&sealed));
        let target = panel("/").await;
        target
            .state
            .db()
            .create_admin("someone", "h", true)
            .await
            .unwrap();

        for (what, code, bytes, pass) in [
            ("no passphrase", "passphrase_required", sealed.clone(), ""),
            (
                "wrong passphrase",
                "wrong_passphrase",
                sealed.clone(),
                "wrong horse",
            ),
            (
                "not a backup",
                "invalid_backup",
                b"hello world".to_vec(),
                "",
            ),
            (
                "a plain gzip of something else",
                "invalid_backup",
                gzip(b"{\"format\":\"other\"}"),
                "",
            ),
        ] {
            let error = restore(&target.state, &bytes, pass).await.unwrap_err();
            assert_eq!(error.code, code, "{what}: {}", error.message);
            assert!(
                target
                    .state
                    .db()
                    .admin_by_username("someone")
                    .await
                    .unwrap()
                    .is_some(),
                "{what}: untouched"
            );
        }
        restore(&target.state, &sealed, "correct horse")
            .await
            .unwrap();
        assert!(target
            .state
            .db()
            .admin_by_username("boss")
            .await
            .unwrap()
            .is_some());
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    #[tokio::test]
    async fn the_restored_settings_are_in_force_at_once() {
        let source = world().await;
        let sub = crate::subscription::Settings {
            title: "Restored title".into(),
            ..Default::default()
        };
        source
            .state
            .db()
            .set_setting(
                crate::subscription::SETTING,
                &serde_json::to_string(&sub).unwrap(),
            )
            .await
            .unwrap();
        let notify = Settings {
            language: "en".into(),
            ..Default::default()
        };
        crate::notify::save(source.state.db(), &notify)
            .await
            .unwrap();
        let backup = restored_from(&source, "", false).await;

        let target = panel("/").await;
        assert_ne!(target.state.subscription().title, "Restored title");
        restore(&target.state, &backup, "").await.unwrap();
        assert_eq!(target.state.subscription().title, "Restored title");
        assert_eq!(target.state.notify_settings().language, "en");
    }
}
