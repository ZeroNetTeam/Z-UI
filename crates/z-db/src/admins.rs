//! The panel's administrators.

use sqlx::any::AnyRow;
use sqlx::Row;

use crate::{now, Database, DbError};

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
}

/// The columns [`admin`] reads, as a literal so queries stay static SQL.
macro_rules! columns {
    () => {
        "id, username, password_hash, totp_secret, is_sudo, token_version, created_at, last_login_at"
    };
}

fn admin(row: AnyRow) -> Result<Admin, sqlx::Error> {
    Ok(Admin {
        id: row.try_get("id")?,
        username: row.try_get("username")?,
        password_hash: row.try_get("password_hash")?,
        totp_secret: row.try_get("totp_secret")?,
        is_sudo: row.try_get::<i64, _>("is_sudo")? != 0,
        token_version: row.try_get("token_version")?,
        created_at: row.try_get("created_at")?,
        last_login_at: row.try_get("last_login_at")?,
    })
}

impl Database {
    pub async fn admin_count(&self) -> Result<i64, DbError> {
        let row = sqlx::query("SELECT COUNT(*) FROM admins")
            .fetch_one(self.pool())
            .await?;
        Ok(row.try_get::<i64, _>(0)?)
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
        let row = sqlx::query(concat!(
            "INSERT INTO admins (username, password_hash, is_sudo, created_at) ",
            "VALUES ($1, $2, $3, $4) RETURNING ",
            columns!()
        ))
        .bind(username)
        .bind(password_hash)
        .bind(i64::from(is_sudo))
        .bind(now())
        .fetch_one(self.pool())
        .await?;
        Ok(admin(row)?)
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

    pub async fn touch_admin_login(&self, id: i64) -> Result<(), DbError> {
        sqlx::query("UPDATE admins SET last_login_at = $1 WHERE id = $2")
            .bind(now())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
}
