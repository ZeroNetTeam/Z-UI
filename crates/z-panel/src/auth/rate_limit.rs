//! Failed sign-ins, counted per address and per username, so neither a
//! single address guessing many accounts nor many addresses guessing one
//! account get far.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Failures an address may make in a window before it is locked.
const MAX_ADDRESS_FAILURES: u32 = 5;
/// Failures against one username, from anywhere, before it is locked.
/// Higher than the address limit: anyone can fail against a username, so a
/// low limit would let a stranger lock the real admin out at will.
const MAX_USERNAME_FAILURES: u32 = 20;
const WINDOW: Duration = Duration::from_secs(15 * 60);
const LOCKOUT: Duration = Duration::from_secs(15 * 60);
/// Keys remembered at most; the oldest are dropped past this, so a flood
/// of addresses cannot grow the table without bound.
const MAX_KEYS: usize = 10_000;

#[derive(Default)]
pub struct LoginLimiter {
    entries: Mutex<HashMap<String, Entry>>,
}

struct Entry {
    failures: u32,
    first: Instant,
    locked_until: Option<Instant>,
}

fn keys(address: IpAddr, username: &str) -> [String; 2] {
    [
        format!("ip:{address}"),
        format!("user:{}", username.to_ascii_lowercase()),
    ]
}

impl LoginLimiter {
    /// `Some(seconds)` to wait when this address or username is locked.
    pub fn locked(&self, address: IpAddr, username: &str, now: Instant) -> Option<u64> {
        let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        keys(address, username)
            .iter()
            .filter_map(|key| entries.get(key)?.locked_until)
            .filter(|until| *until > now)
            .map(|until| (until - now).as_secs().max(1))
            .max()
    }

    pub fn failed(&self, address: IpAddr, username: &str, now: Instant) {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        if entries.len() >= MAX_KEYS {
            if let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.first)
                .map(|(key, _)| key.clone())
            {
                entries.remove(&oldest);
            }
        }
        for key in keys(address, username) {
            let limit = if key.starts_with("ip:") {
                MAX_ADDRESS_FAILURES
            } else {
                MAX_USERNAME_FAILURES
            };
            let entry = entries.entry(key).or_insert(Entry {
                failures: 0,
                first: now,
                locked_until: None,
            });
            if now.duration_since(entry.first) > WINDOW {
                *entry = Entry {
                    failures: 0,
                    first: now,
                    locked_until: None,
                };
            }
            entry.failures += 1;
            if entry.failures >= limit {
                entry.locked_until = Some(now + LOCKOUT);
            }
        }
    }

    /// A successful sign-in forgets the username's failures (not the
    /// address's: one right password does not excuse guessing others).
    pub fn succeeded(&self, username: &str) {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries.remove(&format!("user:{}", username.to_ascii_lowercase()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 1));
    const OTHER: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 2));

    #[test]
    fn five_failures_lock_the_address() {
        let limiter = LoginLimiter::default();
        let now = Instant::now();
        for _ in 0..4 {
            limiter.failed(HOME, "admin", now);
        }
        assert_eq!(limiter.locked(HOME, "admin", now), None);
        limiter.failed(HOME, "admin", now);
        assert!(limiter.locked(HOME, "admin", now).unwrap() > 800);
        assert!(
            limiter.locked(HOME, "someone", now).is_some(),
            "any name from here"
        );
        // Five failures do not lock the admin out for everyone else.
        assert_eq!(limiter.locked(OTHER, "admin", now), None);
        // And it passes.
        let later = now + LOCKOUT + Duration::from_secs(1);
        assert_eq!(limiter.locked(HOME, "admin", later), None);
    }

    #[test]
    fn many_failures_against_one_username_lock_it_everywhere() {
        let limiter = LoginLimiter::default();
        let now = Instant::now();
        for n in 0..MAX_USERNAME_FAILURES {
            let spread = IpAddr::V4(std::net::Ipv4Addr::new(198, 51, 100, n as u8));
            limiter.failed(spread, "admin", now);
        }
        assert!(limiter.locked(OTHER, "ADMIN", now).is_some());
        assert_eq!(limiter.locked(OTHER, "someone", now), None);
    }

    #[test]
    fn old_failures_age_out_and_success_clears_the_username() {
        let limiter = LoginLimiter::default();
        let now = Instant::now();
        for _ in 0..4 {
            limiter.failed(HOME, "admin", now);
        }
        limiter.failed(HOME, "admin", now + WINDOW + Duration::from_secs(1));
        assert_eq!(
            limiter.locked(HOME, "admin", now + WINDOW + Duration::from_secs(1)),
            None
        );
        limiter.succeeded("admin");
        for _ in 0..4 {
            limiter.failed(OTHER, "admin", now);
        }
        assert_eq!(limiter.locked(OTHER, "admin", now), None);
    }
}
