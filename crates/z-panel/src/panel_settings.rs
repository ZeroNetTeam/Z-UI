//! Where the panel listens, under which path and with which certificate, as
//! an admin set it from the panel or `z-ui setting`. A flag or an
//! environment variable given at start wins over what is stored here (see
//! [`crate::config::Locked`]): whoever wrote it there meant it.

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

pub const KEY: &str = "panel.settings";

/// What is stored: every field is an override, and empty means "as the flags
/// and defaults say".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stored {
    pub listen: Option<SocketAddr>,
    pub base_path: Option<String>,
    /// A certificate of the store the panel serves HTTPS with.
    pub certificate_id: Option<i64>,
}

pub async fn load(db: &z_db::Database) -> Stored {
    match db.setting(KEY).await {
        Ok(Some(text)) => serde_json::from_str(&text).unwrap_or_else(|error| {
            tracing::warn!(%error, "the stored panel settings are unreadable and were ignored");
            Stored::default()
        }),
        _ => Stored::default(),
    }
}

pub async fn save(db: &z_db::Database, stored: &Stored) -> Result<(), z_db::DbError> {
    if *stored == Stored::default() {
        // Nothing to override: no document at all, as on a fresh panel.
        db.set_setting(KEY, "{}").await
    } else {
        db.set_setting(KEY, &serde_json::to_string(stored).unwrap_or_default())
            .await
    }
}

/// `/` or `/secret` (leading slash, none trailing), or why the text is not a
/// path the panel can live under.
pub fn normalize_base_path(text: &str) -> Result<String, String> {
    let trimmed = text.trim().trim_matches('/');
    if trimmed.contains(|c: char| c.is_whitespace() || matches!(c, '?' | '#' | '\\')) {
        return Err(format!("the base path {text:?} is not a plain URL path"));
    }
    if trimmed.split('/').any(|part| part == "." || part == "..") {
        return Err(format!("the base path {text:?} has a . or .. part"));
    }
    Ok(if trimmed.is_empty() {
        "/".into()
    } else {
        format!("/{trimmed}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_paths_are_normalised_and_odd_ones_refused() {
        assert_eq!(normalize_base_path("").unwrap(), "/");
        assert_eq!(normalize_base_path(" / ").unwrap(), "/");
        assert_eq!(normalize_base_path("secret/").unwrap(), "/secret");
        assert_eq!(normalize_base_path("/a/b/").unwrap(), "/a/b");
        for bad in ["/a?b", "/a#b", "/a b", "/a/../b", "/./a", "/a\\b"] {
            assert!(normalize_base_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn stored_settings_survive_a_round_trip_and_unknown_fields_are_ignored() {
        let text = r#"{"listen":"127.0.0.1:8443","base_path":"/x","certificate_id":4,"later":1}"#;
        let stored: Stored = serde_json::from_str(text).unwrap();
        assert_eq!(stored.listen, Some("127.0.0.1:8443".parse().unwrap()));
        assert_eq!(stored.base_path.as_deref(), Some("/x"));
        assert_eq!(stored.certificate_id, Some(4));
        let again: Stored = serde_json::from_str(&serde_json::to_string(&stored).unwrap()).unwrap();
        assert_eq!(again, stored);
        assert_eq!(
            serde_json::from_str::<Stored>("{}").unwrap(),
            Stored::default()
        );
    }
}
