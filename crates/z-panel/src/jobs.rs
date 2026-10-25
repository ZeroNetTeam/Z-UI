//! What the panel does on its own, every few seconds: count traffic, and
//! hold every user to its limits.
//!
//! How a pass works ([`enforce_once`]), under the core lock so it never
//! interleaves with an admin's change:
//!  1. the core's per-account bytes since the last pass are added to the
//!     users and to this hour's usage;
//!  2. users seen online are marked, and an on-hold user's time starts on
//!     its first connection (or at its timeout);
//!  3. a period that has passed resets the period's traffic;
//!  4. a user over its data limit becomes `limited`, one past its date
//!     `expired`, and its accounts leave the core at once;
//!  5. a user on more addresses than its IP limit loses the sessions of the
//!     addresses that came last.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use z_db::{Database, NextPlan, TrafficDelta, User};

use crate::accounts;
use crate::error::ApiError;
use crate::server::AppState;

/// How often a pass runs.
pub const INTERVAL: Duration = Duration::from_secs(10);
/// How long hourly usage is kept: a bit over a year, which is as far back as
/// the charts go. Users' lifetime totals are kept in the users.
pub const USAGE_RETENTION: i64 = 400 * 86_400;
/// How often old usage is looked for.
const PRUNE_EVERY: Duration = Duration::from_secs(6 * 3600);
/// How long an address that stopped connecting still holds a place in its
/// user's IP limit.
const ADDRESS_GRACE: Duration = Duration::from_secs(120);

/// Seconds a reset period lasts.
pub fn period_seconds(strategy: &str) -> Option<i64> {
    Some(match strategy {
        "day" => 86_400,
        "week" => 7 * 86_400,
        "month" => 30 * 86_400,
        "year" => 365 * 86_400,
        _ => return None,
    })
}

/// The status `user` should have now, given its counters and dates. An
/// admin's `disabled` and a not-yet-started `on_hold` stand.
pub fn effective_status(user: &User, now: i64) -> &'static str {
    match user.status.as_str() {
        "disabled" => "disabled",
        "on_hold" if user.data_limit > 0 && user.used_traffic >= user.data_limit => "limited",
        "on_hold" => "on_hold",
        _ if user.data_limit > 0 && user.used_traffic >= user.data_limit => "limited",
        _ if user.expire_at.is_some_and(|at| at <= now) => "expired",
        _ => "active",
    }
}

/// The data limit and expiry `user` gets on moving on to `plan` now: the
/// plan's limit, plus what was left of the old one if it carries over.
pub fn switch_to(user: &User, plan: NextPlan, now: i64) -> (i64, Option<i64>) {
    let left = (user.data_limit - user.used_traffic).max(0);
    let data_limit = if plan.carry_over && plan.data_limit > 0 && user.data_limit > 0 {
        plan.data_limit + left
    } else {
        plan.data_limit
    };
    let expire_at = (plan.expire_days > 0).then(|| now + plan.expire_days * 86_400);
    (data_limit, expire_at)
}

/// Store the status `user`'s counters call for (`stored` is the one in the
/// database). A user that ran out of data or time with a next plan moves on
/// to it instead. Returns the user as it now is.
pub async fn settle(
    db: &Database,
    mut user: User,
    stored: &str,
    now: i64,
) -> Result<User, ApiError> {
    let status = effective_status(&user, now);
    if matches!(status, "limited" | "expired") {
        if let Some(plan) = user.next_plan {
            let (data_limit, expire_at) = switch_to(&user, plan, now);
            db.apply_next_plan(user.id, data_limit, expire_at).await?;
            tracing::info!(user = %user.username, "moved on to its next plan");
            return Ok(db.user(user.id).await?.unwrap_or(user));
        }
    }
    user.status = status.into();
    if user.status != stored {
        db.set_user_status(user.id, &user.status).await?;
    }
    Ok(user)
}

/// Where each user's addresses stand against its IP limit: first seen, last
/// seen. Kept in memory; a restart starts afresh.
#[derive(Default)]
pub struct AddressBook {
    seen: HashMap<i64, Vec<(IpAddr, Instant, Instant)>>,
}

impl AddressBook {
    /// Record `addresses` as connected for `user` now, and return those over
    /// `limit`: the addresses that came last.
    fn over_limit(
        &mut self,
        user: i64,
        addresses: &[IpAddr],
        limit: usize,
        now: Instant,
    ) -> Vec<IpAddr> {
        let seen = self.seen.entry(user).or_default();
        seen.retain(|(_, _, last)| now.duration_since(*last) < ADDRESS_GRACE);
        for address in addresses {
            match seen.iter_mut().find(|(seen, _, _)| seen == address) {
                Some(entry) => entry.2 = now,
                None => seen.push((*address, now, now)),
            }
        }
        seen.sort_by_key(|(_, first, _)| *first);
        seen.iter()
            .skip(limit)
            .map(|(address, _, _)| *address)
            .filter(|address| addresses.contains(address))
            .collect()
    }
}

/// Run passes until the process ends.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_prune: Option<Instant> = None;
        loop {
            ticker.tick().await;
            if let Err(error) = enforce_once(&state).await {
                tracing::warn!(error = %error.message, "the enforcement pass failed");
            }
            if last_prune.is_none_or(|at| at.elapsed() >= PRUNE_EVERY) {
                last_prune = Some(Instant::now());
                match state.db().prune_usage(z_db::now() - USAGE_RETENTION).await {
                    Ok(0) => {}
                    Ok(rows) => tracing::info!(rows, "old hourly usage was forgotten"),
                    Err(error) => tracing::warn!(%error, "old usage could not be pruned"),
                }
                if let Err(error) = state
                    .db()
                    .prune_notified(z_db::now() - crate::notify::watch::REMEMBER)
                    .await
                {
                    tracing::warn!(%error, "old notifications could not be forgotten");
                }
                let before = z_db::now() - crate::audit::RETENTION.as_secs() as i64;
                match state.db().prune_audit(before).await {
                    Ok(0) => {}
                    Ok(rows) => tracing::info!(rows, "old audit entries were forgotten"),
                    Err(error) => tracing::warn!(%error, "the audit log could not be pruned"),
                }
            }
        }
    });
}

/// What the core counted since the last look, as users' usage.
async fn traffic_deltas(
    state: &AppState,
    by_name: &std::collections::HashMap<&str, &z_db::User>,
) -> Vec<TrafficDelta> {
    state
        .node()
        .traffic(true)
        .await
        .into_iter()
        .filter_map(|traffic| {
            let user = by_name.get(traffic.email.as_str())?;
            Some(TrafficDelta {
                user_id: user.id,
                uplink: traffic.uplink as i64,
                downlink: traffic.downlink as i64,
            })
        })
        .collect()
}

/// A last count before the core stops, so what was used in the final
/// seconds is not lost (a takeover that is undone gives it back).
pub async fn flush_traffic(state: &AppState) {
    let db = state.db();
    let Ok(users) = db.all_users().await else {
        return;
    };
    let by_name = accounts::by_username(&users);
    if let Err(error) = db.add_traffic(&traffic_deltas(state, &by_name).await).await {
        tracing::warn!(%error, "the last traffic could not be counted");
    }
}

/// One pass (see the module's description).
pub async fn enforce_once(state: &AppState) -> Result<(), ApiError> {
    let _guard = state.core_lock().lock().await;
    let node = state.node();
    let db = state.db();
    let now = z_db::now();

    // 1. Traffic.
    let users = db.all_users().await?;
    let by_name = accounts::by_username(&users);
    let deltas = traffic_deltas(state, &by_name).await;
    state.relief().note_traffic(
        &deltas
            .iter()
            .map(|d| (d.user_id, (d.uplink + d.downlink).max(0) as u64))
            .collect::<Vec<_>>(),
        now,
    );
    db.add_traffic(&deltas).await?;

    // 2. Online.
    let online = node.online().await;
    let online_ids: Vec<i64> = online
        .iter()
        .filter_map(|account| by_name.get(account.email.as_str()).map(|user| user.id))
        .collect();
    db.set_users_online(&online_ids).await?;

    // 3–4. Periods, time and limits, on the counters as they are now.
    for before in db.all_users().await? {
        let mut after = before.clone();
        if let Some(period) = period_seconds(&after.reset_strategy) {
            let since = after.last_reset_at.unwrap_or(after.created_at);
            if now >= since + period {
                db.reset_user_traffic(after.id).await?;
                after.used_traffic = 0;
                after.last_reset_at = Some(now);
                if after.status == "limited" {
                    after.status = "active".into();
                }
            }
        }
        if after.status == "on_hold" {
            let connected = online_ids.contains(&after.id);
            let timed_out = after.on_hold_timeout.is_some_and(|at| now >= at);
            if connected || timed_out {
                let expire_at = after.on_hold_duration.map(|duration| now + duration);
                db.start_user_time(after.id, expire_at.unwrap_or(0)).await?;
                after.status = "active".into();
                after.expire_at = expire_at;
                after.on_hold_duration = None;
                after.on_hold_timeout = None;
                tracing::info!(user = %after.username, "on-hold time started");
            }
        }
        let after = settle(db, after, &before.status, now).await?;
        if after.status != before.status {
            tracing::info!(user = %after.username, from = %before.status, to = %after.status, "user status changed");
            accounts::sync_user(state, Some(&before), Some(&after)).await?;
        }
    }

    // 5. Addresses.
    let users = db.all_users().await?;
    let by_name = accounts::by_username(&users);
    let instant = Instant::now();
    let extras: Vec<(String, IpAddr)> = {
        let mut book = state.address_book();
        online
            .iter()
            .filter_map(|account| Some((account, by_name.get(account.email.as_str())?)))
            .filter(|(_, user)| user.ip_limit > 0)
            .flat_map(|(account, user)| {
                let addresses: Vec<IpAddr> = account
                    .addresses
                    .iter()
                    .filter_map(|a| a.parse().ok())
                    .collect();
                book.over_limit(user.id, &addresses, user.ip_limit as usize, instant)
                    .into_iter()
                    .map(|extra| (user.username.clone(), extra))
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    for (username, extra) in extras {
        let ended = node.end_address(&username, extra).await;
        if ended > 0 {
            tracing::info!(user = %username, address = %extra, ended, "over its IP limit");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_follows_the_counters() {
        let mut user = crate::accounts::tests_support::user("a", "active");
        assert_eq!(effective_status(&user, 100), "active");
        user.data_limit = 10;
        user.used_traffic = 10;
        assert_eq!(effective_status(&user, 100), "limited");
        user.used_traffic = 0;
        user.expire_at = Some(100);
        assert_eq!(effective_status(&user, 100), "expired");
        user.expire_at = Some(200);
        user.status = "limited".into();
        assert_eq!(
            effective_status(&user, 100),
            "active",
            "a raised limit frees it"
        );
        user.status = "disabled".into();
        assert_eq!(effective_status(&user, 100), "disabled");
        user.status = "on_hold".into();
        assert_eq!(
            effective_status(&user, 300),
            "on_hold",
            "its time has not started"
        );
    }

    #[test]
    fn a_next_plan_carries_what_was_left() {
        let mut user = crate::accounts::tests_support::user("a", "active");
        user.data_limit = 100;
        user.used_traffic = 70;
        let plan = NextPlan {
            data_limit: 1000,
            expire_days: 2,
            carry_over: true,
        };
        assert_eq!(switch_to(&user, plan, 10), (1030, Some(10 + 2 * 86_400)));
        let plain = NextPlan {
            carry_over: false,
            expire_days: 0,
            ..plan
        };
        assert_eq!(switch_to(&user, plain, 10), (1000, None));
        let unlimited = NextPlan {
            data_limit: 0,
            ..plan
        };
        assert_eq!(switch_to(&user, unlimited, 0).0, 0, "no limit stays none");
    }

    #[test]
    fn the_addresses_that_came_last_are_over_the_limit() {
        let mut book = AddressBook::default();
        let (a, b, c): (IpAddr, IpAddr, IpAddr) = (
            "10.0.0.1".parse().unwrap(),
            "10.0.0.2".parse().unwrap(),
            "10.0.0.3".parse().unwrap(),
        );
        let start = Instant::now();
        assert!(book.over_limit(1, &[a], 2, start).is_empty());
        assert!(book
            .over_limit(1, &[a, b], 2, start + Duration::from_secs(1))
            .is_empty());
        assert_eq!(
            book.over_limit(1, &[a, b, c], 2, start + Duration::from_secs(2)),
            vec![c]
        );
        // `a` went away long ago: `c` takes its place.
        let later = start + ADDRESS_GRACE + Duration::from_secs(5);
        assert!(book.over_limit(1, &[b, c], 2, later).is_empty());
    }
}
