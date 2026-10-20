//! A reseller's quotas: how many users they may own, and how much data they
//! may hand out as those users' data limits. The checks are pure; the API
//! gathers the numbers and turns a refusal into an error.

use z_db::Holdings;

/// What an admin is capped at; 0 means no cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub users: i64,
    pub data: i64,
}

/// What a user was before the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Before {
    /// There was no such user: it is being made.
    New,
    /// It exists, with this peak data ([`z_db::plan_peak`]); `None` for no limit.
    Existing(Option<i64>),
}

/// A change to the users an admin owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub before: Before,
    /// The peak data of the user after the change; `None` for no limit.
    pub after: Option<i64>,
    /// How many alike users are made, for `Before::New`.
    pub count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Too many users.
    UserLimit { limit: i64 },
    /// A user without a data limit cannot be counted against a quota.
    Unlimited,
    /// Not enough of the quota is left; `left` bytes are.
    Exceeded { left: i64 },
}

pub fn check(limits: Limits, held: Holdings, change: Change) -> Result<(), Refusal> {
    let added = match change.before {
        Before::New => change.count.max(1),
        Before::Existing(_) => 0,
    };
    if limits.users > 0 && held.users + added > limits.users {
        return Err(Refusal::UserLimit {
            limit: limits.users,
        });
    }
    if limits.data > 0 {
        let was = match change.before {
            Before::New => 0,
            Before::Existing(peak) => peak.unwrap_or(0),
        };
        let Some(after) = change.after else {
            // Staying unlimited, as it was before the quota, is let be.
            return if matches!(change.before, Before::Existing(None)) {
                Ok(())
            } else {
                Err(Refusal::Unlimited)
            };
        };
        let more = after * added.max(1) - was;
        if more > 0 && held.allocated + more > limits.data {
            return Err(Refusal::Exceeded {
                left: (limits.data - held.allocated).max(0),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: i64 = 1 << 30;

    fn held(users: i64, allocated: i64) -> Holdings {
        Holdings {
            users,
            allocated,
            ..Default::default()
        }
    }

    fn make(after: Option<i64>, count: i64) -> Change {
        Change {
            before: Before::New,
            after,
            count,
        }
    }

    #[test]
    fn no_caps_means_no_refusals() {
        let free = Limits { users: 0, data: 0 };
        assert_eq!(check(free, held(9999, 9999 * GB), make(None, 50)), Ok(()));
    }

    #[test]
    fn the_user_cap_counts_the_users_about_to_be_made() {
        let limits = Limits { users: 5, data: 0 };
        assert_eq!(check(limits, held(4, 0), make(Some(GB), 1)), Ok(()));
        assert_eq!(
            check(limits, held(5, 0), make(Some(GB), 1)),
            Err(Refusal::UserLimit { limit: 5 })
        );
        assert_eq!(
            check(limits, held(2, 0), make(Some(GB), 4)),
            Err(Refusal::UserLimit { limit: 5 }),
            "a batch of four does not fit in three places"
        );
        let edit = Change {
            before: Before::Existing(Some(GB)),
            after: Some(2 * GB),
            count: 1,
        };
        assert_eq!(
            check(limits, held(5, GB), edit),
            Ok(()),
            "an edit adds no user"
        );
    }

    #[test]
    fn the_data_quota_counts_what_is_handed_out() {
        let limits = Limits {
            users: 0,
            data: 10 * GB,
        };
        assert_eq!(
            check(limits, held(1, 6 * GB), make(Some(4 * GB), 1)),
            Ok(())
        );
        assert_eq!(
            check(limits, held(1, 6 * GB), make(Some(5 * GB), 1)),
            Err(Refusal::Exceeded { left: 4 * GB })
        );
        assert_eq!(
            check(limits, held(0, 0), make(Some(4 * GB), 3)),
            Err(Refusal::Exceeded { left: 10 * GB }),
            "three of 4 GB are 12 GB"
        );
        assert_eq!(check(limits, held(0, 0), make(Some(4 * GB), 2)), Ok(()));
    }

    #[test]
    fn raising_a_users_limit_costs_only_the_difference() {
        let limits = Limits {
            users: 0,
            data: 10 * GB,
        };
        let raise = |from, to| Change {
            before: Before::Existing(Some(from)),
            after: Some(to),
            count: 1,
        };
        assert_eq!(
            check(limits, held(2, 10 * GB), raise(4 * GB, 4 * GB)),
            Ok(())
        );
        assert_eq!(
            check(limits, held(2, 10 * GB), raise(4 * GB, 5 * GB)),
            Err(Refusal::Exceeded { left: 0 })
        );
        assert_eq!(
            check(limits, held(2, 10 * GB), raise(4 * GB, 5 * GB / 2)),
            Ok(()),
            "lowering is always let"
        );
        // Over the quota already (it was cut): lowering still works.
        assert_eq!(
            check(limits, held(2, 12 * GB), raise(4 * GB, 2 * GB)),
            Ok(())
        );
    }

    #[test]
    fn a_user_without_a_limit_cannot_be_counted() {
        let limits = Limits {
            users: 0,
            data: 10 * GB,
        };
        assert_eq!(
            check(limits, held(0, 0), make(None, 1)),
            Err(Refusal::Unlimited)
        );
        let to_unlimited = Change {
            before: Before::Existing(Some(GB)),
            after: None,
            count: 1,
        };
        assert_eq!(
            check(limits, held(1, GB), to_unlimited),
            Err(Refusal::Unlimited)
        );
        let stays = Change {
            before: Before::Existing(None),
            after: None,
            count: 1,
        };
        assert_eq!(
            check(limits, held(1, 0), stays),
            Ok(()),
            "a user made before the quota can still have its note edited"
        );
        // With no quota, unlimited users are fine.
        assert_eq!(
            check(Limits { users: 3, data: 0 }, held(0, 0), make(None, 1)),
            Ok(())
        );
    }
}
