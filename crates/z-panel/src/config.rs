//! How the panel is started: flags, or the same names as `ZUI_*` variables.

use std::net::SocketAddr;
use std::path::PathBuf;

/// What a flag or an environment variable fixed at start: the panel does not
/// let an admin change those from its pages, and what is stored for them
/// stays unused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Locked {
    pub listen: bool,
    pub base_path: bool,
    pub certificate: bool,
}

#[derive(Debug, Clone, clap::Parser)]
#[command(name = "z-ui", version, about = "Z-UI, a control panel for zray-core")]
pub struct PanelConfig {
    /// Address and port the panel listens on.
    #[arg(long, env = "ZUI_LISTEN", default_value = "0.0.0.0:2053")]
    pub listen: SocketAddr,

    /// Where the database and the core's files live.
    #[arg(long, env = "ZUI_DATA_DIR", default_value = "data", global = true)]
    pub data_dir: PathBuf,

    /// The panel is served only under this path, and answers 404 anywhere
    /// else, so a scanner finding the port does not find the panel.
    #[arg(long, env = "ZUI_BASE_PATH", default_value = "/")]
    pub base_path: String,

    /// `sqlite://…` or `postgres://…`; defaults to `z-ui.db` in the data
    /// directory.
    #[arg(long, env = "ZUI_DATABASE_URL", global = true)]
    pub database_url: Option<String>,

    /// Serve HTTPS with this certificate chain (PEM); needs `--key`.
    #[arg(long, env = "ZUI_CERT_FILE", requires = "key")]
    pub cert: Option<PathBuf>,

    /// The private key (PEM) for `--cert`.
    #[arg(long, env = "ZUI_KEY_FILE", requires = "cert")]
    pub key: Option<PathBuf>,

    /// Also serve subscriptions, and only them, on this address: for a
    /// subscription domain or port apart from the panel's.
    #[arg(long, env = "ZUI_SUB_LISTEN")]
    pub sub_listen: Option<SocketAddr>,

    #[command(subcommand)]
    pub command: Option<Command>,

    /// Which of the above were given rather than defaulted.
    #[arg(skip)]
    pub locked: Locked,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Command {
    /// Manage admins from the server, for when nobody can sign in.
    #[command(subcommand)]
    Admin(AdminCommand),
    /// Look at or change where the panel listens, under which path and with
    /// which certificate. Changes apply when the panel next starts.
    #[command(subcommand)]
    Setting(SettingCommand),
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum SettingCommand {
    /// Show what the panel uses, and where each part comes from.
    Show,
    /// Store new values.
    Set {
        /// The address and port to listen on, `0.0.0.0:2053`.
        #[arg(long, conflicts_with = "port")]
        listen: Option<SocketAddr>,
        /// Only the port, keeping the address.
        #[arg(long)]
        port: Option<u16>,
        /// The path the panel is served under (`/` for none).
        #[arg(long)]
        path: Option<String>,
        /// The id of a certificate in the store to serve HTTPS with, or
        /// `none` for plain HTTP.
        #[arg(long)]
        certificate: Option<String>,
    },
    /// Forget everything stored: the flags and defaults apply again. For a
    /// panel that cannot be reached any more.
    Reset,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum AdminCommand {
    /// Set a new password for an admin (a random one is printed if none is
    /// given) and end their sessions.
    ResetPassword {
        username: String,
        #[arg(long, env = "ZUI_NEW_PASSWORD")]
        password: Option<String>,
        /// Also turn two-factor sign-in off, for a lost authenticator.
        #[arg(long)]
        disable_2fa: bool,
    },
}

impl PanelConfig {
    /// The command line and environment, with which settings they fixed.
    pub fn from_args() -> Self {
        Self::from_args_iter(std::env::args_os())
    }

    pub fn from_args_iter<I, T>(args: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        use clap::{CommandFactory, FromArgMatches};
        let matches = Self::command().get_matches_from(args);
        let mut config = Self::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
        let given = |id: &str| {
            matches
                .value_source(id)
                .is_some_and(|source| source != clap::parser::ValueSource::DefaultValue)
        };
        config.locked = Locked {
            listen: given("listen"),
            base_path: given("base_path"),
            certificate: given("cert") || given("key"),
        };
        config
    }

    /// What an admin stored, for the settings no flag or variable fixed.
    pub fn apply_stored(&mut self, stored: &crate::panel_settings::Stored) {
        if let (false, Some(listen)) = (self.locked.listen, stored.listen) {
            self.listen = listen;
        }
        if let (false, Some(path)) = (self.locked.base_path, &stored.base_path) {
            // What is stored was checked on the way in; were it not, it must
            // not keep the panel from starting, or from being reset.
            match crate::panel_settings::normalize_base_path(path) {
                Ok(path) => self.base_path = path,
                Err(error) => tracing::warn!(%error, "the stored panel path was ignored"),
            }
        }
    }

    /// Normalise the base path to `/` or `/secret` (leading slash, none
    /// trailing).
    pub fn resolved(mut self) -> anyhow::Result<Self> {
        self.base_path = crate::panel_settings::normalize_base_path(&self.base_path)
            .map_err(anyhow::Error::msg)?;
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
    fn what_a_flag_or_variable_fixed_is_told_apart_from_a_default() {
        let none = PanelConfig::from_args_iter(["z-ui"]);
        assert_eq!(none.locked, Locked::default());
        let some =
            PanelConfig::from_args_iter(["z-ui", "--listen", "127.0.0.1:9", "--base-path", "/p"]);
        assert!(some.locked.listen && some.locked.base_path && !some.locked.certificate);
    }

    #[test]
    fn stored_settings_fill_in_only_what_was_not_fixed() {
        let stored = crate::panel_settings::Stored {
            listen: Some("0.0.0.0:4444".parse().unwrap()),
            base_path: Some("/stored".into()),
            certificate_id: None,
        };
        let mut free = PanelConfig::from_args_iter(["z-ui"]);
        free.apply_stored(&stored);
        assert_eq!(free.listen.port(), 4444);
        assert_eq!(free.base_path, "/stored");

        let mut fixed = PanelConfig::from_args_iter(["z-ui", "--listen", "127.0.0.1:9"]);
        fixed.apply_stored(&stored);
        assert_eq!(fixed.listen.port(), 9, "a flag beats what is stored");
        assert_eq!(fixed.base_path, "/stored");
    }

    #[test]
    fn a_base_path_with_a_query_is_refused() {
        assert!(PanelConfig::parse_from(["z-ui", "--base-path", "/a?b"])
            .resolved()
            .is_err());
    }
}
