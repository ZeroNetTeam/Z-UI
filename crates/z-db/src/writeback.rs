//! Giving usage back to the database of a panel that is being brought back
//! after this one served for a while: what its users used meanwhile is added
//! to the counters that panel keeps, so nothing counted here is lost there.
//! This is the only place another panel's database is written, and it writes
//! nothing but those counters.

use std::path::Path;

use sqlx::any::AnyPoolOptions;
use sqlx::Row;

use crate::source::encode_path;
use crate::DbError;

/// Where the old panel counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counters {
    /// 3X-UI: `client_traffics`, by the client's email.
    ThreeXUi,
    /// PasarGuard and Marzban: `users.used_traffic`, by username.
    Users,
}

/// Add `(name, bytes)` to the counters of the SQLite file `path`, all or
/// none. Returns how many names were found there.
pub async fn add_usage(
    path: &Path,
    counters: Counters,
    amounts: &[(String, i64)],
) -> Result<usize, DbError> {
    sqlx::any::install_default_drivers();
    if !path.is_file() {
        return Err(DbError::Backup(format!(
            "there is no file {}",
            path.display()
        )));
    }
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let pool = AnyPoolOptions::new()
        .max_connections(1)
        .connect(&format!(
            "sqlite://{}?mode=rw",
            encode_path(&absolute.to_string_lossy())
        ))
        .await?;
    let mut tx = pool.begin().await?;
    let mut found = 0usize;
    let has_all_time = counters == Counters::ThreeXUi
        && sqlx::query(
            "SELECT COUNT(*) FROM pragma_table_info('client_traffics') WHERE name = 'all_time'",
        )
        .fetch_one(&mut *tx)
        .await?
        .try_get::<i64, _>(0)?
            > 0;
    for (name, bytes) in amounts.iter().filter(|(_, bytes)| *bytes > 0) {
        let done = match counters {
            Counters::ThreeXUi if has_all_time => {
                sqlx::query("UPDATE client_traffics SET down = COALESCE(down, 0) + $1, all_time = COALESCE(all_time, 0) + $1 WHERE email = $2")
                    .bind(bytes)
                    .bind(name)
                    .execute(&mut *tx)
                    .await?
            }
            Counters::ThreeXUi => {
                sqlx::query("UPDATE client_traffics SET down = COALESCE(down, 0) + $1 WHERE email = $2")
                    .bind(bytes)
                    .bind(name)
                    .execute(&mut *tx)
                    .await?
            }
            Counters::Users => {
                sqlx::query("UPDATE users SET used_traffic = COALESCE(used_traffic, 0) + $1 WHERE username = $2")
                    .bind(bytes)
                    .bind(name)
                    .execute(&mut *tx)
                    .await?
            }
        };
        found += done.rows_affected() as usize;
    }
    tx.commit().await?;
    pool.close().await;
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn old_panel(script: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        sqlx::any::install_default_drivers();
        let dir = std::env::temp_dir().join(format!(
            "z-db-writeback-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.db");
        let pool = AnyPoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .unwrap();
        for statement in script.split(';').filter(|s| !s.trim().is_empty()) {
            sqlx::query(sqlx::AssertSqlSafe(statement.to_string()))
                .execute(&pool)
                .await
                .unwrap();
        }
        pool.close().await;
        (dir, path)
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    async fn read(path: &Path, sql: &str) -> Vec<(String, i64)> {
        let source = crate::Source::open(&path.to_string_lossy()).await.unwrap();
        source
            .query(sql)
            .await
            .unwrap()
            .into_iter()
            .map(|row| {
                (
                    row[0].clone().unwrap_or_default(),
                    row[1].clone().unwrap_or_default().parse().unwrap_or(0),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn what_was_used_meanwhile_is_added_to_the_counters_of_each_panel_and_nothing_else_changes(
    ) {
        let (dir, path) = old_panel(
            "CREATE TABLE client_traffics (id INTEGER PRIMARY KEY, email TEXT UNIQUE, up INTEGER, down INTEGER, all_time INTEGER, total INTEGER);
             INSERT INTO client_traffics VALUES (1, 'ali@home', 10, 100, 110, 5000), (2, 'bob@home', 0, 0, 0, 0)",
        )
        .await;
        let found = add_usage(
            &path,
            Counters::ThreeXUi,
            &[
                ("ali@home".into(), 900),
                ("nobody".into(), 5),
                ("bob@home".into(), 0),
            ],
        )
        .await
        .unwrap();
        assert_eq!(found, 1, "only a user that is there, with something used");
        assert_eq!(
            read(
                &path,
                "SELECT email, CAST(down AS TEXT) FROM client_traffics ORDER BY id"
            )
            .await,
            [("ali@home".into(), 1000), ("bob@home".into(), 0)]
        );
        assert_eq!(
            read(
                &path,
                "SELECT email, CAST(all_time AS TEXT) FROM client_traffics WHERE id = 1"
            )
            .await[0]
                .1,
            1010
        );
        assert_eq!(
            read(
                &path,
                "SELECT email, CAST(total AS TEXT) FROM client_traffics WHERE id = 1"
            )
            .await[0]
                .1,
            5000,
            "limits are not touched"
        );
        std::fs::remove_dir_all(&dir).unwrap();

        // A version of 3X-UI without `all_time`, and the panels that keep one number.
        let (dir, path) = old_panel(
            "CREATE TABLE client_traffics (id INTEGER PRIMARY KEY, email TEXT UNIQUE, up INTEGER, down INTEGER);
             INSERT INTO client_traffics VALUES (1, 'ali@home', 1, 2)",
        )
        .await;
        add_usage(&path, Counters::ThreeXUi, &[("ali@home".into(), 7)])
            .await
            .unwrap();
        assert_eq!(
            read(
                &path,
                "SELECT email, CAST(down AS TEXT) FROM client_traffics"
            )
            .await[0]
                .1,
            9
        );
        std::fs::remove_dir_all(&dir).unwrap();

        let (dir, path) = old_panel(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT UNIQUE, used_traffic INTEGER);
             INSERT INTO users VALUES (1, 'alice', 1000), (2, 'Alice', 5)",
        )
        .await;
        assert_eq!(
            add_usage(&path, Counters::Users, &[("alice".into(), 24)])
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            read(
                &path,
                "SELECT username, CAST(used_traffic AS TEXT) FROM users ORDER BY id"
            )
            .await,
            [("alice".into(), 1024), ("Alice".into(), 5)],
            "a name is matched exactly"
        );
        // A file that is not there, or a table that is not, is an error and changes nothing.
        assert!(add_usage(
            &dir.join("missing.db"),
            Counters::Users,
            &[("alice".into(), 1)]
        )
        .await
        .is_err());
        assert!(add_usage(&path, Counters::ThreeXUi, &[("alice".into(), 1)])
            .await
            .is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
