//! Noticing what is worth telling: users running out of data or time, the
//! core stopping, and sign-ins that look wrong.
//!
//! Each fact is told once. The usual way to be told twice is a restart, so
//! what was told is kept in the database under a key that names the fact
//! (`data_low:<user>:<limit>:<reset>`): a reset, or a raised limit, makes the
//! next crossing a new fact. The first pass on a panel only learns what is
//! already true, so switching notifications on does not send a message for
//! every user who ran out long ago.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::Event;
use crate::server::AppState;

/// How often the users and the core are looked at.
pub const INTERVAL: Duration = Duration::from_secs(60);
/// Failures from one address, within [`FAILURE_WINDOW`], that are worth a message.
const FAILURES_TO_TELL: u32 = 3;
const FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);
/// After telling, how long the same address stays quiet.
const QUIET_AFTER_TELLING: Duration = Duration::from_secs(30 * 60);
/// How long what was told is remembered.
pub const REMEMBER: i64 = 90 * 86_400;
/// Passes the core must be down for before it is said to be.
const DOWN_PASSES: u32 = 2;

const BASELINE: &str = "baseline:users";

/// Look at every user. `now` is a Unix time.
pub async fn users(state: &AppState, now: i64) -> Result<(), z_db::DbError> {
    let db = state.db();
    let settings = state.notify_settings();
    let first = !db.was_notified(BASELINE).await?;
    for user in db.all_users().await? {
        let owner = user.admin_id;
        let (id, name) = (user.id, user.username.clone());
        let reset = user.last_reset_at.unwrap_or(0);
        let mut facts: Vec<(String, Event)> = Vec::new();
        let limit = user.data_limit;
        if limit > 0 && user.status == "active" {
            let percent = (i128::from(user.used_traffic) * 100 / i128::from(limit)) as u32;
            if percent >= u32::from(settings.events.data_low_percent) && percent < 100 {
                facts.push((
                    format!("data_low:{id}:{limit}:{reset}"),
                    Event::UserDataLow {
                        user: name.clone(),
                        user_id: id,
                        owner,
                        used: user.used_traffic,
                        limit,
                        percent,
                    },
                ));
            }
        }
        if let (Some(at), "active") = (user.expire_at, user.status.as_str()) {
            let left = at - now;
            if left > 0 && left <= i64::from(settings.events.expiring_days) * 86_400 {
                facts.push((
                    format!("expiring:{id}:{at}"),
                    Event::UserExpiring {
                        user: name.clone(),
                        user_id: id,
                        owner,
                        days: (left + 86_399) / 86_400,
                        expire_at: at,
                    },
                ));
            }
        }
        match user.status.as_str() {
            "limited" => facts.push((
                format!("limited:{id}:{limit}:{reset}"),
                Event::UserLimited {
                    user: name.clone(),
                    user_id: id,
                    owner,
                    limit,
                },
            )),
            "expired" => {
                let at = user.expire_at.unwrap_or(0);
                facts.push((
                    format!("expired:{id}:{at}"),
                    Event::UserExpired {
                        user: name.clone(),
                        user_id: id,
                        owner,
                        expire_at: at,
                    },
                ));
            }
            _ => {}
        }
        for (key, event) in facts {
            if db.mark_notified(&key).await? && !first {
                state.notify(event);
            }
        }
    }
    if first {
        db.mark_notified(BASELINE).await?;
    }
    Ok(())
}

/// What is known of the core between passes.
#[derive(Debug, Default)]
pub struct CoreWatch {
    down_for: u32,
    told_down: bool,
}

/// Say so when the core stops while an inbound should be served, and when it
/// is back.
pub async fn core(state: &AppState, watch: &mut CoreWatch) -> Result<(), z_db::DbError> {
    let expected = state
        .db()
        .inbounds()
        .await?
        .iter()
        .any(|inbound| inbound.enabled);
    let running = state.node().status().await.running;
    if running {
        if watch.told_down {
            state.notify(Event::CoreUp);
        }
        *watch = CoreWatch::default();
    } else if expected {
        watch.down_for += 1;
        if watch.down_for >= DOWN_PASSES && !watch.told_down {
            watch.told_down = true;
            state.notify(Event::CoreDown);
        }
    } else {
        // Nothing to serve: stopped is as it should be.
        *watch = CoreWatch {
            told_down: watch.told_down,
            down_for: 0,
        };
    }
    Ok(())
}

/// One figure of the server over its limit, for some looks in a row.
#[derive(Debug, Default)]
pub struct Strain {
    over: u32,
    calm: u32,
    told: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Nothing,
    /// Over the limit for long enough: tell.
    High,
    /// Well under it again, after having been told: tell.
    Normal,
}

impl Strain {
    /// Take one look at `value` against `limit`. It takes `needed` looks over
    /// the limit in a row to be high, and two looks `margin` under it to be
    /// normal again, so a figure hovering at the limit is not told over and over.
    pub fn look(&mut self, value: Option<f64>, limit: f64, margin: f64, needed: u32) -> Change {
        let Some(value) = value else {
            *self = Strain::default();
            return Change::Nothing;
        };
        if value >= limit {
            self.calm = 0;
            self.over += 1;
            if self.over >= needed && !self.told {
                self.told = true;
                return Change::High;
            }
        } else {
            if !self.told {
                self.over = 0;
            }
            if value <= limit - margin {
                self.calm += 1;
                if self.told && self.calm >= 2 {
                    *self = Strain::default();
                    return Change::Normal;
                }
            }
        }
        Change::Nothing
    }
}

/// What is known of the server between passes.
#[derive(Debug, Default)]
pub struct ServerWatch {
    cpu: Strain,
    memory: Strain,
    disk: Strain,
    temperature: Strain,
}

/// Look at the processor, memory, disk and temperature of the server.
pub fn server(state: &AppState, watch: &mut ServerWatch) {
    let events = state.notify_settings().events;
    let Some(now) = state.monitor().latest() else {
        return;
    };
    let percent = |used: Option<u64>, total: Option<u64>| match (used, total) {
        (Some(used), Some(total)) if total > 0 => Some(used as f64 / total as f64 * 100.0),
        _ => None,
    };
    let looks = [
        (
            "cpu",
            &mut watch.cpu,
            now.cpu.map(f64::from),
            f64::from(events.cpu_percent),
            5.0,
            events.cpu_minutes.max(1),
        ),
        (
            "memory",
            &mut watch.memory,
            percent(now.mem_used, now.mem_total),
            f64::from(events.memory_percent),
            5.0,
            3,
        ),
        (
            "disk",
            &mut watch.disk,
            percent(now.disk_used, now.disk_total),
            f64::from(events.disk_percent),
            3.0,
            2,
        ),
        (
            "temperature",
            &mut watch.temperature,
            now.temperature,
            f64::from(events.temperature_celsius),
            5.0,
            2,
        ),
    ];
    for (what, strain, value, limit, margin, needed) in looks {
        match strain.look(value, limit, margin, needed) {
            Change::High => state.notify(Event::ServerHigh {
                what: what.into(),
                value: value.unwrap_or(limit),
                limit,
            }),
            Change::Normal => state.notify(Event::ServerNormal { what: what.into() }),
            Change::Nothing => {}
        }
    }
}

const TRAFFIC_STEP: &str = "notify.traffic_step";

/// Tell each time the traffic the panel has carried passes another multiple
/// of the step. What was told is kept with the step, so changing the step
/// (or the first look) only learns where things stand.
pub async fn traffic(state: &AppState) -> Result<(), z_db::DbError> {
    let db = state.db();
    let step = u64::from(state.notify_settings().events.traffic_step_gb.max(1)) << 30;
    let total = u64::try_from(db.lifetime_total(None).await?).unwrap_or(0);
    let multiple = total / step;
    let stored = db.setting(TRAFFIC_STEP).await?.and_then(|text| {
        let (k, s) = text.split_once(':')?;
        Some((k.parse::<u64>().ok()?, s.parse::<u64>().ok()?))
    });
    match stored {
        Some((last, was)) if was == step => {
            if multiple > last {
                state.notify(Event::TrafficStep { total, step });
                db.set_setting(TRAFFIC_STEP, &format!("{multiple}:{step}"))
                    .await?;
            }
        }
        _ => {
            db.set_setting(TRAFFIC_STEP, &format!("{multiple}:{step}"))
                .await?;
        }
    }
    Ok(())
}

const BOOT_ID: &str = "server.boot_id";

/// Tell when the machine has started again since the panel last looked.
/// `now` is this boot's id; the first look only remembers it.
pub async fn rebooted(state: &AppState, now: Option<String>) -> Result<(), z_db::DbError> {
    let Some(now) = now else { return Ok(()) };
    let before = state.db().setting(BOOT_ID).await?;
    if before.as_deref() != Some(now.as_str()) {
        state.db().set_setting(BOOT_ID, &now).await?;
        if before.is_some() {
            state.notify(Event::ServerRebooted);
        }
    }
    Ok(())
}

/// One look at everything.
pub async fn pass(
    state: &AppState,
    core_watch: &mut CoreWatch,
    server_watch: &mut ServerWatch,
    now: i64,
) {
    if let Err(error) = users(state, now).await {
        tracing::warn!(%error, "looking at the users for news failed");
    }
    if let Err(error) = core(state, core_watch).await {
        tracing::warn!(%error, "looking at the core for news failed");
    }
    server(state, server_watch);
    if let Err(error) = traffic(state).await {
        tracing::warn!(%error, "looking at the traffic for news failed");
    }
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        if let Err(error) = rebooted(&state, crate::monitor::system::boot_id()).await {
            tracing::warn!(%error, "looking for a reboot failed");
        }
        let mut ticker = tokio::time::interval(INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut core_watch = CoreWatch::default();
        let mut server_watch = ServerWatch::default();
        loop {
            ticker.tick().await;
            pass(&state, &mut core_watch, &mut server_watch, z_db::now()).await;
        }
    });
}

/// Failed sign-ins seen, by admin and address.
#[derive(Default)]
pub struct Logins {
    seen: HashMap<String, Failures>,
}

struct Failures {
    count: u32,
    first: Instant,
    told: Option<Instant>,
}

impl Logins {
    /// Count a failure; `Some(count)` when it is time to tell.
    pub fn failed(&mut self, admin: &str, address: &str, now: Instant) -> Option<u32> {
        if self.seen.len() > 5000 {
            self.seen
                .retain(|_, f| now.duration_since(f.first) < FAILURE_WINDOW);
        }
        let entry = self
            .seen
            .entry(format!("{admin}@{address}"))
            .or_insert(Failures {
                count: 0,
                first: now,
                told: None,
            });
        if now.duration_since(entry.first) > FAILURE_WINDOW {
            entry.count = 0;
            entry.first = now;
        }
        entry.count += 1;
        if entry.count < FAILURES_TO_TELL {
            return None;
        }
        if entry
            .told
            .is_some_and(|at| now.duration_since(at) < QUIET_AFTER_TELLING)
        {
            return None;
        }
        entry.told = Some(now);
        Some(entry.count)
    }

    /// Whether a lockout of `address` is news: once for each time it holds.
    pub fn locked(&mut self, address: &str, now: Instant) -> bool {
        let entry = self
            .seen
            .entry(format!("lock@{address}"))
            .or_insert(Failures {
                count: 0,
                first: now,
                told: None,
            });
        if entry
            .told
            .is_some_and(|at| now.duration_since(at) < QUIET_AFTER_TELLING)
        {
            return false;
        }
        entry.told = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::http::testing::{ok, Mock};
    use super::super::Settings;
    use super::*;
    use crate::server::testing::{panel, Panel};

    const DAY: i64 = 86_400;

    async fn world(telegram: &Mock) -> Panel {
        let panel = panel("/").await;
        let mut settings = Settings {
            language: "en".into(),
            ..Default::default()
        };
        settings.telegram.enabled = true;
        settings.telegram.token = "123:abc".into();
        settings.telegram.api_base = telegram.url();
        settings.telegram.chat_ids = vec![-1001];
        panel.state.set_notify_settings(settings.checked().unwrap());
        panel.state.notifier().retry_after(vec![]);
        panel
    }

    async fn user(
        panel: &Panel,
        name: &str,
        limit: i64,
        used: i64,
        expire: Option<i64>,
    ) -> z_db::User {
        let fields = z_db::UserFields {
            username: name.into(),
            status: "active".into(),
            data_limit: limit,
            expire_at: expire,
            reset_strategy: "no_reset".into(),
            ..Default::default()
        };
        let db = panel.state.db();
        let user = db
            .create_user(&fields, name, name, name, None)
            .await
            .unwrap();
        // A user starts its period when it is made; make that a while ago, so
        // a reset in the same second as the test is a different moment.
        sqlx::query("UPDATE users SET last_reset_at = last_reset_at - 1000 WHERE id = $1")
            .bind(user.id)
            .execute(db.pool())
            .await
            .unwrap();
        if used > 0 {
            db.add_traffic(&[z_db::TrafficDelta {
                user_id: user.id,
                uplink: used,
                downlink: 0,
            }])
            .await
            .unwrap();
        }
        user
    }

    async fn told(panel: &Panel, telegram: &Mock) -> Vec<String> {
        panel.state.notifier().settle().await;
        telegram
            .requests()
            .iter()
            .map(|r| r.json()["text"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    #[test]
    fn a_figure_is_high_after_enough_looks_over_and_normal_again_well_under() {
        let mut strain = Strain::default();
        let mut look = |value: f64| strain.look(Some(value), 90.0, 5.0, 3);
        assert_eq!(look(95.0), Change::Nothing);
        assert_eq!(look(95.0), Change::Nothing);
        assert_eq!(look(60.0), Change::Nothing, "a dip starts the count again");
        assert_eq!(look(95.0), Change::Nothing);
        assert_eq!(look(96.0), Change::Nothing);
        assert_eq!(look(97.0), Change::High, "three in a row");
        assert_eq!(look(99.0), Change::Nothing, "told once");
        // Hovering just under the limit is not normal, and does not tell again either.
        assert_eq!(look(88.0), Change::Nothing);
        assert_eq!(look(92.0), Change::Nothing);
        assert_eq!(
            look(80.0),
            Change::Nothing,
            "one look well under is not enough"
        );
        assert_eq!(look(81.0), Change::Normal);
        assert_eq!(look(50.0), Change::Nothing);
        // A sensor that is not there says nothing, and forgets.
        let mut none = Strain::default();
        assert_eq!(none.look(None, 85.0, 5.0, 2), Change::Nothing);
    }

    #[tokio::test]
    async fn the_server_is_told_when_it_stays_over_its_limits_and_when_it_is_well_again() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        let mut watch = ServerWatch::default();
        let high = |panel: &Panel, cpu: f32, temperature: Option<f64>| {
            panel
                .state
                .monitor()
                .inject(crate::monitor::system::Snapshot {
                    cpu: Some(cpu),
                    temperature,
                    ..Default::default()
                });
        };
        // Two looks over: not yet (the processor needs three minutes).
        high(&panel, 95.0, Some(70.0));
        server(&panel.state, &mut watch);
        high(&panel, 95.0, Some(70.0));
        server(&panel.state, &mut watch);
        assert!(told(&panel, &telegram).await.is_empty());
        high(&panel, 96.0, Some(90.0));
        server(&panel.state, &mut watch);
        high(&panel, 96.0, Some(91.0));
        server(&panel.state, &mut watch);
        let said = told(&panel, &telegram).await;
        assert_eq!(said.len(), 2, "the processor and the temperature: {said:?}");
        assert!(
            said.iter()
                .any(|m| m.contains("Processor") && m.contains("96%")),
            "{said:?}"
        );
        assert!(
            said.iter()
                .any(|m| m.contains("Temperature") && m.contains("85")),
            "{said:?}"
        );
        // Calm again.
        for _ in 0..2 {
            high(&panel, 20.0, Some(50.0));
            server(&panel.state, &mut watch);
        }
        let said = told(&panel, &telegram).await;
        assert_eq!(said.len(), 4, "and both are told as normal again: {said:?}");
        assert!(said.iter().filter(|m| m.contains("back to normal")).count() == 2);
    }

    #[tokio::test]
    async fn the_traffic_is_told_at_every_step_and_never_twice_and_the_first_look_only_learns() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        let mut settings = panel.state.notify_settings();
        settings.events.traffic_step_gb = 20;
        panel.state.set_notify_settings(settings);
        let gb = 1i64 << 30;
        let ali = user(&panel, "ali", 0, 15 * gb, None).await;
        traffic(&panel.state).await.unwrap();
        assert!(
            told(&panel, &telegram).await.is_empty(),
            "the first look only learns"
        );
        let add = |bytes: i64| {
            let db = panel.state.db().clone();
            async move {
                db.add_traffic(&[z_db::TrafficDelta {
                    user_id: ali.id,
                    uplink: 0,
                    downlink: bytes,
                }])
                .await
                .unwrap();
            }
        };
        add(4 * gb).await;
        traffic(&panel.state).await.unwrap();
        assert!(
            told(&panel, &telegram).await.is_empty(),
            "19 GB is not yet 20"
        );
        add(2 * gb).await;
        traffic(&panel.state).await.unwrap();
        traffic(&panel.state).await.unwrap();
        let said = told(&panel, &telegram).await;
        assert_eq!(said.len(), 1, "21 GB passed 20, told once: {said:?}");
        assert!(said[0].contains("grew"), "{said:?}");
        add(20 * gb).await;
        traffic(&panel.state).await.unwrap();
        assert_eq!(told(&panel, &telegram).await.len(), 2, "41 GB passed 40");

        // A new step is learned, not told.
        let mut settings = panel.state.notify_settings();
        settings.events.traffic_step_gb = 10;
        panel.state.set_notify_settings(settings);
        traffic(&panel.state).await.unwrap();
        assert_eq!(
            told(&panel, &telegram).await.len(),
            2,
            "changing the step tells nothing"
        );
    }

    #[tokio::test]
    async fn a_new_boot_is_told_and_the_first_look_and_a_mere_restart_are_not() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        rebooted(&panel.state, Some("boot-1".into())).await.unwrap();
        rebooted(&panel.state, Some("boot-1".into())).await.unwrap();
        rebooted(&panel.state, None).await.unwrap();
        assert!(told(&panel, &telegram).await.is_empty());
        rebooted(&panel.state, Some("boot-2".into())).await.unwrap();
        rebooted(&panel.state, Some("boot-2".into())).await.unwrap();
        let said = told(&panel, &telegram).await;
        assert_eq!(said.len(), 1);
        assert!(said[0].contains("restarted"), "{said:?}");
    }

    #[tokio::test]
    async fn the_first_pass_only_learns_and_later_crossings_are_told_once() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        let now = z_db::now();
        user(&panel, "already", 1000, 900, None).await;
        users(&panel.state, now).await.unwrap();
        assert!(
            told(&panel, &telegram).await.is_empty(),
            "what was already true is not news"
        );

        let fresh = user(&panel, "fresh", 1000, 850, None).await;
        users(&panel.state, now).await.unwrap();
        users(&panel.state, now).await.unwrap();
        let messages = told(&panel, &telegram).await;
        assert_eq!(
            messages.len(),
            1,
            "told once, however many passes: {messages:?}"
        );
        assert!(messages[0].contains("running low on data") && messages[0].contains("fresh"));
        assert!(messages[0].contains("85%"), "{}", messages[0]);

        // A reset and more use is another crossing.
        let db = panel.state.db();
        db.reset_user_traffic(fresh.id).await.unwrap();
        db.add_traffic(&[z_db::TrafficDelta {
            user_id: fresh.id,
            uplink: 950,
            downlink: 0,
        }])
        .await
        .unwrap();
        users(&panel.state, now).await.unwrap();
        assert_eq!(
            told(&panel, &telegram).await.len(),
            2,
            "after a reset, the next time is told"
        );
    }

    #[tokio::test]
    async fn a_limit_a_date_and_the_end_of_each_are_told_with_their_owner() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        let now = z_db::now();
        users(&panel.state, now).await.unwrap(); // the baseline, of nothing

        let soon = user(&panel, "soon", 0, 0, Some(now + 2 * DAY + 100)).await;
        user(&panel, "later", 0, 0, Some(now + 30 * DAY)).await;
        let out = user(&panel, "out", 1000, 1000, None).await;
        panel
            .state
            .db()
            .set_user_status(out.id, "limited")
            .await
            .unwrap();
        let late = user(&panel, "late", 0, 0, Some(now - DAY)).await;
        panel
            .state
            .db()
            .set_user_status(late.id, "expired")
            .await
            .unwrap();
        users(&panel.state, now).await.unwrap();

        let mut messages = told(&panel, &telegram).await;
        messages.sort();
        assert_eq!(messages.len(), 3, "{messages:?}");
        let all = messages.join("\n");
        assert!(
            all.contains("time is running out") && all.contains("<code>soon</code>"),
            "{all}"
        );
        assert!(
            all.contains("<code>3</code>"),
            "2 days and a bit is 3 days to the end: {all}"
        );
        assert!(all.contains("out of data") && all.contains("<code>out</code>"));
        assert!(all.contains("time has ended") && all.contains("<code>late</code>"));
        assert!(!all.contains("later"), "thirty days away is not soon");
        let _ = soon;

        // Moving the date is another fact: the same user is told again.
        panel
            .state
            .db()
            .update_user(
                late.id,
                &z_db::UserFields {
                    username: "late".into(),
                    status: "expired".into(),
                    expire_at: Some(now - 2 * DAY),
                    reset_strategy: "no_reset".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        users(&panel.state, now).await.unwrap();
        assert_eq!(told(&panel, &telegram).await.len(), 4);
    }

    #[tokio::test]
    async fn the_percent_and_the_days_are_the_admins_to_set() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        let mut settings = panel.state.notify_settings();
        settings.events.data_low_percent = 50;
        settings.events.expiring_days = 10;
        panel.state.set_notify_settings(settings);
        let now = z_db::now();
        users(&panel.state, now).await.unwrap();
        user(&panel, "half", 1000, 500, None).await;
        user(&panel, "week", 0, 0, Some(now + 9 * DAY)).await;
        users(&panel.state, now).await.unwrap();
        assert_eq!(told(&panel, &telegram).await.len(), 2);
    }

    #[tokio::test]
    async fn the_core_stopping_is_told_after_two_looks_and_its_return_once() {
        let telegram = ok().await;
        let panel = world(&telegram).await;
        let db = panel.state.db();
        let mut watch = CoreWatch::default();
        // Nothing to serve: nothing is wrong.
        core(&panel.state, &mut watch).await.unwrap();
        core(&panel.state, &mut watch).await.unwrap();
        assert!(told(&panel, &telegram).await.is_empty());

        db.create_inbound(&z_db::InboundFields {
            tag: "vless-1".into(),
            remark: "x".into(),
            enabled: true,
            listen: "127.0.0.1".into(),
            port: crate::server::testing::free_port().into(),
            protocol: "vless".into(),
            settings: r#"{"decryption":"none"}"#.into(),
            stream: r#"{"network":"raw","security":"none"}"#.into(),
            sniffing: "{}".into(),
        })
        .await
        .unwrap();
        core(&panel.state, &mut watch).await.unwrap();
        assert!(
            told(&panel, &telegram).await.is_empty(),
            "one look is not enough"
        );
        core(&panel.state, &mut watch).await.unwrap();
        core(&panel.state, &mut watch).await.unwrap();
        let messages = told(&panel, &telegram).await;
        assert_eq!(messages.len(), 1, "said once: {messages:?}");
        assert!(messages[0].contains("core has stopped"));

        crate::core_config::sync(&panel.state).await.unwrap();
        core(&panel.state, &mut watch).await.unwrap();
        core(&panel.state, &mut watch).await.unwrap();
        let messages = told(&panel, &telegram).await;
        assert_eq!(messages.len(), 2, "and its return, once: {messages:?}");
        assert!(messages[1].contains("running again"));
        panel.state.node().stop().await;
    }

    #[test]
    fn failed_sign_ins_are_told_after_a_few_and_then_left_alone_for_a_while() {
        let mut logins = Logins::default();
        let start = Instant::now();
        assert_eq!(logins.failed("root", "203.0.113.9", start), None);
        assert_eq!(logins.failed("root", "203.0.113.9", start), None);
        assert_eq!(logins.failed("root", "203.0.113.9", start), Some(3));
        assert_eq!(
            logins.failed("root", "203.0.113.9", start),
            None,
            "quiet after telling"
        );
        assert_eq!(
            logins.failed("root", "198.51.100.1", start),
            None,
            "another address starts over"
        );
        let later = start + QUIET_AFTER_TELLING + Duration::from_secs(1);
        for _ in 0..2 {
            assert_eq!(logins.failed("root", "203.0.113.9", later), None);
        }
        assert!(
            logins.failed("root", "203.0.113.9", later).is_some(),
            "told again after the quiet"
        );

        // Old failures age out of the count.
        let mut logins = Logins::default();
        logins.failed("a", "x", start);
        logins.failed("a", "x", start);
        assert_eq!(
            logins.failed("a", "x", start + FAILURE_WINDOW + Duration::from_secs(1)),
            None
        );
    }

    #[test]
    fn a_lockout_is_news_once_for_each_while() {
        let mut logins = Logins::default();
        let start = Instant::now();
        assert!(logins.locked("203.0.113.9", start));
        assert!(!logins.locked("203.0.113.9", start + Duration::from_secs(60)));
        assert!(logins.locked("198.51.100.1", start));
        assert!(logins.locked(
            "203.0.113.9",
            start + QUIET_AFTER_TELLING + Duration::from_secs(1)
        ));
    }
}
