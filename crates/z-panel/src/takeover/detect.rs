//! Finding a panel to take over: the layouts the official installers of
//! 3X-UI (a systemd service), PasarGuard and Marzban (docker compose) leave.
//! Anything else is not guessed at.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use z_db::Source;

use crate::migrate::model::Kind;

/// What stops and starts a panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Service {
    Systemd { unit: String },
    Compose { file: PathBuf },
}

/// Where a panel's subscriptions were served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub host: String,
    pub port: u16,
}

impl Address {
    pub fn text(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// A panel found on the machine. Paths are as the machine knows them (`/etc/x-ui`),
/// whatever root the search was made under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detected {
    pub kind: Kind,
    pub services: Vec<Service>,
    pub database: Option<PathBuf>,
    /// Marzban's Xray configuration, which holds its inbounds.
    pub xray_config: Option<PathBuf>,
    /// What is copied for the way back and then removed.
    pub remove: Vec<PathBuf>,
    pub panel_port: Option<u16>,
    pub sub: Option<Address>,
    /// The certificate the panel served HTTPS with (certificate, key).
    pub cert: Option<(PathBuf, PathBuf)>,
    /// Why it cannot be taken over by this program, when it cannot.
    pub unsupported: Option<String>,
}

/// `path` as it is under `root`.
pub fn at(root: &Path, path: &Path) -> PathBuf {
    root.join(path.strip_prefix("/").unwrap_or(path))
}

fn present(root: &Path, path: &str) -> bool {
    at(root, Path::new(path)).symlink_metadata().is_ok()
}

fn existing(root: &Path, paths: &[&str]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|p| present(root, p))
        .map(PathBuf::from)
        .collect()
}

/// `KEY = "value"` lines of an `.env` file.
pub fn parse_env(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            Some((key.trim().to_string(), value.to_string()))
        })
        .collect()
}

/// The file a SQLAlchemy SQLite URL names (`sqlite:////abs.db` is absolute,
/// `sqlite:///rel.db` is under `data`), or why the database is not one this
/// program reads.
pub fn sqlite_path(url: &str, data: &Path) -> Result<PathBuf, String> {
    let url = url.trim();
    if !url.starts_with("sqlite") {
        let scheme = url.split(['+', ':']).next().unwrap_or("another");
        return Err(format!(
            "its database is {scheme}, not a SQLite file; take its users over with `z-ui migrate` once they are exported to SQLite or PostgreSQL"
        ));
    }
    let after = url.split_once("://").map(|(_, rest)| rest).ok_or_else(|| {
        format!("its database address {url:?} is not one this program understands")
    })?;
    let path = after.strip_prefix('/').unwrap_or(after);
    let path = path.split('?').next().unwrap_or(path);
    if path.is_empty() || path == ":memory:" {
        return Err("its database is not kept in a file".into());
    }
    Ok(if path.starts_with('/') {
        PathBuf::from(path)
    } else {
        data.join(path)
    })
}

fn three_x_ui(root: &Path) -> Option<Detected> {
    let units = [
        "/etc/systemd/system/x-ui.service",
        "/usr/lib/systemd/system/x-ui.service",
    ];
    let dirs = ["/usr/local/x-ui", "/etc/x-ui"];
    if !units.iter().chain(dirs.iter()).any(|p| present(root, p)) {
        return None;
    }
    let database = ["/etc/x-ui/x-ui.db", "/usr/local/x-ui/x-ui.db"]
        .iter()
        .find(|p| at(root, Path::new(p)).is_file())
        .map(PathBuf::from);
    let remove = existing(
        root,
        &[
            "/usr/local/x-ui",
            "/etc/x-ui",
            "/var/log/x-ui",
            "/usr/bin/x-ui",
            units[0],
            units[1],
        ],
    );
    Some(Detected {
        kind: Kind::ThreeXUi,
        services: vec![Service::Systemd {
            unit: "x-ui".into(),
        }],
        unsupported: database
            .is_none()
            .then(|| "its database file (x-ui.db) was not found".to_string()),
        database,
        xray_config: None,
        remove,
        panel_port: None,
        sub: None,
        cert: None,
    })
}

fn compose_panel(root: &Path, kind: Kind, app: &str, data: &str, cli: &str) -> Option<Detected> {
    let compose = [
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yaml",
        "compose.yml",
    ]
    .iter()
    .map(|name| format!("{app}/{name}"))
    .find(|p| present(root, p))?;
    let env = std::fs::read_to_string(at(root, Path::new(&format!("{app}/.env"))))
        .map(|text| parse_env(&text))
        .unwrap_or_default();
    let data_dir = Path::new(data);
    let (database, unsupported) = match env.get("SQLALCHEMY_DATABASE_URL") {
        Some(url) => match sqlite_path(url, data_dir) {
            Ok(path) => (Some(path), None),
            Err(why) => (None, Some(why)),
        },
        None => (Some(data_dir.join("db.sqlite3")), None),
    };
    // Replacing is for the two panels most used here; Marzban's data can still
    // be imported from the Migration page.
    let unsupported = unsupported
        .or_else(|| {
            (kind == Kind::Marzban).then(|| {
                "replacing is supported for 3X-UI and PasarGuard only; Marzban's users can be imported from the panel's Migration page".to_string()
            })
        })
        .or_else(|| {
        database
            .as_ref()
            .filter(|p| !at(root, p).is_file())
            .map(|p| format!("its database file {} was not found", p.display()))
    });
    let port = env
        .get("UVICORN_PORT")
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8000);
    let cert = match (
        env.get("UVICORN_SSL_CERTFILE"),
        env.get("UVICORN_SSL_KEYFILE"),
    ) {
        (Some(c), Some(k)) if !c.is_empty() && !k.is_empty() => {
            Some((PathBuf::from(c), PathBuf::from(k)))
        }
        _ => None,
    };
    let xray_config = (kind == Kind::Marzban).then(|| {
        PathBuf::from(
            env.get("XRAY_JSON")
                .cloned()
                .unwrap_or_else(|| format!("{data}/xray_config.json")),
        )
    });
    let bin = format!("/usr/local/bin/{cli}");
    Some(Detected {
        kind,
        services: vec![Service::Compose {
            file: PathBuf::from(compose),
        }],
        database,
        xray_config,
        remove: existing(root, &[app, data, &bin]),
        panel_port: Some(port),
        sub: Some(Address {
            host: env
                .get("UVICORN_HOST")
                .filter(|h| !h.is_empty())
                .cloned()
                .unwrap_or_else(|| "0.0.0.0".into()),
            port,
        }),
        cert,
        unsupported,
    })
}

/// Every panel this machine has that a takeover knows how to stop and start,
/// from what is on the disk under `root`.
pub fn detect(root: &Path) -> Vec<Detected> {
    let mut found = Vec::new();
    found.extend(three_x_ui(root));
    found.extend(compose_panel(
        root,
        Kind::Pasarguard,
        "/opt/pasarguard",
        "/var/lib/pasarguard",
        "pasarguard",
    ));
    found.extend(compose_panel(
        root,
        Kind::Marzban,
        "/opt/marzban",
        "/var/lib/marzban",
        "marzban",
    ));
    found
}

/// What only 3X-UI's database says: where it listens and with which
/// certificate it serves.
pub async fn inspect(root: &Path, detected: &mut Detected) {
    if detected.kind != Kind::ThreeXUi {
        return;
    }
    let Some(database) = detected.database.clone() else {
        return;
    };
    let Ok(source) = Source::open(&at(root, &database).to_string_lossy()).await else {
        return;
    };
    let Ok(rows) = source.rows("settings").await else {
        return;
    };
    let settings: HashMap<String, String> = rows
        .iter()
        .filter_map(|r| {
            Some((
                r.filled("key")?.to_string(),
                r.text("value")?.trim().to_string(),
            ))
        })
        .collect();
    let port = |key: &str, default: u16| {
        settings
            .get(key)
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(default)
    };
    detected.panel_port = Some(port("webPort", 2053));
    if settings.get("subEnable").is_none_or(|v| v != "false") {
        detected.sub = Some(Address {
            host: settings
                .get("subListen")
                .filter(|h| !h.is_empty())
                .cloned()
                .unwrap_or_else(|| "0.0.0.0".into()),
            port: port("subPort", 2096),
        });
    }
    let pair = |cert: &str, key: &str| match (settings.get(cert), settings.get(key)) {
        (Some(c), Some(k)) if !c.is_empty() && !k.is_empty() => {
            let (c, k) = (PathBuf::from(c), PathBuf::from(k));
            (at(root, &c).is_file() && at(root, &k).is_file()).then_some((c, k))
        }
        _ => None,
    };
    detected.cert = pair("webCertFile", "webKeyFile").or_else(|| pair("subCertFile", "subKeyFile"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_files_are_read_with_or_without_quotes_and_comments() {
        let env = parse_env("# a comment\nA = \"1\"\nB='two'\n\nC=3\n  D = x y \n");
        assert_eq!(env["A"], "1");
        assert_eq!(env["B"], "two");
        assert_eq!(env["C"], "3");
        assert_eq!(env["D"], "x y");
        assert!(!env.contains_key("# a comment"));
    }

    #[test]
    fn a_sqlalchemy_sqlite_address_is_read_and_any_other_database_is_refused() {
        let data = Path::new("/var/lib/pasarguard");
        assert_eq!(
            sqlite_path("sqlite+aiosqlite:////var/lib/pasarguard/db.sqlite3", data).unwrap(),
            Path::new("/var/lib/pasarguard/db.sqlite3")
        );
        assert_eq!(
            sqlite_path("sqlite:///db.sqlite3", data).unwrap(),
            Path::new("/var/lib/pasarguard/db.sqlite3")
        );
        assert_eq!(
            sqlite_path("sqlite:////abs/x.db?mode=rwc", data).unwrap(),
            Path::new("/abs/x.db")
        );
        for other in [
            "mysql+asyncmy://u:p@db/pasarguard",
            "postgresql+asyncpg://u:p@db/pg",
            "sqlite:///:memory:",
            "sqlite:",
        ] {
            assert!(sqlite_path(other, data).is_err(), "{other}");
        }
        assert!(sqlite_path("mysql://u@h/d", data)
            .unwrap_err()
            .contains("mysql"));
    }
}
