//! `z-node` — the core a panel drives.
//!
//! The panel never touches zray-core directly: it talks to a [`Node`]. In the
//! beta there is one, [`LocalNode`], which runs the core inside the panel's
//! own process. A node on another server (v1.0) implements the same trait
//! over the network, and nothing above it changes.

mod local;

use async_trait::async_trait;
use z_common::node::{AccountTraffic, Applied, NodeError, NodeStatus, OnlineAccount};

pub use local::LocalNode;

#[async_trait]
pub trait Node: Send + Sync {
    /// Whether the core is serving, and what.
    async fn status(&self) -> NodeStatus;

    /// Serve `config` (an Xray-shaped JSON configuration) from now on:
    /// start the core, reload it, or hand over to new listeners, whichever
    /// the change needs.
    async fn apply(&self, config: serde_json::Value) -> Result<Applied, NodeError>;

    /// Add accounts to an inbound without a reload, each in the shape of an
    /// entry of the inbound's `settings.clients`.
    async fn add_accounts(
        &self,
        inbound: &str,
        clients: &[serde_json::Value],
    ) -> Result<(), NodeError>;

    /// Remove accounts from an inbound and end their sessions there.
    async fn remove_accounts(&self, inbound: &str, emails: &[String]) -> Result<usize, NodeError>;

    /// Bytes per account; `reset` zeroes the counters as they are read.
    async fn traffic(&self, reset: bool) -> Vec<AccountTraffic>;

    /// Accounts connected now.
    async fn online(&self) -> Vec<OnlineAccount>;

    /// End every open session of an account. Returns how many.
    async fn end_sessions(&self, email: &str) -> usize;

    /// End an account's sessions from one client address. Returns how many.
    async fn end_address(&self, email: &str, address: std::net::IpAddr) -> usize;

    /// Hold an account to these bytes per second (0 = no limit).
    async fn set_limit(&self, email: &str, uplink: u64, downlink: u64);

    /// Stop serving, waiting briefly for open sessions.
    async fn stop(&self);
}
