//! What an import from another panel needs of the database: the fields of a
//! user that only an import sets, usage by the hour, and the memory of the old
//! panel's subscription links ([`crate::Database::user_by_legacy`]).

use sqlx::Row;

use crate::{now, Database, DbError, User};

/// What an imported user keeps of its past.
#[derive(Debug, Clone, Default)]
pub struct ImportedUser {
    pub used_traffic: i64,
    /// Everything it ever used, resets included.
    pub lifetime_traffic: i64,
    pub created_at: i64,
    pub last_reset_at: Option<i64>,
    pub online_at: Option<i64>,
}

/// Who an old subscription link names.
#[derive(Debug, Clone, Copy)]
pub enum LegacyRef<'a> {
    /// The id the user had in the old panel.
    Id(i64),
    Username(&'a str),
}

impl Database {
    /// Give user `id` the past it had in the old panel.
    pub async fn set_user_imported(&self, id: i64, past: &ImportedUser) -> Result<(), DbError> {
        sqlx::query(concat!(
            "UPDATE users SET used_traffic = $1, lifetime_traffic = $2, created_at = $3, ",
            "last_reset_at = $4, online_at = $5 WHERE id = $6"
        ))
        .bind(past.used_traffic)
        .bind(past.lifetime_traffic.max(past.used_traffic))
        .bind(past.created_at)
        .bind(past.last_reset_at)
        .bind(past.online_at)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Add usage by the hour to user `id`'s charts: `(hour, uplink, downlink)`.
    /// What is there for an hour is added to, not replaced.
    pub async fn add_usage_hours(
        &self,
        user_id: i64,
        hours: &[(i64, i64, i64)],
    ) -> Result<(), DbError> {
        if hours.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool().begin().await?;
        for (hour, uplink, downlink) in hours {
            sqlx::query(
                "INSERT INTO user_usage (user_id, hour, uplink, downlink) VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (user_id, hour) DO UPDATE SET uplink = user_usage.uplink + excluded.uplink, \
                 downlink = user_usage.downlink + excluded.downlink",
            )
            .bind(user_id)
            .bind(hour / 3600 * 3600)
            .bind(uplink)
            .bind(downlink)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Make user `id`'s charts these hours and no others: what an earlier
    /// import put there is replaced by what the old panel counted since.
    pub async fn replace_usage_hours(
        &self,
        user_id: i64,
        hours: &[(i64, i64, i64)],
    ) -> Result<(), DbError> {
        sqlx::query("DELETE FROM user_usage WHERE user_id = $1")
            .bind(user_id)
            .execute(self.pool())
            .await?;
        self.add_usage_hours(user_id, hours).await
    }

    /// Remember the key the old panel (`kind`) signed its links with.
    pub async fn set_legacy_secret(&self, kind: &str, secret: &str) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO legacy_secrets (kind, secret, imported_at) VALUES ($1, $2, $3) \
             ON CONFLICT (kind) DO UPDATE SET secret = excluded.secret, imported_at = excluded.imported_at",
        )
        .bind(kind)
        .bind(secret)
        .bind(now())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn legacy_secret(&self, kind: &str) -> Result<Option<String>, DbError> {
        Ok(
            sqlx::query("SELECT secret FROM legacy_secrets WHERE kind = $1")
                .bind(kind)
                .fetch_optional(self.pool())
                .await?
                .map(|row| row.try_get(0))
                .transpose()?,
        )
    }

    /// Remember that the old panel's user `source_id`, named `source_name`
    /// there, is now `user_id`.
    pub async fn add_legacy_user(
        &self,
        kind: &str,
        source_id: i64,
        source_name: &str,
        user_id: i64,
        valid_after: i64,
    ) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO legacy_users (kind, source_id, source_name, user_id, valid_after) VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (kind, source_id) DO UPDATE SET user_id = excluded.user_id, \
             valid_after = excluded.valid_after, source_name = excluded.source_name",
        )
        .bind(kind)
        .bind(source_id)
        .bind(source_name)
        .bind(user_id)
        .bind(valid_after)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// The user an old subscription link of panel `kind` names, and the time
    /// before which a link no longer counts for it.
    pub async fn user_by_legacy(
        &self,
        kind: &str,
        who: LegacyRef<'_>,
    ) -> Result<Option<(User, i64)>, DbError> {
        let row = match who {
            LegacyRef::Id(id) => {
                sqlx::query("SELECT user_id, valid_after FROM legacy_users WHERE kind = $1 AND source_id = $2")
                    .bind(kind)
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await?
            }
            LegacyRef::Username(name) => {
                sqlx::query("SELECT user_id, valid_after FROM legacy_users WHERE kind = $1 AND source_name = $2")
                .bind(kind)
                .bind(name)
                .fetch_optional(self.pool())
                .await?
            }
        };
        let Some(row) = row else {
            return Ok(None);
        };
        let (user_id, valid_after): (i64, i64) = (row.try_get(0)?, row.try_get(1)?);
        Ok(self.user(user_id).await?.map(|user| (user, valid_after)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UserFields;

    async fn fresh() -> (Database, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "z-db-migrate-{}-{}",
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

    fn fields(name: &str) -> UserFields {
        UserFields {
            username: name.into(),
            status: "active".into(),
            reset_strategy: "no_reset".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn an_imported_user_keeps_its_past_and_its_old_panels_ids_find_it() {
        let (db, dir) = fresh().await;
        let user = db
            .create_user(&fields("ali"), "u", "p", "tok", None)
            .await
            .unwrap();
        db.set_user_imported(
            user.id,
            &ImportedUser {
                used_traffic: 500,
                lifetime_traffic: 300,
                created_at: 1_600_000_000,
                last_reset_at: Some(1_650_000_000),
                online_at: Some(1_700_000_000),
            },
        )
        .await
        .unwrap();
        let again = db.user(user.id).await.unwrap().unwrap();
        assert_eq!(
            (again.used_traffic, again.lifetime_traffic),
            (500, 500),
            "lifetime is never less than what is used"
        );
        assert_eq!(
            (again.created_at, again.last_reset_at, again.online_at),
            (1_600_000_000, Some(1_650_000_000), Some(1_700_000_000))
        );

        assert_eq!(db.legacy_secret("marzban").await.unwrap(), None);
        db.set_legacy_secret("marzban", "s3cret").await.unwrap();
        db.set_legacy_secret("marzban", "s4cret").await.unwrap();
        assert_eq!(
            db.legacy_secret("marzban").await.unwrap().as_deref(),
            Some("s4cret")
        );

        db.add_legacy_user("marzban", 17, "ali-old", user.id, 1_650_000_000)
            .await
            .unwrap();
        let (found, valid_after) = db
            .user_by_legacy("marzban", LegacyRef::Id(17))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (found.username.as_str(), valid_after),
            ("ali", 1_650_000_000)
        );
        assert!(
            db.user_by_legacy("marzban", LegacyRef::Username("ali-old"))
                .await
                .unwrap()
                .is_some(),
            "by the name it had there"
        );
        assert!(
            db.user_by_legacy("marzban", LegacyRef::Username("ali"))
                .await
                .unwrap()
                .is_none(),
            "not by the one it has now"
        );
        assert!(
            db.user_by_legacy("pasarguard", LegacyRef::Id(17))
                .await
                .unwrap()
                .is_none(),
            "another panel's ids are not these"
        );
        assert!(db
            .user_by_legacy("marzban", LegacyRef::Username("nobody"))
            .await
            .unwrap()
            .is_none());

        // Gone with the user.
        db.delete_user(user.id).await.unwrap();
        assert!(db
            .user_by_legacy("marzban", LegacyRef::Id(17))
            .await
            .unwrap()
            .is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn usage_by_the_hour_adds_up() {
        let (db, dir) = fresh().await;
        let user = db
            .create_user(&fields("ali"), "u", "p", "tok", None)
            .await
            .unwrap();
        db.add_usage_hours(user.id, &[(7200, 1, 10), (7200 + 5, 2, 20), (10_800, 0, 5)])
            .await
            .unwrap();
        let points = db.user_usage(user.id, 0).await.unwrap();
        let sum: (i64, i64) = points
            .iter()
            .fold((0, 0), |a, p| (a.0 + p.uplink, a.1 + p.downlink));
        assert_eq!(points.len(), 2, "two hours");
        assert_eq!(sum, (3, 35));

        // Replaced, not added to: the last count of the old panel is the one.
        db.replace_usage_hours(user.id, &[(7200, 4, 40)])
            .await
            .unwrap();
        let points = db.user_usage(user.id, 0).await.unwrap();
        assert_eq!(
            points
                .iter()
                .map(|p| (p.hour, p.uplink, p.downlink))
                .collect::<Vec<_>>(),
            [(7200, 4, 40)]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
