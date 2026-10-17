//! The data a node reports and takes.

use serde::{Deserialize, Serialize};

/// Whether a node's core is serving, and what with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStatus {
    /// The core is up and every listener bound.
    pub running: bool,
    /// The core's version, e.g. `zray-core 0.1.0`.
    pub core_version: String,
    /// Inbound tags being served.
    pub inbounds: Vec<String>,
    /// Sessions open now, all inbounds together.
    pub active_sessions: u64,
}

/// How a configuration change was applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Applied {
    /// The core started on it.
    Started,
    /// Listeners kept, everything else swapped.
    Reloaded,
    /// Listeners changed: handed over to a new server without a restart.
    Restarted,
}

/// One account's bytes since the last reset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountTraffic {
    pub email: String,
    pub uplink: u64,
    pub downlink: u64,
}

/// An account connected now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnlineAccount {
    pub email: String,
    pub sessions: u64,
    /// Client addresses, when the node keeps them.
    pub addresses: Vec<String>,
}

/// Why a node refused something.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum NodeError {
    #[error("the core is not running")]
    NotRunning,
    #[error("configuration rejected: {0}")]
    Config(String),
    #[error("{0}")]
    Refused(String),
}
