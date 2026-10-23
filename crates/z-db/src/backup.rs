//! A backup of everything the panel keeps, as one JSON document, and putting
//! it back.
//!
//! The document is logical, not a copy of a file: for each table, its columns
//! and its rows. That is what lets one backup be read by SQLite and by
//! PostgreSQL alike, and by a newer panel than the one that made it. The
//! tables and columns are not listed here: they are asked of the database
//! itself, so a table a later migration adds is in the next backup with no
//! change to this file.
//!
//! Reading happens in one transaction, so what is written is one moment of
//! the panel. Putting back is one transaction too: it either replaces
//! everything or changes nothing.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use serde::Deserialize;
use serde_json::Value;
use sqlx::{AssertSqlSafe, Row};
use tokio_stream::StreamExt;

use crate::{now, Database, DbError};

/// What a backup says it is.
pub const BACKUP_FORMAT: &str = "z-ui-backup";
const VERSION: u32 = 1;
/// The table sqlx keeps its own migration record in: never part of a backup.
const MIGRATIONS: &str = "_sqlx_migrations";

/// What was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    pub schema: i64,
    /// Rows written, by table.
    pub tables: Vec<(String, u64)>,
}

/// What was put back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub schema: i64,
    pub panel_version: String,
    pub created_at: i64,
    pub engine: String,
    /// Rows put back, by table.
    pub tables: Vec<(String, u64)>,
}

#[derive(Debug, Clone)]
struct Column {
    name: String,
    /// A number (otherwise text).
    number: bool,
}

#[derive(Deserialize)]
struct File {
    format: String,
    version: u32,
    schema: i64,
    #[serde(default)]
    engine: String,
    #[serde(default)]
    panel_version: String,
    #[serde(default)]
    created_at: i64,
    tables: BTreeMap<String, TableData>,
}

#[derive(Deserialize)]
struct TableData {
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
}

fn bad(message: impl Into<String>) -> DbError {
    DbError::Backup(message.into())
}

fn io(error: std::io::Error) -> DbError {
    DbError::Backup(error.to_string())
}

impl Database {
    pub fn is_postgres(&self) -> bool {
        self.postgres
    }

    /// The newest migration this database has had applied.
    pub async fn schema_version(&self) -> Result<i64, DbError> {
        let row = sqlx::query("SELECT COALESCE(MAX(version), 0) AS v FROM _sqlx_migrations")
            .fetch_one(self.pool())
            .await?;
        Ok(row.try_get("v")?)
    }

    /// Every table of the panel, with its columns, in a stable order.
    async fn layout(&self) -> Result<Vec<(String, Vec<Column>)>, DbError> {
        let names: Vec<String> = if self.postgres {
            sqlx::query(
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = current_schema() AND table_type = 'BASE TABLE' ORDER BY table_name",
            )
        } else {
            sqlx::query(
                "SELECT name AS table_name FROM sqlite_master \
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
        }
        .fetch_all(self.pool())
        .await?
        .into_iter()
        .map(|row| row.try_get::<String, _>("table_name"))
        .collect::<Result<_, _>>()?;
        let mut layout = Vec::new();
        for table in names.into_iter().filter(|name| name != MIGRATIONS) {
            let rows = if self.postgres {
                sqlx::query(
                    "SELECT column_name AS name, data_type AS kind FROM information_schema.columns \
                     WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
                )
                .bind(&table)
                .fetch_all(self.pool())
                .await?
            } else {
                sqlx::query("SELECT name, type AS kind FROM pragma_table_info($1) ORDER BY cid")
                    .bind(&table)
                    .fetch_all(self.pool())
                    .await?
            };
            let columns = rows
                .into_iter()
                .map(|row| {
                    let kind: String = row.try_get("kind")?;
                    Ok(Column {
                        name: row.try_get("name")?,
                        number: kind.to_ascii_lowercase().contains("int"),
                    })
                })
                .collect::<Result<_, sqlx::Error>>()?;
            layout.push((table, columns));
        }
        Ok(layout)
    }

    /// Write the whole database as a backup to `out`, apart from the tables
    /// in `skip`. Large tables are streamed row by row.
    pub async fn export_json<W: Write>(
        &self,
        out: &mut W,
        panel_version: &str,
        skip: &[&str],
    ) -> Result<ExportReport, DbError> {
        let layout = self.layout().await?;
        // One transaction: every table as of one moment.
        let mut tx = self.pool().begin().await?;
        if self.postgres {
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .execute(&mut *tx)
                .await?;
        }
        let schema = {
            let row = sqlx::query("SELECT COALESCE(MAX(version), 0) AS v FROM _sqlx_migrations")
                .fetch_one(&mut *tx)
                .await?;
            row.try_get::<i64, _>("v")?
        };
        write!(
            out,
            "{{\"format\":{},\"version\":{VERSION},\"engine\":{},\"schema\":{schema},\"panel_version\":{},\"created_at\":{},\"tables\":{{",
            serde_json::to_string(BACKUP_FORMAT).unwrap_or_default(),
            serde_json::to_string(if self.postgres { "postgres" } else { "sqlite" }).unwrap_or_default(),
            serde_json::to_string(panel_version).unwrap_or_default(),
            now(),
        )
        .map_err(io)?;
        let mut written = Vec::new();
        let mut first_table = true;
        for (table, columns) in layout
            .iter()
            .filter(|(name, _)| !skip.contains(&name.as_str()))
        {
            if !first_table {
                out.write_all(b",").map_err(io)?;
            }
            first_table = false;
            let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
            write!(
                out,
                "{}:{{\"columns\":{},\"rows\":[",
                serde_json::to_string(table).unwrap_or_default(),
                serde_json::to_string(&names).unwrap_or_default(),
            )
            .map_err(io)?;
            let sql = format!("SELECT {} FROM {table}", names.join(", "));
            let mut rows = sqlx::query(AssertSqlSafe(sql)).fetch(&mut *tx);
            let mut count = 0u64;
            while let Some(row) = rows.next().await {
                let row = row?;
                let cells = columns
                    .iter()
                    .enumerate()
                    .map(|(at, column)| {
                        Ok(if column.number {
                            row.try_get::<Option<i64>, _>(at)?
                                .map_or(Value::Null, Value::from)
                        } else {
                            row.try_get::<Option<String>, _>(at)?
                                .map_or(Value::Null, Value::from)
                        })
                    })
                    .collect::<Result<Vec<Value>, sqlx::Error>>()?;
                if count > 0 {
                    out.write_all(b",").map_err(io)?;
                }
                serde_json::to_writer(&mut *out, &cells).map_err(|e| bad(e.to_string()))?;
                count += 1;
            }
            out.write_all(b"]}").map_err(io)?;
            written.push((table.clone(), count));
        }
        out.write_all(b"}}").map_err(io)?;
        tx.rollback().await?;
        Ok(ExportReport {
            schema,
            tables: written,
        })
    }

    /// Replace everything the panel keeps with what the backup in `input`
    /// holds. Tables the backup leaves out are emptied, except the ones in
    /// `keep_tables`, which stay as they are (a log a backup does not carry).
    /// `keep_settings` are setting keys that stay as they are now. Nothing
    /// changes if anything is wrong with the backup.
    pub async fn import_json<R: Read + Send>(
        &self,
        input: R,
        keep_settings: &[&str],
        keep_tables: &[&str],
    ) -> Result<ImportReport, DbError> {
        let file: File = serde_json::from_reader(input)
            .map_err(|e| bad(format!("not a readable backup: {e}")))?;
        if file.format != BACKUP_FORMAT {
            return Err(bad("this is not a Z-UI backup"));
        }
        if file.version != VERSION {
            return Err(bad(format!(
                "backup format {} is not one this panel reads",
                file.version
            )));
        }
        let ours = self.schema_version().await?;
        if file.schema > ours {
            return Err(bad(format!(
                "this backup was made by a newer panel (database version {} against {ours}); update the panel first",
                file.schema
            )));
        }
        let layout = self.layout().await?;
        // Checked before anything is touched: every table and column the
        // backup names must be one this panel has, or be ignored.
        let mut plans = Vec::new();
        for (table, columns) in &layout {
            let Some(data) = file.tables.get(table) else {
                plans.push((table.clone(), columns.clone(), None));
                continue;
            };
            let mut picked = Vec::new();
            for (at, name) in data.columns.iter().enumerate() {
                if let Some(column) = columns.iter().find(|c| &c.name == name) {
                    picked.push((at, column.clone()));
                }
            }
            for row in &data.rows {
                if row.len() != data.columns.len() {
                    return Err(bad(format!(
                        "a row of {table} has {} values for {} columns",
                        row.len(),
                        data.columns.len()
                    )));
                }
                for (at, column) in &picked {
                    let ok = match &row[*at] {
                        Value::Null => true,
                        Value::Number(n) => column.number && n.is_i64(),
                        Value::String(_) => !column.number,
                        _ => false,
                    };
                    if !ok {
                        return Err(bad(format!(
                            "a value in {table}.{} is not what the column holds",
                            column.name
                        )));
                    }
                }
            }
            plans.push((table.clone(), columns.clone(), Some((data, picked))));
        }

        let mut tx = self.pool().begin().await?;
        // Settings the caller wants kept are read before the table is emptied.
        let mut kept = Vec::new();
        for key in keep_settings {
            if let Some(row) = sqlx::query("SELECT value FROM settings WHERE key = $1")
                .bind(*key)
                .fetch_optional(&mut *tx)
                .await?
            {
                kept.push((key.to_string(), row.try_get::<String, _>("value")?));
            }
        }
        let mut report = Vec::new();
        for (table, columns, data) in &plans {
            if data.is_none() && keep_tables.contains(&table.as_str()) {
                continue;
            }
            sqlx::query(AssertSqlSafe(format!("DELETE FROM {table}")))
                .execute(&mut *tx)
                .await?;
            let mut count = 0u64;
            if let Some((data, picked)) = data {
                if !picked.is_empty() {
                    let names: Vec<&str> = picked.iter().map(|(_, c)| c.name.as_str()).collect();
                    let marks: Vec<String> = (1..=names.len()).map(|n| format!("${n}")).collect();
                    let sql = format!(
                        "INSERT INTO {table} ({}) VALUES ({})",
                        names.join(", "),
                        marks.join(", ")
                    );
                    for row in &data.rows {
                        if table == "settings"
                            && keep_settings.iter().any(|k| row[0].as_str() == Some(k))
                        {
                            continue;
                        }
                        let mut query = sqlx::query(AssertSqlSafe(sql.clone()));
                        for (at, column) in picked {
                            query = match (&row[*at], column.number) {
                                (Value::Number(n), true) => query.bind(n.as_i64()),
                                (Value::String(s), false) => query.bind(Some(s.clone())),
                                (_, true) => query.bind(None::<i64>),
                                (_, false) => query.bind(None::<String>),
                            };
                        }
                        query.execute(&mut *tx).await?;
                        count += 1;
                    }
                }
            }
            if table == "settings" {
                for (key, value) in &kept {
                    sqlx::query("INSERT INTO settings (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = excluded.value")
                        .bind(key)
                        .bind(value)
                        .execute(&mut *tx)
                        .await?;
                }
            }
            // A serial column carries on from what is there now.
            if self.postgres && columns.iter().any(|c| c.name == "id" && c.number) {
                sqlx::query(AssertSqlSafe(format!(
                    "SELECT setval(pg_get_serial_sequence('{table}', 'id'), COALESCE(MAX(id), 1), MAX(id) IS NOT NULL) FROM {table}"
                )))
                .execute(&mut *tx)
                .await?;
            }
            report.push((table.clone(), count));
        }
        tx.commit().await?;
        Ok(ImportReport {
            schema: file.schema,
            panel_version: file.panel_version,
            created_at: file.created_at,
            engine: file.engine,
            tables: report,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fresh(name: &str) -> (Database, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("z-db-backup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&format!("sqlite://{}/panel.db", dir.display()))
            .await
            .unwrap();
        (db, dir)
    }

    /// A panel with a little of everything in it.
    async fn filled(db: &Database) {
        let boss = db.create_admin("boss", "hash", true).await.unwrap();
        db.set_setting("auth.session_secret", "SECRET-A")
            .await
            .unwrap();
        db.set_setting("panel.note", "with \"quotes\", commas and ünïcode ✓")
            .await
            .unwrap();
        let groups = db.access_groups().await.unwrap();
        for (name, limit) in [("ali", 1000), ("sara", 0)] {
            let fields = crate::UserFields {
                username: name.into(),
                status: "active".into(),
                data_limit: limit,
                note: "line one\nline two".into(),
                reset_strategy: "no_reset".into(),
                group_ids: vec![groups[0].id],
                ..Default::default()
            };
            let user = db
                .create_user(&fields, name, "pw", &format!("{name}-token"), Some(boss.id))
                .await
                .unwrap();
            db.add_traffic(&[crate::TrafficDelta {
                user_id: user.id,
                uplink: 5,
                downlink: 7,
            }])
            .await
            .unwrap();
        }
        db.create_api_key(boss.id, "bot", "zk_abc", "hash", "*", "", None)
            .await
            .unwrap();
        db.save_telegram_link(42, "admin", boss.id, "Ali")
            .await
            .unwrap();
    }

    async fn export(db: &Database, skip: &[&str]) -> (Vec<u8>, ExportReport) {
        let mut out = Vec::new();
        let report = db.export_json(&mut out, "0.1.0", skip).await.unwrap();
        (out, report)
    }

    #[tokio::test]
    async fn a_backup_is_json_with_every_table_and_what_is_skipped_is_left_out() {
        let (db, dir) = fresh("shape").await;
        filled(&db).await;
        let (bytes, report) = export(&db, &["user_usage", "audit_log"]).await;
        let doc: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            (
                doc["format"].as_str(),
                doc["version"].as_i64(),
                doc["engine"].as_str()
            ),
            (Some("z-ui-backup"), Some(1), Some("sqlite"))
        );
        assert_eq!(doc["schema"].as_i64(), Some(report.schema));
        assert!(report.schema >= 8);
        let tables = doc["tables"].as_object().unwrap();
        for table in [
            "users",
            "admins",
            "settings",
            "roles",
            "api_keys",
            "telegram_links",
            "access_groups",
        ] {
            assert!(tables.contains_key(table), "{table}");
        }
        assert!(
            !tables.contains_key("user_usage")
                && !tables.contains_key("audit_log")
                && !tables.contains_key("_sqlx_migrations")
        );
        let users = &tables["users"];
        assert_eq!(users["rows"].as_array().unwrap().len(), 2);
        assert!(users["columns"]
            .as_array()
            .unwrap()
            .contains(&Value::from("sub_token")));
        assert!(report.tables.iter().any(|(t, n)| t == "users" && *n == 2));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_backup_put_back_in_an_empty_panel_makes_it_the_same() {
        let (db, dir) = fresh("same").await;
        filled(&db).await;
        let (bytes, _) = export(&db, &[]).await;
        let (other, other_dir) = fresh("same-target").await;
        other.create_admin("someone-else", "x", true).await.unwrap();
        let report = other.import_json(&bytes[..], &[], &[]).await.unwrap();
        assert_eq!(report.engine, "sqlite");
        assert!(report.tables.iter().any(|(t, n)| t == "users" && *n == 2));

        let (again, _) = export(&other, &[]).await;
        let (a, b): (Value, Value) = (
            serde_json::from_slice(&bytes).unwrap(),
            serde_json::from_slice(&again).unwrap(),
        );
        assert_eq!(a["tables"], b["tables"], "every table, row for row");
        assert!(
            other
                .admin_by_username("someone-else")
                .await
                .unwrap()
                .is_none(),
            "what was there is replaced"
        );
        assert_eq!(
            other.setting("panel.note").await.unwrap().as_deref(),
            Some("with \"quotes\", commas and ünïcode ✓")
        );
        assert_eq!(
            other
                .user_by_token("ali-token")
                .await
                .unwrap()
                .unwrap()
                .note,
            "line one\nline two"
        );
        // New rows carry on after the highest id.
        let boss = other.admin_by_username("boss").await.unwrap().unwrap();
        let fields = crate::UserFields {
            username: "new".into(),
            status: "active".into(),
            reset_strategy: "no_reset".into(),
            ..Default::default()
        };
        let made = other
            .create_user(&fields, "u", "p", "t", Some(boss.id))
            .await
            .unwrap();
        assert!(made.id > 2);
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&other_dir).unwrap();
    }

    #[tokio::test]
    async fn a_restore_drops_what_the_backup_lacks_and_keeps_what_it_is_told_to() {
        let (db, dir) = fresh("keep").await;
        filled(&db).await;
        let (bytes, _) = export(&db, &["user_usage"]).await;
        // After the backup: more users, more traffic, and a new session secret.
        db.set_setting("auth.session_secret", "SECRET-B")
            .await
            .unwrap();
        let fields = crate::UserFields {
            username: "later".into(),
            status: "active".into(),
            reset_strategy: "no_reset".into(),
            ..Default::default()
        };
        let later = db
            .create_user(&fields, "u", "p", "later-token", None)
            .await
            .unwrap();
        db.add_traffic(&[crate::TrafficDelta {
            user_id: later.id,
            uplink: 1,
            downlink: 1,
        }])
        .await
        .unwrap();

        db.import_json(&bytes[..], &["auth.session_secret"], &[])
            .await
            .unwrap();
        assert!(
            db.user_by_token("later-token").await.unwrap().is_none(),
            "a user made after the backup is gone"
        );
        assert_eq!(
            db.user_by_token("ali-token")
                .await
                .unwrap()
                .unwrap()
                .username,
            "ali"
        );
        assert_eq!(
            db.setting("auth.session_secret").await.unwrap().as_deref(),
            Some("SECRET-B"),
            "kept as it is now"
        );
        assert!(
            db.total_usage(0, None).await.unwrap().is_empty(),
            "usage was not in the backup, so it is not kept either"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_bad_backup_changes_nothing() {
        let (db, dir) = fresh("bad").await;
        filled(&db).await;
        let (good, _) = export(&db, &[]).await;
        let doc: Value = serde_json::from_slice(&good).unwrap();
        let tamper = |change: &dyn Fn(&mut Value)| {
            let mut copy = doc.clone();
            change(&mut copy);
            serde_json::to_vec(&copy).unwrap()
        };
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("not json", b"hello".to_vec()),
            ("wrong format", tamper(&|d| d["format"] = "other".into())),
            ("future format", tamper(&|d| d["version"] = 99.into())),
            ("newer schema", tamper(&|d| d["schema"] = 9999.into())),
            (
                "short row",
                tamper(&|d| {
                    d["tables"]["admins"]["rows"][0]
                        .as_array_mut()
                        .unwrap()
                        .pop();
                }),
            ),
            (
                "text in a number",
                tamper(&|d| d["tables"]["users"]["rows"][0][6] = "lots".into()),
            ),
            (
                "float in a number",
                tamper(&|d| d["tables"]["users"]["rows"][0][6] = 1.5.into()),
            ),
            (
                "number in text",
                tamper(&|d| d["tables"]["users"]["rows"][0][1] = 7.into()),
            ),
        ];
        for (what, bytes) in cases {
            let error = db.import_json(&bytes[..], &[], &[]).await.unwrap_err();
            assert!(matches!(error, DbError::Backup(_)), "{what}: {error:?}");
            assert_eq!(
                db.admins().await.unwrap().len(),
                1,
                "{what}: nothing was touched"
            );
            assert!(
                db.user_by_token("ali-token").await.unwrap().is_some(),
                "{what}"
            );
        }
        // A row that breaks a constraint half-way undoes the whole thing.
        let dup = tamper(&|d| {
            let row = d["tables"]["users"]["rows"][0].clone();
            d["tables"]["users"]["rows"]
                .as_array_mut()
                .unwrap()
                .push(row);
        });
        assert!(db.import_json(&dup[..], &[], &[]).await.is_err());
        assert_eq!(db.admins().await.unwrap().len(), 1);
        assert!(
            db.user_by_token("ali-token").await.unwrap().is_some(),
            "rolled back"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn an_older_backup_with_fewer_columns_and_extra_ones_still_goes_in() {
        let (db, dir) = fresh("older").await;
        filled(&db).await;
        let (bytes, _) = export(&db, &[]).await;
        let mut doc: Value = serde_json::from_slice(&bytes).unwrap();
        // As a backup of an earlier panel: a column that is new since is missing,
        // and one that has since been dropped is there.
        let users = &mut doc["tables"]["users"];
        let at = users["columns"]
            .as_array()
            .unwrap()
            .iter()
            .position(|c| c == "hwid_limit")
            .unwrap();
        users["columns"].as_array_mut().unwrap().remove(at);
        for row in users["rows"].as_array_mut().unwrap() {
            row.as_array_mut().unwrap().remove(at);
        }
        users["columns"]
            .as_array_mut()
            .unwrap()
            .push("long_gone".into());
        for row in users["rows"].as_array_mut().unwrap() {
            row.as_array_mut().unwrap().push("x".into());
        }
        doc["tables"]["a_table_since_dropped"] =
            serde_json::json!({"columns": ["a"], "rows": [[1]]});
        doc["schema"] = 3.into();
        let report = db
            .import_json(&serde_json::to_vec(&doc).unwrap()[..], &[], &[])
            .await
            .unwrap();
        assert!(report.tables.iter().any(|(t, n)| t == "users" && *n == 2));
        let ali = db.user_by_token("ali-token").await.unwrap().unwrap();
        assert_eq!(ali.hwid_limit, 0, "what the backup lacks takes its default");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_table_the_backup_lacks_can_be_left_as_it_is() {
        let (db, dir) = fresh("tables").await;
        filled(&db).await;
        let (bytes, _) = export(&db, &["audit_log"]).await;
        db.add_audit(&crate::NewAudit {
            admin_name: "boss".into(),
            action: "user.create".into(),
            via: "session".into(),
            status: 201,
            ..Default::default()
        })
        .await
        .unwrap();
        db.import_json(&bytes[..], &[], &["audit_log"])
            .await
            .unwrap();
        let log = db
            .audit(&crate::AuditQuery {
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            log.len(),
            1,
            "the log is not emptied for lack of one in the backup"
        );
        db.import_json(&bytes[..], &[], &[]).await.unwrap();
        assert!(
            db.audit(&crate::AuditQuery {
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap()
            .is_empty(),
            "unless it was not asked to be kept"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
