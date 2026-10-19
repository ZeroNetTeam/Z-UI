//! The log the dashboard shows, live.
//!
//! The core runs inside this process, so the panel's lines and the core's
//! come through one `tracing` stream. A layer keeps the latest records in a
//! ring (so a page opened now still sees what just happened) and sends each
//! new one to whoever is watching. The target says whose it is: `zero_*` and
//! `zray` are the core's, `z_*` the panel's.
//!
//! The ring is memory only and bounded; it is not a log file. What is kept
//! is what passed the process's own log filter (`ZUI_LOG`), so raising the
//! level there shows more here too.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::broadcast;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// How many records are kept.
pub const KEEP: usize = 2000;
/// Longest message kept, so one runaway line cannot fill the ring.
const MAX_MESSAGE: usize = 2000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Record {
    /// Rises with every record, so a client can tell what it has seen.
    pub id: u64,
    /// Unix milliseconds.
    pub time: i64,
    pub level: &'static str,
    /// `core`, `panel` or `other`.
    pub source: &'static str,
    pub target: String,
    pub message: String,
}

/// 0 for the most severe.
fn rank(level: &str) -> u8 {
    match level {
        "error" => 0,
        "warn" => 1,
        "info" => 2,
        "debug" => 3,
        _ => 4,
    }
}

fn level_name(level: &Level) -> &'static str {
    match *level {
        Level::ERROR => "error",
        Level::WARN => "warn",
        Level::INFO => "info",
        Level::DEBUG => "debug",
        Level::TRACE => "trace",
    }
}

fn source_of(target: &str) -> &'static str {
    let root = target.split("::").next().unwrap_or(target);
    if root.starts_with("zero_") || root == "zray" || root.starts_with("zray_") {
        "core"
    } else if root.starts_with("z_") {
        "panel"
    } else {
        "other"
    }
}

/// What the viewer asked to see.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// `core` or `panel`; anything else, everything.
    pub source: Option<String>,
    /// This level and the more severe ones.
    pub level: Option<String>,
    /// A word in the message or the target, in any case.
    pub search: Option<String>,
}

impl Filter {
    pub fn allows(&self, record: &Record) -> bool {
        if let Some(source) = self
            .source
            .as_deref()
            .filter(|s| matches!(*s, "core" | "panel"))
        {
            if record.source != source {
                return false;
            }
        }
        if let Some(level) = self.level.as_deref().filter(|l| !l.is_empty()) {
            if rank(record.level) > rank(level) {
                return false;
            }
        }
        if let Some(word) = self
            .search
            .as_deref()
            .map(str::trim)
            .filter(|w| !w.is_empty())
        {
            let word = word.to_lowercase();
            if !record.message.to_lowercase().contains(&word)
                && !record.target.to_lowercase().contains(&word)
            {
                return false;
            }
        }
        true
    }
}

/// The levels the dashboard can set the whole process to.
pub const LEVELS: &[&str] = &["error", "warn", "info", "debug"];

/// The filter directive for a level: that level everywhere, except in the
/// libraries that say too much at debug (every SQL statement, every HTTP
/// frame), which stay at warnings.
pub fn directive(level: &str) -> String {
    format!("{level},sqlx=warn,hyper=warn,hyper_util=warn,h2=warn,rustls=warn,tokio_util=warn")
}

/// Swaps the process's log filter for one made from a directive.
type Reloader = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// How the process's log level is read and changed. The default does
/// nothing but remember; `main` installs the real filter's reloader.
struct LevelControl {
    /// A level of [`LEVELS`], or the text of a custom `ZUI_LOG`.
    current: Mutex<String>,
    /// Whether `ZUI_LOG` set it, which the dashboard leaves alone.
    from_env: std::sync::atomic::AtomicBool,
    apply: Mutex<Option<Reloader>>,
}

/// The ring and the feed. Cheap to clone: it is shared.
#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Inner>,
}

struct Inner {
    ring: Mutex<VecDeque<Arc<Record>>>,
    next: AtomicU64,
    feed: broadcast::Sender<Arc<Record>>,
    level: LevelControl,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                ring: Mutex::new(VecDeque::with_capacity(KEEP)),
                next: AtomicU64::new(1),
                feed: broadcast::channel(512).0,
                level: LevelControl {
                    current: Mutex::new("info".into()),
                    from_env: std::sync::atomic::AtomicBool::new(false),
                    apply: Mutex::new(None),
                },
            }),
        }
    }
}

impl LogBuffer {
    /// Add a record; it is numbered and sent on.
    pub fn push(&self, time: i64, level: &'static str, target: &str, message: String) {
        let message = if message.len() > MAX_MESSAGE {
            let mut end = MAX_MESSAGE;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}…", &message[..end])
        } else {
            message
        };
        // Numbered and stored under the lock, so ids are in ring order.
        let mut ring = self.inner.ring.lock().unwrap_or_else(|p| p.into_inner());
        let record = Arc::new(Record {
            id: self.inner.next.fetch_add(1, Ordering::Relaxed),
            time,
            level,
            source: source_of(target),
            target: target.to_string(),
            message,
        });
        if ring.len() == KEEP {
            ring.pop_front();
        }
        ring.push_back(Arc::clone(&record));
        // Nobody watching is not an error.
        let _ = self.inner.feed.send(record);
    }

    /// The latest `limit` records that pass `filter`, oldest first.
    pub fn recent(&self, filter: &Filter, limit: usize) -> Vec<Arc<Record>> {
        let ring = self.inner.ring.lock().unwrap_or_else(|p| p.into_inner());
        let mut found: Vec<Arc<Record>> = ring
            .iter()
            .rev()
            .filter(|record| filter.allows(record))
            .take(limit)
            .cloned()
            .collect();
        found.reverse();
        found
    }

    /// The backlog and then every new record, as one consistent sequence:
    /// subscribing comes first, so nothing falls between the two.
    pub fn watch(
        &self,
        filter: &Filter,
        backlog: usize,
    ) -> (Vec<Arc<Record>>, broadcast::Receiver<Arc<Record>>) {
        let receiver = self.inner.feed.subscribe();
        (self.recent(filter, backlog), receiver)
    }

    /// Install the reloader of the process's log filter, which is now at
    /// `current` (a level, or custom text when `from_env`).
    pub fn install_level(
        &self,
        current: &str,
        from_env: bool,
        apply: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static,
    ) {
        let level = &self.inner.level;
        *level.current.lock().unwrap_or_else(|p| p.into_inner()) = current.to_string();
        level.from_env.store(from_env, Ordering::Relaxed);
        *level.apply.lock().unwrap_or_else(|p| p.into_inner()) = Some(Box::new(apply));
    }

    /// The process's log level, and whether `ZUI_LOG` fixes it.
    pub fn level(&self) -> (String, bool) {
        let level = &self.inner.level;
        (
            level
                .current
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            level.from_env.load(Ordering::Relaxed),
        )
    }

    /// Change the process's log level to one of [`LEVELS`].
    pub fn set_level(&self, name: &str) -> Result<(), String> {
        if !LEVELS.contains(&name) {
            return Err(format!("{name:?} is not a log level"));
        }
        let level = &self.inner.level;
        if level.from_env.load(Ordering::Relaxed) {
            return Err("the level is fixed by ZUI_LOG; unset it to change it here".into());
        }
        if let Some(apply) = level
            .apply
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            apply(&directive(name))?;
        }
        *level.current.lock().unwrap_or_else(|p| p.into_inner()) = name.to_string();
        Ok(())
    }

    pub fn layer(&self) -> LogLayer {
        LogLayer {
            buffer: self.clone(),
        }
    }
}

/// The `tracing` layer that fills a [`LogBuffer`].
pub struct LogLayer {
    buffer: LogBuffer,
}

#[derive(Default)]
struct Fields {
    message: String,
    rest: String,
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.rest, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.rest, " {}={value}", field.name());
        }
    }
}

impl<S: Subscriber> Layer<S> for LogLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let mut message = fields.message;
        message.push_str(&fields.rest);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let metadata = event.metadata();
        self.buffer.push(
            time,
            level_name(metadata.level()),
            metadata.target(),
            message,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn events_become_records_with_their_fields_and_source() {
        let buffer = LogBuffer::default();
        let subscriber = tracing_subscriber::registry().with(buffer.layer());
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "zero_runtime::server", tag = "vless-1", "listening");
            tracing::warn!(target: "z_ui::jobs", user = %"alice", "user status changed");
            tracing::debug!(target: "hyper::proto", "noise");
        });
        let all = buffer.recent(&Filter::default(), 10);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].message, "listening tag=vless-1");
        assert_eq!((all[0].source, all[0].level), ("core", "info"));
        assert_eq!(
            (all[1].source, all[1].message.as_str()),
            ("panel", "user status changed user=alice")
        );
        assert_eq!(all[2].source, "other");
        assert!(all[0].id < all[1].id && all[1].id < all[2].id);
    }

    #[test]
    fn a_filter_picks_source_level_and_word() {
        let buffer = LogBuffer::default();
        buffer.push(1, "info", "zero_runtime::server", "listening on 443".into());
        buffer.push(2, "warn", "zero_runtime::server", "Handshake failed".into());
        buffer.push(
            3,
            "error",
            "z_ui::jobs",
            "the enforcement pass failed".into(),
        );
        buffer.push(4, "debug", "z_db", "query".into());
        let filter = |source: &str, level: &str, search: &str| Filter {
            source: Some(source.into()),
            level: Some(level.into()),
            search: Some(search.into()),
        };
        let messages = |f: &Filter| {
            buffer
                .recent(f, 10)
                .iter()
                .map(|r| r.time)
                .collect::<Vec<_>>()
        };
        assert_eq!(messages(&Filter::default()), [1, 2, 3, 4]);
        assert_eq!(messages(&filter("core", "", "")), [1, 2]);
        assert_eq!(
            messages(&filter("panel", "warn", "")),
            [3],
            "warn and worse"
        );
        assert_eq!(messages(&filter("all", "", "HANDSHAKE")), [2], "any case");
        assert_eq!(
            messages(&filter("", "", "z_ui")),
            [3],
            "the target is searched too"
        );
        assert_eq!(
            buffer
                .recent(&Filter::default(), 2)
                .iter()
                .map(|r| r.time)
                .collect::<Vec<_>>(),
            [3, 4]
        );
    }

    #[test]
    fn the_ring_is_bounded_and_long_lines_are_cut() {
        let buffer = LogBuffer::default();
        for i in 0..(KEEP + 10) {
            buffer.push(i as i64, "info", "z_ui", format!("line {i}"));
        }
        let all = buffer.recent(&Filter::default(), KEEP + 100);
        assert_eq!(all.len(), KEEP);
        assert_eq!(all[0].message, "line 10");
        buffer.push(0, "info", "z_ui", "é".repeat(5000));
        let last = buffer.recent(&Filter::default(), 1);
        assert!(last[0].message.len() <= MAX_MESSAGE + 4 && last[0].message.ends_with('…'));
    }

    #[test]
    fn the_level_is_changed_through_the_installed_reloader() {
        let buffer = LogBuffer::default();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = Arc::clone(&seen);
        buffer.install_level("info", false, move |directive| {
            sink.lock().unwrap().push(directive.to_string());
            Ok(())
        });
        assert_eq!(buffer.level(), ("info".to_string(), false));
        buffer.set_level("debug").unwrap();
        assert_eq!(buffer.level().0, "debug");
        let directive = seen.lock().unwrap()[0].clone();
        assert!(
            directive.starts_with("debug,") && directive.contains("sqlx=warn"),
            "{directive}"
        );
        assert!(buffer.set_level("loud").is_err());
        assert_eq!(buffer.level().0, "debug", "a refused level changes nothing");

        let fixed = LogBuffer::default();
        fixed.install_level("zero_runtime=trace", true, |_| Ok(()));
        assert!(fixed.set_level("warn").unwrap_err().contains("ZUI_LOG"));
        assert_eq!(fixed.level(), ("zero_runtime=trace".to_string(), true));
    }

    #[tokio::test]
    async fn a_watcher_gets_the_backlog_then_what_follows() {
        let buffer = LogBuffer::default();
        buffer.push(1, "info", "z_ui", "before".into());
        let (backlog, mut feed) = buffer.watch(&Filter::default(), 10);
        buffer.push(2, "info", "z_ui", "after".into());
        assert_eq!(backlog.len(), 1);
        assert_eq!(feed.recv().await.unwrap().message, "after");
    }
}
