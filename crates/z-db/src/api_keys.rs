//! API keys. Only a hash of a key is kept; the key itself is shown once, when
//! it is made.

use sqlx::any::AnyRow;
use sqlx::Row;

use crate::{now, Database, DbError};

/// An API key as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKey {
    pub id: i64,
    pub admin_id: i64,
    pub name: String,
    /// The first characters of the key, to tell keys apart in a list.
    pub prefix: String,
    /// `*` for whatever the admin may, or comma-separated permissions.
    pub permissions: String,
    /// Comma-separated addresses or CIDR ranges; empty for from anywhere.
    pub allowed_ips: String,
    pub expires_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub last_used_ip: Option<String>,
    pub created_at: i64,
}

macro_rules! columns {
    () => {
        "id, admin_id, name, prefix, permissions, allowed_ips, expires_at, last_used_at, last_used_ip, created_at"
    };
}

fn key(row: AnyRow) -> Result<ApiKey, sqlx::Error> {
    Ok(ApiKey {
        id: row.try_get("id")?,
        admin_id: row.try_get("admin_id")?,
        name: row.try_get("name")?,
        prefix: row.try_get("prefix")?,
        permissions: row.try_get("permissions")?,
        allowed_ips: row.try_get("allowed_ips")?,
        expires_at: row.try_get("expires_at")?,
        last_used_at: row.try_get("last_used_at")?,
        last_used_ip: row.try_get("last_used_ip")?,
        created_at: row.try_get("created_at")?,
    })
}

impl Database {
    /// Keep a new key by its hash. Fails if the hash exists, which a random
    /// key never causes.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_api_key(
        &self,
        admin_id: i64,
        name: &str,
        prefix: &str,
        key_hash: &str,
        permissions: &str,
        allowed_ips: &str,
        expires_at: Option<i64>,
    ) -> Result<ApiKey, DbError> {
        let row = sqlx::query(concat!(
            "INSERT INTO api_keys (admin_id, name, prefix, key_hash, permissions, allowed_ips, expires_at, created_at) ",
            "VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING ",
            columns!()
        ))
        .bind(admin_id)
        .bind(name)
        .bind(prefix)
        .bind(key_hash)
        .bind(permissions)
        .bind(allowed_ips)
        .bind(expires_at)
        .bind(now())
        .fetch_one(self.pool())
        .await?;
        Ok(key(row)?)
    }

    pub async fn api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, DbError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            columns!(),
            " FROM api_keys WHERE key_hash = $1"
        ))
        .bind(key_hash)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(key).transpose()?)
    }

    /// An admin's keys, newest first.
    pub async fn api_keys_of(&self, admin_id: i64) -> Result<Vec<ApiKey>, DbError> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            columns!(),
            " FROM api_keys WHERE admin_id = $1 ORDER BY id DESC"
        ))
        .bind(admin_id)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(key).collect::<Result<_, _>>()?)
    }

    /// Every admin's key count, for the admins' list.
    pub async fn api_key_counts(&self) -> Result<std::collections::HashMap<i64, i64>, DbError> {
        let rows = sqlx::query("SELECT admin_id, COUNT(*) AS n FROM api_keys GROUP BY admin_id")
            .fetch_all(self.pool())
            .await?;
        rows.into_iter()
            .map(|row| Ok((row.try_get("admin_id")?, row.try_get("n")?)))
            .collect::<Result<_, sqlx::Error>>()
            .map_err(DbError::from)
    }

    /// Revoke one of `admin_id`'s keys.
    pub async fn delete_api_key(&self, id: i64, admin_id: i64) -> Result<Option<ApiKey>, DbError> {
        let row = sqlx::query(concat!(
            "DELETE FROM api_keys WHERE id = $1 AND admin_id = $2 RETURNING ",
            columns!()
        ))
        .bind(id)
        .bind(admin_id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(key).transpose()?)
    }

    pub async fn touch_api_key(&self, id: i64, address: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE api_keys SET last_used_at = $1, last_used_ip = $2 WHERE id = $3")
            .bind(now())
            .bind(address)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Database;

    #[tokio::test]
    async fn a_key_is_found_by_its_hash_used_and_revoked() {
        let dir = std::env::temp_dir().join(format!("z-db-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&format!("sqlite://{}/panel.db", dir.display()))
            .await
            .unwrap();
        let admin = db.create_admin("a", "h", true).await.unwrap();
        let other = db.create_admin("b", "h", true).await.unwrap();
        let key = db
            .create_api_key(
                admin.id,
                "bot",
                "zk_Ab12",
                "hash-1",
                "users.read",
                "10.0.0.0/8",
                Some(99),
            )
            .await
            .unwrap();
        assert_eq!(key.permissions, "users.read");
        assert_eq!(key.expires_at, Some(99));
        assert!(key.last_used_at.is_none());
        assert!(
            db.create_api_key(admin.id, "twin", "zk_x", "hash-1", "*", "", None)
                .await
                .is_err(),
            "a hash names one key"
        );

        assert_eq!(
            db.api_key_by_hash("hash-1").await.unwrap(),
            Some(key.clone())
        );
        assert_eq!(db.api_key_by_hash("nope").await.unwrap(), None);
        db.touch_api_key(key.id, "203.0.113.9").await.unwrap();
        let used = db.api_key_by_hash("hash-1").await.unwrap().unwrap();
        assert_eq!(used.last_used_ip.as_deref(), Some("203.0.113.9"));
        assert!(used.last_used_at.is_some());

        db.create_api_key(admin.id, "second", "zk_y", "hash-2", "*", "", None)
            .await
            .unwrap();
        assert_eq!(
            db.api_keys_of(admin.id).await.unwrap()[0].name,
            "second",
            "newest first"
        );
        assert_eq!(db.api_key_counts().await.unwrap()[&admin.id], 2);

        assert!(
            db.delete_api_key(key.id, other.id).await.unwrap().is_none(),
            "another admin's key is not theirs to revoke"
        );
        assert_eq!(
            db.delete_api_key(key.id, admin.id)
                .await
                .unwrap()
                .unwrap()
                .name,
            "bot"
        );
        assert_eq!(db.api_key_by_hash("hash-1").await.unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
