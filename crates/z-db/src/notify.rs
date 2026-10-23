//! What the panel has already told, and which Telegram chats are whose.

use sqlx::any::AnyRow;
use sqlx::Row;

use crate::{now, Database, DbError};

/// A Telegram chat tied to an admin or a user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramLink {
    pub chat_id: i64,
    /// `admin` or `user`.
    pub kind: String,
    pub ref_id: i64,
    pub name: String,
    pub created_at: i64,
}

fn link(row: AnyRow) -> Result<TelegramLink, sqlx::Error> {
    Ok(TelegramLink {
        chat_id: row.try_get("chat_id")?,
        kind: row.try_get("kind")?,
        ref_id: row.try_get("ref_id")?,
        name: row.try_get("name")?,
        created_at: row.try_get("created_at")?,
    })
}

impl Database {
    /// Write `key` down as told. `true` when it was not there before, so the
    /// caller that gets `true` is the one that tells.
    pub async fn mark_notified(&self, key: &str) -> Result<bool, DbError> {
        let inserted = sqlx::query(
            "INSERT INTO notify_sent (key, at) VALUES ($1, $2) ON CONFLICT (key) DO NOTHING",
        )
        .bind(key)
        .bind(now())
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(inserted > 0)
    }

    pub async fn was_notified(&self, key: &str) -> Result<bool, DbError> {
        Ok(sqlx::query("SELECT 1 FROM notify_sent WHERE key = $1")
            .bind(key)
            .fetch_optional(self.pool())
            .await?
            .is_some())
    }

    /// Forget what was told before `before`; returns how many went.
    pub async fn prune_notified(&self, before: i64) -> Result<u64, DbError> {
        Ok(
            sqlx::query("DELETE FROM notify_sent WHERE at < $1 AND key NOT LIKE 'baseline:%'")
                .bind(before)
                .execute(self.pool())
                .await?
                .rows_affected(),
        )
    }

    pub async fn telegram_link(&self, chat_id: i64) -> Result<Option<TelegramLink>, DbError> {
        let row = sqlx::query(
            "SELECT chat_id, kind, ref_id, name, created_at FROM telegram_links WHERE chat_id = $1",
        )
        .bind(chat_id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(link).transpose()?)
    }

    /// The chats of one admin or user (`kind` is `admin` or `user`).
    pub async fn telegram_links_of(
        &self,
        kind: &str,
        ref_id: i64,
    ) -> Result<Vec<TelegramLink>, DbError> {
        let rows = sqlx::query(
            "SELECT chat_id, kind, ref_id, name, created_at FROM telegram_links \
             WHERE kind = $1 AND ref_id = $2 ORDER BY chat_id",
        )
        .bind(kind)
        .bind(ref_id)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(link).collect::<Result<_, _>>()?)
    }

    /// Every chat of a kind.
    pub async fn telegram_links_by_kind(&self, kind: &str) -> Result<Vec<TelegramLink>, DbError> {
        let rows = sqlx::query(
            "SELECT chat_id, kind, ref_id, name, created_at FROM telegram_links \
             WHERE kind = $1 ORDER BY chat_id",
        )
        .bind(kind)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(link).collect::<Result<_, _>>()?)
    }

    /// Tie a chat to an admin or user, replacing what it was tied to.
    pub async fn save_telegram_link(
        &self,
        chat_id: i64,
        kind: &str,
        ref_id: i64,
        name: &str,
    ) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO telegram_links (chat_id, kind, ref_id, name, created_at) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (chat_id) DO UPDATE SET kind = excluded.kind, ref_id = excluded.ref_id, name = excluded.name",
        )
        .bind(chat_id)
        .bind(kind)
        .bind(ref_id)
        .bind(name)
        .bind(now())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn delete_telegram_link(&self, chat_id: i64) -> Result<bool, DbError> {
        Ok(sqlx::query("DELETE FROM telegram_links WHERE chat_id = $1")
            .bind(chat_id)
            .execute(self.pool())
            .await?
            .rows_affected()
            > 0)
    }

    /// Cut every chat of one admin or user.
    pub async fn delete_telegram_links_of(&self, kind: &str, ref_id: i64) -> Result<u64, DbError> {
        Ok(
            sqlx::query("DELETE FROM telegram_links WHERE kind = $1 AND ref_id = $2")
                .bind(kind)
                .bind(ref_id)
                .execute(self.pool())
                .await?
                .rows_affected(),
        )
    }

    /// Whether `admin_id` has signed in from `ip` before (making the first admin
    /// counts), as far as the audit log reaches back.
    pub async fn admin_login_seen(&self, admin_id: i64, ip: &str) -> Result<bool, DbError> {
        Ok(sqlx::query(
            "SELECT 1 FROM audit_log WHERE admin_id = $1 AND ip = $2 AND action IN ('auth.login', 'auth.setup') AND status < 400 LIMIT 1",
        )
        .bind(admin_id)
        .bind(ip)
        .fetch_optional(self.pool())
        .await?
        .is_some())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Database, NewAudit};

    #[tokio::test]
    async fn facts_are_told_once_and_chats_belong_to_someone() {
        let dir = std::env::temp_dir().join(format!("z-db-notify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&format!("sqlite://{}/panel.db", dir.display()))
            .await
            .unwrap();

        assert!(
            db.mark_notified("data_low:1:100:0").await.unwrap(),
            "the first time"
        );
        assert!(
            !db.mark_notified("data_low:1:100:0").await.unwrap(),
            "not again"
        );
        assert!(db.was_notified("data_low:1:100:0").await.unwrap());
        assert!(
            db.mark_notified("data_low:1:200:0").await.unwrap(),
            "a raised limit is another fact"
        );
        db.mark_notified("baseline:data_low").await.unwrap();
        assert_eq!(
            db.prune_notified(i64::MAX).await.unwrap(),
            2,
            "baselines stay"
        );
        assert!(db.was_notified("baseline:data_low").await.unwrap());

        db.save_telegram_link(111, "admin", 1, "Ali").await.unwrap();
        db.save_telegram_link(222, "user", 7, "Sara").await.unwrap();
        db.save_telegram_link(333, "admin", 1, "Ali's phone")
            .await
            .unwrap();
        assert_eq!(db.telegram_link(111).await.unwrap().unwrap().name, "Ali");
        assert_eq!(db.telegram_links_of("admin", 1).await.unwrap().len(), 2);
        assert_eq!(
            db.telegram_links_by_kind("user").await.unwrap()[0].ref_id,
            7
        );
        // A chat is one thing at a time.
        db.save_telegram_link(111, "user", 9, "Ali again")
            .await
            .unwrap();
        let moved = db.telegram_link(111).await.unwrap().unwrap();
        assert_eq!((moved.kind.as_str(), moved.ref_id), ("user", 9));
        assert!(db.delete_telegram_link(333).await.unwrap());
        assert!(!db.delete_telegram_link(333).await.unwrap());
        assert_eq!(db.delete_telegram_links_of("user", 9).await.unwrap(), 1);

        assert!(!db.admin_login_seen(1, "203.0.113.5").await.unwrap());
        let entry = |ip: &str, status: i64| NewAudit {
            admin_id: Some(1),
            admin_name: "ali".into(),
            via: "session".into(),
            ip: ip.into(),
            action: "auth.login".into(),
            status,
            ..Default::default()
        };
        db.add_audit(&entry("203.0.113.5", 401)).await.unwrap();
        assert!(
            !db.admin_login_seen(1, "203.0.113.5").await.unwrap(),
            "a failure is not a sign-in"
        );
        db.add_audit(&entry("203.0.113.5", 200)).await.unwrap();
        assert!(db.admin_login_seen(1, "203.0.113.5").await.unwrap());
        assert!(!db.admin_login_seen(1, "198.51.100.9").await.unwrap());
        assert!(!db.admin_login_seen(2, "203.0.113.5").await.unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
