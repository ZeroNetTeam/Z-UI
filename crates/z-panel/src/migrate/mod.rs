//! Moving to this panel from 3X-UI, PasarGuard or Marzban.
//!
//! The other panel's database is opened read-only ([`z_db::Source`]) and read
//! into a [`model::Plan`]; the plan is shown to the admin with every
//! approximation and omission it holds; and only what the admin confirms is
//! written ([`apply`]). Users keep their quota, usage, dates and
//! credentials; the subscription links they were given keep working
//! ([`legacy`] for the signed links of PasarGuard and Marzban, the user's own
//! token for 3X-UI's).

pub mod apply;
mod common;
pub mod legacy;
mod marz;
pub mod model;
mod threexui;

#[cfg(test)]
pub(crate) mod tests;

use serde_json::Value;
use z_db::{DbError, Source};

use model::{Kind, Plan};

/// Which panel `source` is from, if any it knows.
pub async fn detect(source: &Source) -> Result<Option<Kind>, DbError> {
    let tables = source.tables().await?;
    if tables.contains("users") {
        let columns = source.columns("users").await?;
        if columns.iter().any(|c| c == "proxy_settings") {
            return Ok(Some(Kind::Pasarguard));
        }
        if tables.contains("proxies") {
            return Ok(Some(Kind::Marzban));
        }
    }
    if tables.contains("inbounds") {
        let columns = source.columns("inbounds").await?;
        if columns.iter().any(|c| c == "stream_settings") {
            return Ok(Some(Kind::ThreeXUi));
        }
    }
    Ok(None)
}

/// Why a database could not be read.
#[derive(Debug)]
pub enum ReadError {
    /// Not a database, or not one that can be opened.
    Open(DbError),
    /// A database, but none of a panel this one reads.
    Unknown,
    Read(DbError),
}

/// Open `location` (a SQLite file, or a `postgres://` address) and read what
/// it holds. `kind` says which panel it is from, when not to be found out;
/// `xray_config` is Marzban's `xray_config.json`, which holds its inbounds.
pub async fn read(
    location: &str,
    kind: Option<Kind>,
    xray_config: Option<&Value>,
) -> Result<Plan, ReadError> {
    let source = Source::open(location).await.map_err(ReadError::Open)?;
    let kind = match kind {
        Some(kind) => kind,
        None => detect(&source)
            .await
            .map_err(ReadError::Read)?
            .ok_or(ReadError::Unknown)?,
    };
    let plan = match kind {
        Kind::ThreeXUi => threexui::read(&source).await,
        Kind::Pasarguard => marz::read_pasarguard(&source).await,
        Kind::Marzban => marz::read_marzban(&source, xray_config).await,
    };
    plan.map_err(ReadError::Read)
}
