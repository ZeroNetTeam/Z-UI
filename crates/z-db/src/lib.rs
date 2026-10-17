//! `z-db` — where the panel keeps what it knows.
//!
//! One pool over SQLite (the default: one file next to the binary, nothing
//! to install) or PostgreSQL (for big installations), chosen by the URL.
//! Migrations are one set of SQL that both accept; they run on every start
//! and are no-ops once applied.

use sqlx::any::{AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Row};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("database migration: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("unsupported database URL {0:?}; use sqlite:// or postgres://")]
    Url(String),
}

/// The panel's database.
#[derive(Clone)]
pub struct Database {
    pool: AnyPool,
}

impl Database {
    /// Open the database at `url` (`sqlite://path/z-ui.db` or
    /// `postgres://…`), creating a SQLite file if it is missing, and bring
    /// its schema up to date.
    pub async fn open(url: &str) -> Result<Self, DbError> {
        sqlx::any::install_default_drivers();
        let url = if url.starts_with("sqlite:") {
            // Create the file on first start.
            if url.contains("mode=") {
                url.to_string()
            } else if url.contains('?') {
                format!("{url}&mode=rwc")
            } else {
                format!("{url}?mode=rwc")
            }
        } else if url.starts_with("postgres:") || url.starts_with("postgresql:") {
            url.to_string()
        } else {
            return Err(DbError::Url(url.into()));
        };
        // SQLite allows one writer; a small pool avoids "database is locked".
        let connections = if url.starts_with("sqlite:") { 4 } else { 16 };
        let pool = AnyPoolOptions::new()
            .max_connections(connections)
            .connect(&url)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    /// The pool, for the panel's own queries.
    pub fn pool(&self) -> &AnyPool {
        &self.pool
    }

    /// A setting, if it was ever set.
    pub async fn setting(&self, key: &str) -> Result<Option<String>, DbError> {
        let row: Option<AnyRow> = sqlx::query("SELECT value FROM settings WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| row.get::<String, _>(0)))
    }

    /// Set a setting, replacing what was there.
    pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ($1, $2) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn settings_survive_a_reopen() {
        let dir = std::env::temp_dir().join(format!("z-db-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let url = format!("sqlite://{}/panel.db", dir.display());
        let db = Database::open(&url).await.unwrap();
        assert_eq!(db.setting("panel.port").await.unwrap(), None);
        db.set_setting("panel.port", "2053").await.unwrap();
        db.set_setting("panel.port", "2096").await.unwrap();
        drop(db);
        let db = Database::open(&url).await.unwrap();
        assert_eq!(db.setting("panel.port").await.unwrap().as_deref(), Some("2096"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn an_unknown_scheme_is_refused() {
        assert!(matches!(
            Database::open("mysql://x").await,
            Err(DbError::Url(_))
        ));
    }
}
