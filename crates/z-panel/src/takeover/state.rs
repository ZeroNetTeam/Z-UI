//! Where a takeover has got to, kept in the data directory so a command run
//! later (the way back, from the server or from the panel) knows what was
//! done and what is kept.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::detect::Detected;

/// How long the old panel's files are kept in place, and the way back open.
pub const WINDOW: i64 = 7 * 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Read and imported; the old panel still runs.
    Prepared,
    /// This panel took over; the old one is stopped, its files in place.
    Switched,
    /// Back to the old panel.
    RolledBack,
    /// The old panel's files were deleted, so there is no way back.
    Finished,
}

/// A user brought over, to give back what it used here if the old panel is
/// brought back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovedUser {
    /// The name it has here.
    pub username: String,
    /// The name it had there (an email in 3X-UI).
    pub source_name: String,
    /// Its lifetime traffic here when this panel took over; what it grew by
    /// since is what it used here.
    pub lifetime: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub phase: Phase,
    pub detected: Detected,
    pub prepared_at: i64,
    pub switched_at: Option<i64>,
    /// When the old panel's files are deleted by themselves.
    pub expires_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub snapshot: PathBuf,
    /// Users this takeover made here (by name).
    pub created_users: Vec<String>,
    pub moved: Vec<MovedUser>,
    /// Tags of the inbounds switched on at the cutover.
    pub enable_tags: Vec<String>,
    /// Ports of those inbounds: (port, udp).
    pub inbound_ports: Vec<(u16, bool)>,
    /// `ZUI_SUB_LISTEN` as written at the cutover.
    pub sub_listen: Option<String>,
    pub certificate_id: Option<i64>,
    /// Whether the day-before notice was sent.
    pub notified: bool,
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("takeover")
}

fn file(data_dir: &Path) -> PathBuf {
    dir(data_dir).join("state.json")
}

pub fn load(data_dir: &Path) -> Option<State> {
    let text = std::fs::read_to_string(file(data_dir)).ok()?;
    match serde_json::from_str(&text) {
        Ok(state) => Some(state),
        Err(error) => {
            tracing::warn!(%error, "the takeover state is unreadable and was ignored");
            None
        }
    }
}

pub fn save(data_dir: &Path, state: &State) -> std::io::Result<()> {
    let folder = dir(data_dir);
    std::fs::create_dir_all(&folder)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700))?;
    }
    let temp = folder.join("state.json.part");
    std::fs::write(&temp, serde_json::to_vec_pretty(state).unwrap_or_default())?;
    std::fs::rename(&temp, file(data_dir))
}
