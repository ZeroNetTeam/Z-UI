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

use z_db::{TrafficDelta, User};

use crate::accounts;
use crate::error::ApiError;
use crate::server::AppState;

/// How often a pass runs.
pub const INTERVAL: Duration = Duration::from_secs(10);
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
        loop {
            ticker.tick().await;
            if let Err(error) = enforce_once(&state).await {
                tracing::warn!(error = %error.message, "the enforcement pass failed");
            }
        }
    });
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
    let deltas: Vec<TrafficDelta> = node
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
        .collect();
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
        let status = effective_status(&after, now);
        if status != after.status {
            after.status = status.into();
        }
        if after.status != before.status {
            db.set_user_status(after.id, &after.status).await?;
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
