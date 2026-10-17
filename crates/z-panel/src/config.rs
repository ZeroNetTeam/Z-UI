//! How the panel is started: flags, or the same names as `ZUI_*` variables.

use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, clap::Parser)]
#[command(name = "z-ui", version, about = "Z-UI, a control panel for zray-core")]
pub struct PanelConfig {
    /// Address and port the panel listens on.
    #[arg(long, env = "ZUI_LISTEN", default_value = "0.0.0.0:2053")]
    pub listen: SocketAddr,

    /// Where the database and the core's files live.
    #[arg(long, env = "ZUI_DATA_DIR", default_value = "data")]
    pub data_dir: PathBuf,

    /// The panel is served only under this path, and answers 404 anywhere
    /// else, so a scanner finding the port does not find the panel.
    #[arg(long, env = "ZUI_BASE_PATH", default_value = "/")]
    pub base_path: String,

    /// `sqlite://…` or `postgres://…`; defaults to `z-ui.db` in the data
    /// directory.
    #[arg(long, env = "ZUI_DATABASE_URL")]
    pub database_url: Option<String>,
}

impl PanelConfig {
    /// Normalise the base path to `/` or `/secret` (leading slash, none
    /// trailing).
    pub fn resolved(mut self) -> anyhow::Result<Self> {
        let trimmed = self.base_path.trim().trim_matches('/');
        if trimmed.contains(|c: char| c.is_whitespace() || c == '?' || c == '#') {
            anyhow::bail!("the base path {:?} is not a plain URL path", self.base_path);
        }
        self.base_path = if trimmed.is_empty() {
            "/".into()
        } else {
            format!("/{trimmed}")
        };
        Ok(self)
    }

    pub fn database_url(&self) -> String {
        self.database_url
            .clone()
            .unwrap_or_else(|| format!("sqlite://{}/z-ui.db", self.data_dir.display()))
    }

    /// `base` joined with `path` (which starts with `/`).
    pub fn under_base(&self, path: &str) -> String {
        if self.base_path == "/" {
            path.to_string()
        } else {
            format!("{}{path}", self.base_path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn with_base(base: &str) -> PanelConfig {
        PanelConfig::parse_from(["z-ui", "--base-path", base])
            .resolved()
            .unwrap()
    }

    #[test]
    fn base_paths_are_normalised() {
        assert_eq!(with_base("/").base_path, "/");
        assert_eq!(with_base("").base_path, "/");
        assert_eq!(with_base("secret/").base_path, "/secret");
        assert_eq!(with_base("/a/b/").base_path, "/a/b");
        assert_eq!(with_base("/secret").under_base("/api"), "/secret/api");
        assert_eq!(with_base("/").under_base("/api"), "/api");
    }

    #[test]
    fn a_base_path_with_a_query_is_refused() {
        assert!(PanelConfig::parse_from(["z-ui", "--base-path", "/a?b"])
            .resolved()
            .is_err());
    }
}
