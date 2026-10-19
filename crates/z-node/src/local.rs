//! The core inside the panel's process.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex;
use z_common::node::{AccountTraffic, Applied, NodeError, NodeStatus, OnlineAccount};
use zero_runtime::host::Host;
use zero_runtime::server::Stats;

use crate::Node;

/// How long [`Node::stop`] lets open sessions finish.
const STOP_DRAIN: Duration = Duration::from_secs(5);

/// A node whose core runs in this process.
///
/// The figures (traffic, limits, events) live in one [`Stats`] that outlives
/// every start, stop and handover of the core, so nothing the panel reads
/// resets when the configuration changes.
pub struct LocalNode {
    stats: Arc<Stats>,
    host: Mutex<Option<Arc<Host>>>,
    generation: AtomicU64,
}

impl Default for LocalNode {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalNode {
    /// A node with no core running yet.
    pub fn new() -> Self {
        let stats = Arc::new(Stats::default());
        // The panel enforces per-account address limits, so it needs them.
        stats.accounts.set_track_addresses(true);
        Self {
            stats,
            host: Mutex::new(None),
            generation: AtomicU64::new(0),
        }
    }

    /// The core's live events, for the panel's log.
    pub fn events(&self) -> tokio::sync::broadcast::Receiver<zero_runtime::events::Record> {
        self.stats.events().subscribe()
    }

    async fn host(&self) -> Result<Arc<Host>, NodeError> {
        self.host.lock().await.clone().ok_or(NodeError::NotRunning)
    }
}

#[async_trait]
impl Node for LocalNode {
    async fn status(&self) -> NodeStatus {
        let host = self.host.lock().await.clone();
        let (running, inbounds) = match &host {
            Some(host) => {
                let server = host.server();
                let inbounds = server
                    .config_snapshot()
                    .inbounds
                    .iter()
                    .map(|inbound| inbound.tag.to_string())
                    .collect();
                (server.is_listening(), inbounds)
            }
            None => (false, Vec::new()),
        };
        NodeStatus {
            running,
            core_version: "zray-core (embedded)".into(),
            inbounds,
            active_sessions: self.stats.active.load(Ordering::Relaxed),
        }
    }

    async fn apply(&self, config: serde_json::Value) -> Result<Applied, NodeError> {
        // A core with nothing to listen on does not run.
        let no_inbound = match config.get("inbounds") {
            None => true,
            Some(serde_json::Value::Array(inbounds)) => inbounds.is_empty(),
            // Malformed: the compiler says why.
            Some(_) => false,
        };
        if no_inbound {
            self.stop().await;
            return Ok(Applied::Stopped);
        }
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let (compiled, _) =
            zero_config::compile_config(&config, zero_core::GenerationId(generation))
                .map_err(NodeError::Config)?;
        let mut host = self.host.lock().await;
        match host.as_ref() {
            Some(running) => match running.apply(Arc::clone(&compiled.config)).await {
                Ok(zero_runtime::host::Applied::Reloaded) => Ok(Applied::Reloaded),
                Ok(zero_runtime::host::Applied::Restarted) => Ok(Applied::Restarted),
                Err(error) => Err(NodeError::Refused(error)),
            },
            None => {
                let started = Host::start_with(compiled.config, Arc::clone(&self.stats))
                    .await
                    .map_err(|error| NodeError::Refused(error.to_string()))?;
                *host = Some(Arc::new(started));
                Ok(Applied::Started)
            }
        }
    }

    async fn add_accounts(
        &self,
        inbound: &str,
        clients: &[serde_json::Value],
    ) -> Result<(), NodeError> {
        let server = self.host().await?.server();
        let accounts = clients
            .iter()
            .map(|client| server.parse_account(inbound, client))
            .collect::<Result<Vec<_>, _>>()
            .map_err(NodeError::Refused)?;
        server
            .add_accounts(inbound, &accounts)
            .map_err(NodeError::Refused)
    }

    async fn remove_accounts(&self, inbound: &str, emails: &[String]) -> Result<usize, NodeError> {
        let server = self.host().await?.server();
        let emails: Vec<&str> = emails.iter().map(String::as_str).collect();
        server
            .remove_accounts(inbound, &emails)
            .map_err(NodeError::Refused)
    }

    async fn traffic(&self, reset: bool) -> Vec<AccountTraffic> {
        self.stats
            .accounts
            .traffic(reset)
            .into_iter()
            .map(|t| AccountTraffic {
                email: t.email,
                uplink: t.uplink,
                downlink: t.downlink,
            })
            .collect()
    }

    async fn online(&self) -> Vec<OnlineAccount> {
        self.stats
            .accounts
            .online()
            .into_iter()
            .map(|o| OnlineAccount {
                email: o.email,
                sessions: o.sessions,
                addresses: o.addresses.iter().map(ToString::to_string).collect(),
            })
            .collect()
    }

    async fn end_sessions(&self, email: &str) -> usize {
        self.stats.accounts.end_sessions(email)
    }

    async fn end_address(&self, email: &str, address: std::net::IpAddr) -> usize {
        self.stats.accounts.end_address(email, address)
    }

    async fn set_limit(&self, email: &str, uplink: u64, downlink: u64) {
        self.stats.accounts.set_limit(email, uplink, downlink);
    }

    async fn assets(&self) -> Option<serde_json::Value> {
        let host = self.host.lock().await.clone()?;
        Some(host.server().asset_snapshot())
    }

    async fn refresh_assets(&self) -> Option<serde_json::Value> {
        let host = self.host.lock().await.clone()?;
        Some(host.server().refresh_assets_now().await)
    }

    async fn stop(&self) {
        if let Some(host) = self.host.lock().await.take() {
            host.stop(STOP_DRAIN).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn config(port: u16, clients: serde_json::Value) -> serde_json::Value {
        json!({
            "inbounds": [{
                "tag": "vless-in", "listen": "127.0.0.1", "port": port, "protocol": "vless",
                "settings": {"clients": clients, "decryption": "none"},
            }],
            "outbounds": [{"tag": "direct", "protocol": "freedom"}],
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_node_starts_reloads_takes_accounts_and_stops() {
        let node = LocalNode::new();
        assert!(!node.status().await.running);
        assert_eq!(
            node.add_accounts("vless-in", &[]).await,
            Err(NodeError::NotRunning)
        );

        let port = free_port();
        let applied = node.apply(config(port, json!([]))).await.unwrap();
        assert_eq!(applied, Applied::Started);
        let status = node.status().await;
        assert!(status.running);
        assert_eq!(status.inbounds, ["vless-in"]);

        node.add_accounts(
            "vless-in",
            &[json!({"id": "5783a3e7-e373-51cd-8642-c83782b807c5", "email": "a@p"})],
        )
        .await
        .unwrap();
        assert_eq!(
            node.remove_accounts("vless-in", &["a@p".into()]).await,
            Ok(1)
        );

        let applied = node.apply(config(port, json!([]))).await.unwrap();
        assert_eq!(applied, Applied::Reloaded);
        let applied = node.apply(config(free_port(), json!([]))).await.unwrap();
        assert_eq!(applied, Applied::Restarted);

        assert!(matches!(
            node.apply(json!({"inbounds": "nonsense"})).await,
            Err(NodeError::Config(_))
        ));

        let stopped = node.apply(json!({"inbounds": [], "outbounds": []})).await;
        assert_eq!(stopped, Ok(Applied::Stopped));
        assert!(!node.status().await.running);
        node.stop().await;
    }
}
