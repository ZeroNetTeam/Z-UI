//! The panel's administrators, what they hold, and what they may do.

use std::collections::HashMap;

use sqlx::any::AnyRow;
use sqlx::Row;

use crate::{ids_text, now, parse_ids, Database, DbError, NextPlan};

/// One administrator, as stored. The password is an Argon2 hash; the TOTP
/// secret, when two-factor sign-in is on, is base32.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admin {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub totp_secret: Option<String>,
    pub is_sudo: bool,
    /// A session carries the version it was issued under; raising it ends
    /// every session issued before.
    pub token_version: i64,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
    /// The role whose permissions this admin has; none for a sudo admin.
    pub role_id: Option<i64>,
    /// A disabled admin cannot sign in or use a key.
    pub enabled: bool,
    /// How many users they may own; 0 for any number.
    pub user_limit: i64,
    /// Bytes of data limit they may hand out across their users; 0 for no cap.
    pub data_quota: i64,
    /// The access groups they may give users; empty for every group.
    pub group_ids: Vec<i64>,
    pub note: String,
}

/// What a sudo admin sets on another admin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminFields {
    pub is_sudo: bool,
    pub role_id: Option<i64>,
    pub enabled: bool,
    pub user_limit: i64,
    pub data_quota: i64,
    pub group_ids: Vec<i64>,
    pub note: String,
}

impl Default for AdminFields {
    fn default() -> Self {
        Self {
            is_sudo: false,
            role_id: None,
            enabled: true,
            user_limit: 0,
            data_quota: 0,
            group_ids: Vec::new(),
            note: String::new(),
        }
    }
}

/// What an admin holds: the users they own and the data handed out to them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Holdings {
    pub users: i64,
    /// The data limits of their users, added up; a user's plan to move on
    /// to a bigger limit counts as that limit.
    pub allocated: i64,
    /// Users with no data limit at all, which no cap can count.
    pub unlimited: i64,
    /// What those users have used in the current period.
    pub used: i64,
}

/// The most data a user may be allowed over its life: its limit, or what its
/// next plan moves it on to (with what is left over, if that carries over).
/// `None` when it has no limit, or its next plan has none.
pub fn plan_peak(data_limit: i64, next: Option<NextPlan>) -> Option<i64> {
    if data_limit <= 0 {
        return None;
    }
    let Some(next) = next else {
        return Some(data_limit);
    };
    if next.data_limit <= 0 {
        return None;
    }
    let after = next.data_limit + if next.carry_over { data_limit } else { 0 };
    Some(data_limit.max(after))
}

/// The columns [`admin`] reads, as a literal so queries stay static SQL.
macro_rules! columns {
    () => {
        "id, username, password_hash, totp_secret, is_sudo, token_version, created_at, \
         last_login_at, role_id, enabled, user_limit, data_quota, group_ids, note"
    };
}

fn admin(row: AnyRow) -> Result<Admin, sqlx::Error> {
    let groups: String = row.try_get("group_ids")?;
    Ok(Admin {
        id: row.try_get("id")?,
        username: row.try_get("username")?,
        password_hash: row.try_get("password_hash")?,
        totp_secret: row.try_get("totp_secret")?,
        is_sudo: row.try_get::<i64, _>("is_sudo")? != 0,
        token_version: row.try_get("token_version")?,
        created_at: row.try_get("created_at")?,
        last_login_at: row.try_get("last_login_at")?,
        role_id: row.try_get("role_id")?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        user_limit: row.try_get("user_limit")?,
        data_quota: row.try_get("data_quota")?,
        group_ids: parse_ids(&groups),
        note: row.try_get("note")?,
    })
}

fn add_holding(holdings: &mut Holdings, row: &AnyRow) -> Result<(), sqlx::Error> {
    let data_limit: i64 = row.try_get("data_limit")?;
    let next = row
        .try_get::<Option<i64>, _>("next_data_limit")?
        .map(|next| -> Result<NextPlan, sqlx::Error> {
            Ok(NextPlan {
                data_limit: next,
                expire_days: 0,
                carry_over: row.try_get::<i64, _>("next_carry_over")? != 0,
            })
        })
        .transpose()?;
    holdings.users += 1;
    holdings.used += row.try_get::<i64, _>("used_traffic")?;
    match plan_peak(data_limit, next) {
        Some(peak) => holdings.allocated += peak,
        None => holdings.unlimited += 1,
    }
    Ok(())
}

impl Database {
    pub async fn admin_count(&self) -> Result<i64, DbError> {
        let row = sqlx::query("SELECT COUNT(*) FROM admins")
            .fetch_one(self.pool())
            .await?;
        Ok(row.try_get::<i64, _>(0)?)
    }

    pub async fn admins(&self) -> Result<Vec<Admin>, DbError> {
        let rows = sqlx::query(concat!("SELECT ", columns!(), " FROM admins ORDER BY id"))
            .fetch_all(self.pool())
            .await?;
        Ok(rows.into_iter().map(admin).collect::<Result<_, _>>()?)
    }

    pub async fn admin_by_username(&self, username: &str) -> Result<Option<Admin>, DbError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            columns!(),
            " FROM admins WHERE username = $1"
        ))
        .bind(username)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(admin).transpose()?)
    }

    pub async fn admin_by_id(&self, id: i64) -> Result<Option<Admin>, DbError> {
        let row = sqlx::query(concat!("SELECT ", columns!(), " FROM admins WHERE id = $1"))
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(admin).transpose()?)
    }

    /// Add an administrator. Fails if the username is taken.
    pub async fn create_admin(
        &self,
        username: &str,
        password_hash: &str,
        is_sudo: bool,
    ) -> Result<Admin, DbError> {
        self.create_admin_with(
            username,
            password_hash,
            &AdminFields {
                is_sudo,
                ..Default::default()
            },
        )
        .await
    }

    /// Add an administrator with everything set.
    pub async fn create_admin_with(
        &self,
        username: &str,
        password_hash: &str,
        fields: &AdminFields,
    ) -> Result<Admin, DbError> {
        let row = sqlx::query(concat!(
            "INSERT INTO admins (username, password_hash, is_sudo, created_at, role_id, enabled, ",
            "user_limit, data_quota, group_ids, note) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING ",
            columns!()
        ))
        .bind(username)
        .bind(password_hash)
        .bind(i64::from(fields.is_sudo))
        .bind(now())
        .bind(fields.role_id)
        .bind(i64::from(fields.enabled))
        .bind(fields.user_limit)
        .bind(fields.data_quota)
        .bind(ids_text(&fields.group_ids))
        .bind(&fields.note)
        .fetch_one(self.pool())
        .await?;
        Ok(admin(row)?)
    }

    /// Change what an admin may do and hold. Their password, 2FA and
    /// sessions are not touched.
    pub async fn update_admin(
        &self,
        id: i64,
        fields: &AdminFields,
    ) -> Result<Option<Admin>, DbError> {
        let row = sqlx::query(concat!(
            "UPDATE admins SET is_sudo = $1, role_id = $2, enabled = $3, user_limit = $4, ",
            "data_quota = $5, group_ids = $6, note = $7 WHERE id = $8 RETURNING ",
            columns!()
        ))
        .bind(i64::from(fields.is_sudo))
        .bind(fields.role_id)
        .bind(i64::from(fields.enabled))
        .bind(fields.user_limit)
        .bind(fields.data_quota)
        .bind(ids_text(&fields.group_ids))
        .bind(&fields.note)
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(admin).transpose()?)
    }

    /// Remove an admin and their API keys. Their users go to `heir`.
    pub async fn delete_admin(&self, id: i64, heir: i64) -> Result<bool, DbError> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("UPDATE users SET admin_id = $1 WHERE admin_id = $2")
            .bind(heir)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM api_keys WHERE admin_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let deleted = sqlx::query("DELETE FROM admins WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(deleted > 0)
    }

    /// Enabled sudo admins, of whom at least one must always remain.
    pub async fn sudo_count(&self) -> Result<i64, DbError> {
        let row = sqlx::query("SELECT COUNT(*) FROM admins WHERE is_sudo = 1 AND enabled = 1")
            .fetch_one(self.pool())
            .await?;
        Ok(row.try_get::<i64, _>(0)?)
    }

    /// How many admins have `role_id`.
    pub async fn role_admin_count(&self, role_id: i64) -> Result<i64, DbError> {
        let row = sqlx::query("SELECT COUNT(*) FROM admins WHERE role_id = $1")
            .bind(role_id)
            .fetch_one(self.pool())
            .await?;
        Ok(row.try_get::<i64, _>(0)?)
    }

    /// Replace the password and end every session issued before.
    pub async fn set_admin_password(&self, id: i64, password_hash: &str) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE admins SET password_hash = $1, token_version = token_version + 1 WHERE id = $2",
        )
        .bind(password_hash)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Turn two-factor sign-in on (`Some(secret)`) or off, ending every
    /// session issued before.
    pub async fn set_admin_totp(&self, id: i64, secret: Option<&str>) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE admins SET totp_secret = $1, token_version = token_version + 1 WHERE id = $2",
        )
        .bind(secret)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// End every session an admin has open.
    pub async fn end_admin_sessions(&self, id: i64) -> Result<(), DbError> {
        sqlx::query("UPDATE admins SET token_version = token_version + 1 WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn touch_admin_login(&self, id: i64) -> Result<(), DbError> {
        sqlx::query("UPDATE admins SET last_login_at = $1 WHERE id = $2")
            .bind(now())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// What one admin holds.
    pub async fn holdings(&self, admin_id: i64) -> Result<Holdings, DbError> {
        let rows = sqlx::query(concat!(
            "SELECT data_limit, used_traffic, next_data_limit, next_carry_over ",
            "FROM users WHERE admin_id = $1"
        ))
        .bind(admin_id)
        .fetch_all(self.pool())
        .await?;
        let mut holdings = Holdings::default();
        for row in &rows {
            add_holding(&mut holdings, row)?;
        }
        Ok(holdings)
    }

    /// What every admin holds, by admin id.
    pub async fn all_holdings(&self) -> Result<HashMap<i64, Holdings>, DbError> {
        let rows = sqlx::query(concat!(
            "SELECT admin_id, data_limit, used_traffic, next_data_limit, next_carry_over ",
            "FROM users WHERE admin_id IS NOT NULL"
        ))
        .fetch_all(self.pool())
        .await?;
        let mut map: HashMap<i64, Holdings> = HashMap::new();
        for row in &rows {
            let owner: i64 = row.try_get("admin_id")?;
            add_holding(map.entry(owner).or_default(), row)?;
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    async fn fresh() -> (Database, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "z-db-admins-{}-{}",
            std::process::id(),
            crate::now() ^ rand_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&format!("sqlite://{}/panel.db", dir.display()))
            .await
            .unwrap();
        (db, dir)
    }

    fn rand_suffix() -> i64 {
        use std::hash::{BuildHasher, Hasher};
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish() as i64
    }

    #[tokio::test]
    async fn admins_are_created_found_and_changed() {
        let (db, dir) = fresh().await;
        assert_eq!(db.admin_count().await.unwrap(), 0);
        let admin = db.create_admin("owner", "hash-1", true).await.unwrap();
        assert!(admin.is_sudo && admin.totp_secret.is_none());
        assert!(admin.enabled && admin.role_id.is_none() && admin.group_ids.is_empty());
        assert_eq!(db.admin_count().await.unwrap(), 1);
        assert!(
            db.create_admin("owner", "x", false).await.is_err(),
            "duplicate username"
        );

        db.set_admin_password(admin.id, "hash-2").await.unwrap();
        db.set_admin_totp(admin.id, Some("BASE32")).await.unwrap();
        db.touch_admin_login(admin.id).await.unwrap();
        let again = db.admin_by_username("owner").await.unwrap().unwrap();
        assert_eq!(again.password_hash, "hash-2");
        assert_eq!(again.totp_secret.as_deref(), Some("BASE32"));
        assert_eq!(again.token_version, admin.token_version + 2);
        assert!(again.last_login_at.is_some());
        assert_eq!(db.admin_by_id(admin.id).await.unwrap(), Some(again));
        assert_eq!(db.admin_by_username("nobody").await.unwrap(), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn an_admin_is_given_a_role_a_quota_and_taken_away_again() {
        let (db, dir) = fresh().await;
        let boss = db.create_admin("boss", "h", true).await.unwrap();
        assert_eq!(db.sudo_count().await.unwrap(), 1);
        let role = db.create_role("shop", "users.write", true).await.unwrap();
        let reseller = db
            .create_admin_with(
                "reseller",
                "h",
                &AdminFields {
                    role_id: Some(role.id),
                    user_limit: 5,
                    data_quota: 10_000,
                    group_ids: vec![3, 1],
                    note: "ali's shop".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(!reseller.is_sudo && reseller.enabled);
        assert_eq!(reseller.role_id, Some(role.id));
        assert_eq!(reseller.group_ids, vec![3, 1]);
        assert_eq!(db.role_admin_count(role.id).await.unwrap(), 1);
        assert_eq!(db.admins().await.unwrap().len(), 2);

        let off = db
            .update_admin(
                reseller.id,
                &AdminFields {
                    enabled: false,
                    ..AdminFields {
                        role_id: Some(role.id),
                        ..Default::default()
                    }
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert!(!off.enabled && off.user_limit == 0 && off.group_ids.is_empty());
        assert_eq!(off.password_hash, "h", "a change of rights keeps the login");
        assert!(db
            .update_admin(999, &AdminFields::default())
            .await
            .unwrap()
            .is_none());

        // They own users; deleting them hands the users on and drops their keys.
        let fields = crate::UserFields {
            username: "u1".into(),
            status: "active".into(),
            data_limit: 100,
            reset_strategy: "no_reset".into(),
            ..Default::default()
        };
        let user = db
            .create_user(&fields, "a", "b", "c", Some(reseller.id))
            .await
            .unwrap();
        db.create_api_key(reseller.id, "bot", "zk_abcd", "hash", "*", "", None)
            .await
            .unwrap();
        assert!(db.delete_admin(reseller.id, boss.id).await.unwrap());
        assert_eq!(
            db.user(user.id).await.unwrap().unwrap().admin_id,
            Some(boss.id)
        );
        assert!(db.api_keys_of(reseller.id).await.unwrap().is_empty());
        assert!(!db.delete_admin(reseller.id, boss.id).await.unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_users_peak_is_the_most_data_it_can_come_to_have() {
        let plan = |data_limit, carry_over| {
            Some(NextPlan {
                data_limit,
                expire_days: 30,
                carry_over,
            })
        };
        assert_eq!(plan_peak(100, None), Some(100));
        assert_eq!(plan_peak(0, None), None, "no limit, no peak");
        assert_eq!(plan_peak(100, plan(500, false)), Some(500));
        assert_eq!(plan_peak(100, plan(500, true)), Some(600), "plus the rest");
        assert_eq!(plan_peak(100, plan(50, false)), Some(100), "never less");
        assert_eq!(
            plan_peak(100, plan(0, false)),
            None,
            "the plan has no limit"
        );
    }

    #[tokio::test]
    async fn holdings_add_up_what_an_admin_has_handed_out() {
        let (db, dir) = fresh().await;
        let a = db.create_admin("a", "h", false).await.unwrap();
        let b = db.create_admin("b", "h", false).await.unwrap();
        async fn make(
            db: &Database,
            name: &str,
            limit: i64,
            owner: i64,
            plan: Option<NextPlan>,
        ) -> crate::User {
            let fields = crate::UserFields {
                username: name.into(),
                status: "active".into(),
                data_limit: limit,
                reset_strategy: "no_reset".into(),
                next_plan: plan,
                ..Default::default()
            };
            db.create_user(&fields, name, name, name, Some(owner))
                .await
                .unwrap()
        }
        let u1 = make(&db, "u1", 100, a.id, None).await;
        make(&db, "u2", 200, a.id, None).await;
        make(&db, "u3", 0, a.id, None).await;
        make(
            &db,
            "u4",
            10,
            a.id,
            Some(NextPlan {
                data_limit: 1000,
                expire_days: 30,
                carry_over: false,
            }),
        )
        .await;
        make(&db, "other", 7, b.id, None).await;
        db.add_traffic(&[crate::TrafficDelta {
            user_id: u1.id,
            uplink: 5,
            downlink: 6,
        }])
        .await
        .unwrap();

        let held = db.holdings(a.id).await.unwrap();
        assert_eq!(
            held,
            Holdings {
                users: 4,
                allocated: 100 + 200 + 1000,
                unlimited: 1,
                used: 11
            }
        );
        assert_eq!(db.holdings(999).await.unwrap(), Holdings::default());
        let all = db.all_holdings().await.unwrap();
        assert_eq!(all[&a.id], held);
        assert_eq!(all[&b.id].allocated, 7);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
