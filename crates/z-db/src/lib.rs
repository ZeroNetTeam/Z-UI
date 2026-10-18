//! `z-db` — where the panel keeps what it knows.
//!
//! One pool over SQLite (the default: one file next to the binary, nothing
//! to install) or PostgreSQL (for big installations), chosen by the URL.
//! Each database has its own migrations (they differ only where the two
//! disagree, such as how an id counts up); they run on every start and are
//! no-ops once applied.

mod admins;
mod certificates;
mod inbounds;
mod users;

pub use admins::Admin;
pub use certificates::Certificate;
pub use inbounds::{Host, HostFields, Inbound, InboundFields};
pub use users::{AccessGroup, TrafficDelta, UsagePoint, User, UserFields, UserQuery, UserTemplate};

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

/// Seconds since the Unix epoch, as the database stores times.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
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
        if url.starts_with("sqlite:") {
            sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
        } else {
            sqlx::migrate!("./migrations/postgres").run(&pool).await?;
        }
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
        assert_eq!(
            db.setting("panel.port").await.unwrap().as_deref(),
            Some("2096")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn an_unknown_scheme_is_refused() {
        assert!(matches!(
            Database::open("mysql://x").await,
            Err(DbError::Url(_))
        ));
    }

    #[tokio::test]
    async fn inbounds_hosts_and_certificates_round_trip() {
        let dir = std::env::temp_dir().join(format!("z-db-inbounds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&format!("sqlite://{}/panel.db", dir.display()))
            .await
            .unwrap();
        let fields = crate::InboundFields {
            tag: "vless-443".into(),
            remark: "Reality".into(),
            enabled: true,
            listen: String::new(),
            port: 443,
            protocol: "vless".into(),
            settings: "{}".into(),
            stream: "{}".into(),
            sniffing: "{}".into(),
        };
        let inbound = db.create_inbound(&fields).await.unwrap();
        assert!(db.create_inbound(&fields).await.is_err(), "tags are unique");
        let changed = crate::InboundFields {
            port: 8443,
            enabled: false,
            ..fields.clone()
        };
        let updated = db
            .update_inbound(inbound.id, &changed)
            .await
            .unwrap()
            .unwrap();
        assert_eq!((updated.port, updated.enabled), (8443, false));
        assert_eq!(db.inbounds().await.unwrap().len(), 1);

        let host = db
            .create_host(
                inbound.id,
                &crate::HostFields {
                    remark: "CDN".into(),
                    address: "cdn.example".into(),
                    enabled: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(host.fields.port, None);
        assert_eq!(db.hosts().await.unwrap().len(), 1);

        assert!(db.delete_inbound(inbound.id).await.unwrap());
        assert!(
            db.hosts().await.unwrap().is_empty(),
            "hosts go with their inbound"
        );
        assert!(!db.delete_inbound(inbound.id).await.unwrap());

        let certificate = db
            .create_certificate("example", "example.com", "CERT", "KEY", "upload", 1, 2)
            .await
            .unwrap();
        assert_eq!(
            db.certificate(certificate.id).await.unwrap(),
            Some(certificate.clone())
        );
        assert!(db.delete_certificate(certificate.id).await.unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn users_groups_traffic_and_templates() {
        let dir = std::env::temp_dir().join(format!("z-db-users-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&format!("sqlite://{}/panel.db", dir.display()))
            .await
            .unwrap();
        let groups = db.access_groups().await.unwrap();
        assert_eq!(groups.len(), 1, "a panel starts with the default group");
        assert!(groups[0].all_inbounds);
        let vip = db
            .save_access_group(None, "vip", false, &[3, 4])
            .await
            .unwrap();

        let fields = crate::UserFields {
            username: "alice".into(),
            status: "active".into(),
            data_limit: 1000,
            reset_strategy: "no_reset".into(),
            group_ids: vec![groups[0].id, vip],
            ..Default::default()
        };
        let alice = db
            .create_user(&fields, "uuid-a", "pw-a", "token-a", None)
            .await
            .unwrap();
        assert_eq!(alice.group_ids, vec![groups[0].id, vip]);
        assert!(
            db.create_user(&fields, "u", "p", "t", None).await.is_err(),
            "usernames are unique"
        );
        let bob = db
            .create_user(
                &crate::UserFields {
                    username: "bob".into(),
                    note: "office".into(),
                    group_ids: vec![vip],
                    ..fields.clone()
                },
                "uuid-b",
                "pw-b",
                "token-b",
                None,
            )
            .await
            .unwrap();

        let page = |search: &str, status: &str, group: i64| crate::UserQuery {
            search: Some(search.into()),
            status: Some(status.into()),
            group_id: Some(group),
            offset: 0,
            limit: 50,
        };
        let (found, total) = db.users(&page("", "", 0)).await.unwrap();
        assert_eq!((found.len(), total), (2, 2));
        assert_eq!(found[0].username, "bob", "newest first");
        assert_eq!(
            db.users(&page("OFF", "", 0)).await.unwrap().1,
            1,
            "the note is searched"
        );
        assert_eq!(db.users(&page("", "", groups[0].id)).await.unwrap().1, 1);
        db.set_user_status(bob.id, "disabled").await.unwrap();
        assert_eq!(
            db.users(&page("", "disabled", 0)).await.unwrap().0[0].username,
            "bob"
        );

        db.add_traffic(&[crate::TrafficDelta {
            user_id: alice.id,
            uplink: 300,
            downlink: 400,
        }])
        .await
        .unwrap();
        db.add_traffic(&[crate::TrafficDelta {
            user_id: alice.id,
            uplink: 1,
            downlink: 2,
        }])
        .await
        .unwrap();
        let alice = db.user(alice.id).await.unwrap().unwrap();
        assert_eq!((alice.used_traffic, alice.lifetime_traffic), (703, 703));
        let usage = db.user_usage(alice.id, 0).await.unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!((usage[0].uplink, usage[0].downlink), (301, 402));
        db.reset_user_traffic(alice.id).await.unwrap();
        let alice = db.user(alice.id).await.unwrap().unwrap();
        assert_eq!((alice.used_traffic, alice.lifetime_traffic), (0, 703));
        assert_eq!(
            db.user_by_token("token-a").await.unwrap().unwrap().id,
            alice.id
        );
        assert_eq!(db.user_counts().await.unwrap().get("disabled"), Some(&1));

        let template = crate::UserTemplate {
            name: "30 days".into(),
            data_limit: 50,
            expire_days: 30,
            reset_strategy: "month".into(),
            group_ids: vec![vip],
            ..Default::default()
        };
        let id = db.save_user_template(&template).await.unwrap();
        assert_eq!(db.user_templates().await.unwrap()[0].group_ids, vec![vip]);
        assert!(db.delete_user_template(id).await.unwrap());

        assert!(db.delete_access_group(vip).await.unwrap());
        assert_eq!(
            db.user(bob.id).await.unwrap().unwrap().group_ids,
            Vec::<i64>::new()
        );
        assert!(db.delete_user(alice.id).await.unwrap());
        assert!(db.user_usage(alice.id, 0).await.unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
