//! Lightening the load by itself, when the processor or the temperature of
//! the server stays over its limit (the limits of the notifications):
//!
//!  * users who are connected but have moved nothing for a while leave the
//!    core for a few minutes, and come back when the time is up;
//!  * the users who moved the most are held to a modest speed until the load
//!    is down.
//!
//! What is idle costs the server little; what is heavy costs it most, which
//! is why both are offered. Everything it does is undone by itself, and
//! nothing is stored: a restart of the panel starts the core afresh, with
//! everybody back.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::notify::settings::Relief as Settings;
use crate::notify::Event;
use crate::server::AppState;

/// How often the load is looked at.
const INTERVAL: Duration = Duration::from_secs(15);
/// Looks over the limit in a row before acting.
const LOOKS_TO_ACT: u32 = 2;
/// Looks under it in a row before standing down.
const LOOKS_TO_REST: u32 = 4;
/// How long to leave things alone after standing down.
const QUIET: i64 = 120;
/// How long an idle or quiet user is remembered.
const FORGET: i64 = 86_400;

#[derive(Debug, Clone, Copy, Default)]
struct Activity {
    last_active: i64,
    /// Bytes moved lately, fading with every look.
    recent: f64,
}

/// What to do now.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Decision {
    pub pause: Vec<i64>,
    pub resume: Vec<i64>,
    pub throttle: Vec<i64>,
    pub release: Vec<i64>,
    pub started: bool,
    pub ended: bool,
}

impl Decision {
    fn is_empty(&self) -> bool {
        self.pause.is_empty()
            && self.resume.is_empty()
            && self.throttle.is_empty()
            && self.release.is_empty()
            && !self.ended
    }
}

#[derive(Debug, Default)]
pub struct Relief {
    activity: HashMap<i64, Activity>,
    /// Users out of the core, by when they come back.
    paused: HashMap<i64, i64>,
    throttled: HashSet<i64>,
    strained: u32,
    calm: u32,
    started_at: Option<i64>,
    quiet_until: i64,
}

impl Relief {
    /// Take what each user moved since the last look.
    pub fn note_traffic(&mut self, moved: &[(i64, u64)], now: i64) {
        for activity in self.activity.values_mut() {
            activity.recent *= 0.8;
        }
        for (user, bytes) in moved.iter().filter(|(_, bytes)| *bytes > 0) {
            let activity = self.activity.entry(*user).or_default();
            activity.recent += *bytes as f64;
            activity.last_active = now;
        }
        if self.activity.len() > 20_000 {
            self.activity.retain(|_, a| now - a.last_active < FORGET);
        }
    }

    /// Whether the panel is stepping in now.
    #[cfg(test)]
    pub fn acting(&self) -> bool {
        self.started_at.is_some()
    }

    /// One look at the load: `strained` says it is over the limit, `online`
    /// who is connected.
    pub fn decide(&mut self, now: i64, strained: bool, online: &[i64], cfg: &Settings) -> Decision {
        let mut decision = Decision::default();
        // Time is up for some who were sent out.
        let back: Vec<i64> = self
            .paused
            .iter()
            .filter(|(_, until)| **until <= now)
            .map(|(user, _)| *user)
            .collect();
        for user in back {
            self.paused.remove(&user);
            decision.resume.push(user);
        }
        if strained {
            self.strained += 1;
            self.calm = 0;
        } else {
            self.calm += 1;
            self.strained = 0;
        }

        if let Some(started) = self.started_at {
            let too_long = now - started >= i64::from(cfg.max_minutes) * 60;
            if !cfg.enabled || self.calm >= LOOKS_TO_REST || too_long {
                // Stand down: everybody back, nobody slowed.
                decision
                    .resume
                    .extend(self.paused.drain().map(|(user, _)| user));
                decision.release.extend(self.throttled.drain());
                self.started_at = None;
                self.quiet_until = now + QUIET;
                decision.ended = true;
            } else if strained && cfg.pause_idle {
                // Still strained: whoever has gone idle since leaves too.
                decision.pause = self.idle(now, online, cfg);
            }
        } else if cfg.enabled && self.strained >= LOOKS_TO_ACT && now >= self.quiet_until {
            if cfg.pause_idle {
                decision.pause = self.idle(now, online, cfg);
            }
            if cfg.throttle_busiest {
                decision.throttle = self.busiest(online, cfg);
            }
            // Nothing to do is not stepping in.
            if !decision.pause.is_empty() || !decision.throttle.is_empty() {
                self.started_at = Some(now);
                decision.started = true;
            }
        }
        decision
    }

    /// Who is connected, not paused, and has moved nothing for the idle time:
    /// paused from now.
    fn idle(&mut self, now: i64, online: &[i64], cfg: &Settings) -> Vec<i64> {
        let idle_for = i64::from(cfg.idle_minutes) * 60;
        let until = now + i64::from(cfg.pause_minutes) * 60;
        let mut sent = Vec::new();
        for user in online {
            if self.paused.contains_key(user) || self.throttled.contains(user) {
                continue;
            }
            let last = self.activity.get(user).map_or(0, |a| a.last_active);
            if now - last >= idle_for {
                self.paused.insert(*user, until);
                sent.push(*user);
            }
        }
        sent
    }

    /// The connected users that moved the most lately, not yet slowed.
    fn busiest(&mut self, online: &[i64], cfg: &Settings) -> Vec<i64> {
        let mut candidates: Vec<(i64, f64)> = online
            .iter()
            .filter(|user| !self.paused.contains_key(user) && !self.throttled.contains(user))
            .filter_map(|user| Some((*user, self.activity.get(user)?.recent)))
            .filter(|(_, recent)| *recent > 0.0)
            .collect();
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        candidates.truncate(cfg.busiest_count as usize);
        let slowed: Vec<i64> = candidates.into_iter().map(|(user, _)| user).collect();
        self.throttled.extend(slowed.iter().copied());
        slowed
    }
}

/// Bytes per second for a speed in Mbit/s.
fn bytes_per_second(mbit: u32) -> u64 {
    u64::from(mbit) * 125_000
}

/// One look: the load, who is connected, and what follows.
pub async fn pass(state: &AppState) {
    let settings = state.notify_settings();
    let (cfg, events) = (settings.relief, settings.events);
    let strained = state.monitor().latest().is_some_and(|now| {
        now.cpu
            .is_some_and(|cpu| cpu >= f32::from(events.cpu_percent))
            || now
                .temperature
                .is_some_and(|degrees| degrees >= f64::from(events.temperature_celsius))
    });
    let db = state.db();
    let Ok(users) = db.all_users().await else {
        return;
    };
    let by_name = crate::accounts::by_username(&users);
    let online: Vec<i64> = state
        .node()
        .online()
        .await
        .iter()
        .filter_map(|account| by_name.get(account.email.as_str()).map(|user| user.id))
        .collect();
    let decision = state.relief().decide(z_db::now(), strained, &online, &cfg);
    if decision.is_empty() {
        return;
    }
    let _guard = state.core_lock().lock().await;
    let by_id: HashMap<i64, &z_db::User> = users.iter().map(|u| (u.id, u)).collect();
    for id in &decision.pause {
        if let Some(user) = by_id.get(id) {
            if let Err(error) = crate::accounts::sync_user(state, Some(user), None).await {
                tracing::warn!(user = %user.username, error = %error.message, "pausing an idle user failed");
            }
        }
    }
    for id in &decision.resume {
        if let Some(user) = by_id.get(id) {
            if let Err(error) = crate::accounts::sync_user(state, None, Some(user)).await {
                tracing::warn!(user = %user.username, error = %error.message, "bringing a paused user back failed");
            }
        }
    }
    for id in &decision.throttle {
        if let Some(user) = by_id.get(id) {
            let speed = bytes_per_second(cfg.throttle_mbit);
            state.node().set_limit(&user.username, speed, speed).await;
        }
    }
    for id in &decision.release {
        if let Some(user) = by_id.get(id) {
            state
                .node()
                .set_limit(
                    &user.username,
                    user.speed_up.max(0) as u64,
                    user.speed_down.max(0) as u64,
                )
                .await;
        }
    }
    if decision.started {
        let reason = match (
            state.monitor().latest().and_then(|s| s.cpu),
            state.monitor().latest().and_then(|s| s.temperature),
        ) {
            (_, Some(t)) if t >= f64::from(events.temperature_celsius) => format!("{t:.0}°C"),
            (Some(cpu), _) => format!("{cpu:.0}%"),
            _ => String::new(),
        };
        state.notify(Event::ReliefStarted {
            paused: decision.pause.len(),
            throttled: decision.throttle.len(),
            reason,
        });
    }
    if decision.ended {
        state.notify(Event::ReliefEnded {
            resumed: decision.resume.len(),
            released: decision.release.len(),
        });
    }
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            pass(&state).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Settings {
        Settings {
            enabled: true,
            ..Settings::default()
        }
    }

    const NOW: i64 = 1_000_000;

    /// Users 1 and 2 moved a lot lately (2 more), 3 has been quiet for ten
    /// minutes and 4 was never seen; all four are connected.
    fn crowd() -> (Relief, Vec<i64>) {
        let mut relief = Relief::default();
        relief.note_traffic(&[(3, 10)], NOW - 600);
        relief.note_traffic(&[(1, 5_000_000), (2, 9_000_000)], NOW - 5);
        (relief, vec![1, 2, 3, 4])
    }

    #[test]
    fn it_waits_for_two_looks_over_the_limit_and_then_pauses_the_idle_and_slows_the_busiest() {
        let (mut relief, online) = crowd();
        assert_eq!(
            relief.decide(NOW, true, &online, &cfg()),
            Decision::default(),
            "one look is not enough"
        );
        let decision = relief.decide(NOW + 15, true, &online, &cfg());
        assert!(decision.started);
        assert_eq!(decision.pause, [3, 4], "the quiet ones leave");
        assert_eq!(decision.throttle, [2, 1], "the busiest first");
        assert!(relief.acting());
    }

    #[test]
    fn what_is_not_asked_for_is_not_done_and_nothing_happens_when_it_is_off() {
        let (mut relief, online) = crowd();
        let mut only_slow = cfg();
        only_slow.pause_idle = false;
        only_slow.busiest_count = 1;
        relief.decide(NOW, true, &online, &only_slow);
        let decision = relief.decide(NOW + 15, true, &online, &only_slow);
        assert_eq!(
            (decision.pause.len(), decision.throttle.clone()),
            (0, vec![2])
        );

        let (mut relief, online) = crowd();
        let off = Settings::default();
        relief.decide(NOW, true, &online, &off);
        assert_eq!(
            relief.decide(NOW + 15, true, &online, &off),
            Decision::default()
        );
        assert!(!relief.acting());
    }

    #[test]
    fn nobody_to_pause_or_slow_is_not_stepping_in() {
        let mut relief = Relief::default();
        relief.decide(NOW, true, &[], &cfg());
        let decision = relief.decide(NOW + 15, true, &[], &cfg());
        assert_eq!(decision, Decision::default());
        assert!(!relief.acting());
    }

    #[test]
    fn the_paused_come_back_when_their_time_is_up_and_the_strain_goes_on_pausing_new_idlers() {
        let (mut relief, online) = crowd();
        relief.decide(NOW, true, &online, &cfg());
        relief.decide(NOW + 15, true, &online, &cfg());
        // 5 minutes on, still strained: 3 and 4 are due back, and are sent out again as idle.
        let decision = relief.decide(NOW + 15 + 300, true, &online, &cfg());
        assert_eq!(decision.resume.len(), 2);
        assert!(!decision.started && !decision.ended);
        assert_eq!(
            decision.pause.len(),
            2,
            "3 and 4 are still idle (and 1 and 2 are slowed, not paused)"
        );
    }

    #[test]
    fn four_calm_looks_stand_everything_down_and_it_does_not_start_again_at_once() {
        let (mut relief, online) = crowd();
        relief.decide(NOW, true, &online, &cfg());
        relief.decide(NOW + 15, true, &online, &cfg());
        for look in 1..4 {
            let decision = relief.decide(NOW + 15 + look * 15, false, &online, &cfg());
            assert!(!decision.ended, "look {look}");
        }
        let decision = relief.decide(NOW + 15 + 4 * 15, false, &online, &cfg());
        assert!(decision.ended);
        assert_eq!(decision.resume.len(), 2, "everybody back");
        assert_eq!(decision.release.len(), 2, "nobody slowed");
        assert!(!relief.acting());
        // Strained again straight away: quiet for a while first.
        let t = NOW + 15 + 4 * 15;
        relief.decide(t + 15, true, &online, &cfg());
        assert_eq!(
            relief.decide(t + 30, true, &online, &cfg()),
            Decision::default()
        );
        assert!(
            relief.decide(t + 150, true, &online, &cfg()).started,
            "after the quiet time"
        );
    }

    #[test]
    fn it_stands_down_after_the_longest_time_or_when_it_is_switched_off() {
        let (mut relief, online) = crowd();
        let mut c = cfg();
        c.max_minutes = 10;
        relief.decide(NOW, true, &online, &c);
        relief.decide(NOW + 15, true, &online, &c);
        let late = relief.decide(NOW + 15 + 600, true, &online, &c);
        assert!(late.ended, "ten minutes was the most");

        let (mut relief, online) = crowd();
        relief.decide(NOW, true, &online, &cfg());
        relief.decide(NOW + 15, true, &online, &cfg());
        let off = relief.decide(NOW + 30, true, &online, &Settings::default());
        assert!(
            off.ended && off.release.len() == 2,
            "switching it off lifts what it did"
        );
    }

    #[test]
    fn a_speed_in_mbit_is_bytes_per_second() {
        assert_eq!(bytes_per_second(8), 1_000_000);
    }
}
