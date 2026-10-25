//! Reading the database of another panel (3X-UI, Marzban, PasarGuard), which
//! this panel only looks at. Every column is asked for as text, whichever
//! database it is (a SQLite file, or a PostgreSQL server), so a type the
//! driver has no name for (`timestamptz`) is no obstacle and one reader does
//! for them all; the numbers, flags, times and JSON are read out of the text
//! afterwards, forgivingly: a column an older version of the panel did not
//! have is simply absent.

use std::collections::{HashMap, HashSet};

use sqlx::any::AnyPoolOptions;
use sqlx::{AnyPool, AssertSqlSafe, Row};

use crate::DbError;

pub struct Source {
    pool: AnyPool,
    postgres: bool,
}

/// `\x20`, `?` and the like, made safe to put in a `sqlite://` path.
pub(crate) fn encode_path(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

impl Source {
    /// A SQLite file by its path (opened for reading only: nothing in it is
    /// ever changed), or a `postgres://…` URL.
    pub async fn open(location: &str) -> Result<Source, DbError> {
        sqlx::any::install_default_drivers();
        let postgres = location.starts_with("postgres:") || location.starts_with("postgresql:");
        let url = if postgres {
            location.to_string()
        } else {
            let path = std::path::Path::new(location.strip_prefix("sqlite://").unwrap_or(location));
            if !path.is_file() {
                return Err(DbError::Backup(format!(
                    "there is no file {}",
                    path.display()
                )));
            }
            let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            format!(
                "sqlite://{}?mode=ro",
                encode_path(&absolute.to_string_lossy())
            )
        };
        let pool = AnyPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await?;
        // A file that is not a database fails here, not at the first table.
        sqlx::query("SELECT 1").fetch_one(&pool).await?;
        Ok(Source { pool, postgres })
    }

    /// A consistent copy of this SQLite database in a new file, made while
    /// the panel that owns it may still be writing to it (`VACUUM INTO` reads
    /// one snapshot, and works on a file opened read only).
    pub async fn snapshot(&self, destination: &std::path::Path) -> Result<(), DbError> {
        if self.postgres {
            return Err(DbError::Backup(
                "only a SQLite file can be copied this way".into(),
            ));
        }
        if destination.exists() {
            return Err(DbError::Backup(format!(
                "{} is already there",
                destination.display()
            )));
        }
        let literal = destination.to_string_lossy().replace('\'', "''");
        sqlx::query(AssertSqlSafe(format!("VACUUM INTO '{literal}'")))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn tables(&self) -> Result<HashSet<String>, DbError> {
        let sql = if self.postgres {
            "SELECT CAST(table_name AS TEXT) FROM information_schema.tables WHERE table_schema = current_schema()"
        } else {
            "SELECT CAST(name AS TEXT) FROM sqlite_master WHERE type = 'table'"
        };
        let rows = sqlx::query(sql).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|r| Ok(r.try_get::<String, _>(0)?))
            .collect()
    }

    /// The columns of `table`, in order; empty if there is no such table.
    pub async fn columns(&self, table: &str) -> Result<Vec<String>, DbError> {
        let rows = if self.postgres {
            sqlx::query(
                "SELECT CAST(column_name AS TEXT) FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
            )
            .bind(table)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query("SELECT CAST(name AS TEXT) FROM pragma_table_info($1) ORDER BY cid")
                .bind(table)
                .fetch_all(&self.pool)
                .await?
        };
        rows.iter()
            .map(|r| Ok(r.try_get::<String, _>(0)?))
            .collect()
    }

    /// Every row of `table`.
    pub async fn rows(&self, table: &str) -> Result<Vec<Record>, DbError> {
        let columns = self.columns(table).await?;
        if columns.is_empty() {
            return Ok(Vec::new());
        }
        let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
        let list = columns
            .iter()
            .map(|c| format!("CAST({0} AS TEXT) AS {0}", quote(c)))
            .collect::<Vec<_>>()
            .join(", ");
        // The names come from the database's own catalogue, and are quoted.
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT {list} FROM {}",
            quote(table)
        )))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let mut cells = HashMap::with_capacity(columns.len());
                for (i, name) in columns.iter().enumerate() {
                    cells.insert(name.clone(), row.try_get::<Option<String>, _>(i)?);
                }
                Ok(Record { cells })
            })
            .collect()
    }

    /// Rows of a query the importer wrote (it takes nothing from the file or
    /// from a person) whose columns are all text.
    pub async fn query(&self, sql: &str) -> Result<Vec<Vec<Option<String>>>, DbError> {
        let rows = sqlx::query(AssertSqlSafe(sql.to_string()))
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                (0..row.len())
                    .map(|i| Ok(row.try_get::<Option<String>, _>(i)?))
                    .collect()
            })
            .collect()
    }

    pub fn is_postgres(&self) -> bool {
        self.postgres
    }
}

/// A row, every cell of it text or nothing.
#[derive(Debug, Clone, Default)]
pub struct Record {
    cells: HashMap<String, Option<String>>,
}

/// Seconds since the epoch for a civil date and time (proleptic Gregorian).
fn epoch_seconds(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
    // Howard Hinnant's days_from_civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + hour * 3600 + minute * 60 + second
}

/// A date and time as a database writes it (`2026-10-23 12:34:56.789012`,
/// `2026-10-23T12:34:56Z`, `…+00`, `…+03:30`), as Unix seconds. Without a zone
/// it is UTC, which is how the panels store theirs.
pub fn parse_datetime(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, rest) = text.split_once([' ', 'T']).unwrap_or((text, ""));
    let mut d = date.split('-');
    let (year, month, day) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut seconds = 0;
    let mut offset = 0;
    if !rest.is_empty() {
        // Split the clock from a zone: `Z`, `+hh`, `+hh:mm`, `-hh`.
        let split = rest.find(['Z', 'z', '+', '-']).unwrap_or(rest.len());
        let (clock, zone) = rest.split_at(split);
        let clock = clock.split('.').next().unwrap_or("");
        let mut c = clock.split(':');
        let hour: i64 = c.next().filter(|s| !s.is_empty())?.parse().ok()?;
        let minute: i64 = c.next().map_or(Some(0), |s| s.parse().ok())?;
        let second: i64 = c.next().map_or(Some(0), |s| s.parse().ok())?;
        seconds = hour * 3600 + minute * 60 + second;
        if let Some(sign) = zone.chars().next().filter(|c| matches!(c, '+' | '-')) {
            let mut z = zone[1..].split(':');
            let h: i64 = z.next()?.parse().ok()?;
            let m: i64 = z.next().map_or(Some(0), |s| s.parse().ok())?;
            offset = (h * 3600 + m * 60) * if sign == '-' { -1 } else { 1 };
        }
    }
    Some(epoch_seconds(year, month, day, 0, 0, 0) + seconds - offset)
}

impl Record {
    #[cfg(test)]
    pub fn of(pairs: &[(&str, Option<&str>)]) -> Self {
        Self {
            cells: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.map(str::to_string)))
                .collect(),
        }
    }

    /// Whether the table has this column at all.
    pub fn has(&self, column: &str) -> bool {
        self.cells.contains_key(column)
    }

    /// The text, or nothing for NULL and for a column that is not there.
    pub fn text(&self, column: &str) -> Option<&str> {
        self.cells.get(column)?.as_deref()
    }

    /// The text, unless it is empty.
    pub fn filled(&self, column: &str) -> Option<&str> {
        self.text(column).map(str::trim).filter(|t| !t.is_empty())
    }

    /// A whole number, read from `12`, `12.0`, `1.2e9` or a flag's `true`.
    pub fn int(&self, column: &str) -> Option<i64> {
        let text = self.filled(column)?;
        text.parse::<i64>()
            .ok()
            .or_else(|| {
                text.parse::<f64>()
                    .ok()
                    .filter(|f| f.is_finite())
                    .map(|f| f as i64)
            })
            .or_else(|| self.flag(column).map(i64::from))
    }

    /// A flag, however a database writes one.
    pub fn flag(&self, column: &str) -> Option<bool> {
        match self.filled(column)?.to_ascii_lowercase().as_str() {
            "1" | "true" | "t" | "yes" | "y" => Some(true),
            "0" | "false" | "f" | "no" | "n" => Some(false),
            other => other.parse::<f64>().ok().map(|f| f != 0.0),
        }
    }

    /// A time as Unix seconds: a date and time, or a whole number (of seconds,
    /// or of milliseconds when it is too big for seconds).
    pub fn time(&self, column: &str) -> Option<i64> {
        let text = self.filled(column)?;
        if let Ok(n) = text.parse::<i64>() {
            return Some(if n > 100_000_000_000 { n / 1000 } else { n });
        }
        parse_datetime(text)
    }

    /// JSON kept as text.
    pub fn json(&self, column: &str) -> Option<serde_json::Value> {
        serde_json::from_str(self.filled(column)?).ok()
    }

    /// A list kept as comma-separated text, empty items dropped.
    pub fn list(&self, column: &str) -> Vec<String> {
        self.text(column)
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_read_as_the_databases_write_them() {
        let want = 1_792_758_896; // 2026-10-23 12:34:56 UTC
        for text in [
            "2026-10-23 12:34:56",
            "2026-10-23 12:34:56.789012",
            "2026-10-23T12:34:56Z",
            "2026-10-23T12:34:56+00:00",
            "2026-10-23 12:34:56+00",
            "2026-10-23 16:04:56+03:30",
            "2026-10-23 08:34:56-04",
        ] {
            assert_eq!(parse_datetime(text), Some(want), "{text}");
        }
        assert_eq!(parse_datetime("1970-01-01 00:00:00"), Some(0));
        assert_eq!(parse_datetime("2000-02-29"), Some(951_782_400));
        assert_eq!(parse_datetime("1969-12-31 23:59:59"), Some(-1));
        for bad in [
            "",
            "not a date",
            "2026-13-01",
            "2026-10",
            "2026-10-23 xx:00",
        ] {
            assert_eq!(parse_datetime(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_cell_is_read_however_it_is_written_and_a_missing_column_is_nothing() {
        let row = Record::of(&[
            ("n", Some("12")),
            ("f", Some("12.0")),
            ("big", Some("1.5e3")),
            ("yes", Some("true")),
            ("zero", Some("0")),
            ("empty", Some("  ")),
            ("null", None),
            ("ms", Some("1792758896000")),
            ("s", Some("1792758896")),
            ("dt", Some("2026-10-23 12:34:56.5")),
            ("json", Some("{\"a\": [1, 2]}")),
            ("list", Some("a, b,,c")),
        ]);
        assert_eq!(
            (row.int("n"), row.int("f"), row.int("big")),
            (Some(12), Some(12), Some(1500))
        );
        assert_eq!(
            (row.flag("yes"), row.flag("zero"), row.flag("n")),
            (Some(true), Some(false), Some(true))
        );
        assert_eq!(row.int("yes"), Some(1));
        assert_eq!(
            (row.int("empty"), row.int("null"), row.int("absent")),
            (None, None, None)
        );
        assert!(row.has("null") && !row.has("absent"));
        assert_eq!(
            (row.time("ms"), row.time("s"), row.time("dt")),
            (
                Some(1_792_758_896),
                Some(1_792_758_896),
                Some(1_792_758_896)
            )
        );
        assert_eq!(row.json("json").unwrap()["a"][1], 2);
        assert_eq!(row.list("list"), ["a", "b", "c"]);
        assert_eq!(row.list("absent"), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_sqlite_file_is_read_whatever_the_types_in_it() {
        let dir = std::env::temp_dir().join(format!(
            "z-db-source-{}-{}",
            std::process::id(),
            crate::now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old panel.db");
        {
            let db = crate::Database::open(&format!("sqlite://{}", path.display()))
                .await
                .unwrap();
            sqlx::query("CREATE TABLE things (id INTEGER PRIMARY KEY, name TEXT, price REAL, ok BOOLEAN, at DATETIME, data JSON, blank TEXT)")
                .execute(db.pool())
                .await
                .unwrap();
            sqlx::query("INSERT INTO things VALUES (1, 'a b', 1.5, 1, '2026-10-23 12:34:56.000000', '{\"k\": 1}', NULL)")
                .execute(db.pool())
                .await
                .unwrap();
        }
        let source = Source::open(path.to_str().unwrap()).await.unwrap();
        assert!(source.tables().await.unwrap().contains("things"));
        assert_eq!(
            source.columns("things").await.unwrap()[..3],
            ["id", "name", "price"]
        );
        assert!(source.columns("missing").await.unwrap().is_empty());
        let rows = source.rows("things").await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            (row.int("id"), row.text("name"), row.flag("ok")),
            (Some(1), Some("a b"), Some(true))
        );
        assert_eq!(row.time("at"), Some(1_792_758_896));
        assert_eq!(row.json("data").unwrap()["k"], 1);
        assert!(row.has("blank") && row.text("blank").is_none());
        assert!(source.rows("missing").await.unwrap().is_empty());
        assert_eq!(
            source.query("SELECT 'x', CAST(2 AS TEXT)").await.unwrap(),
            [[Some("x".into()), Some("2".into())]]
        );

        // A copy of it, made without writing to it, is a database of its own.
        let copy = dir.join("copy.db");
        source.snapshot(&copy).await.unwrap();
        let copied = Source::open(copy.to_str().unwrap()).await.unwrap();
        assert_eq!(copied.rows("things").await.unwrap().len(), 1);
        assert!(source.snapshot(&copy).await.is_err(), "no copy over a copy");

        // Read only: the old panel's file is never changed.
        let write = sqlx::query("DELETE FROM things")
            .execute(&source.pool)
            .await;
        assert!(write.is_err(), "the source is opened read only");

        assert!(Source::open(dir.join("nothing.db").to_str().unwrap())
            .await
            .is_err());
        std::fs::write(
            dir.join("not-a-db"),
            b"this is not a database at all, just text, long enough to be tried",
        )
        .unwrap();
        assert!(Source::open(dir.join("not-a-db").to_str().unwrap())
            .await
            .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
