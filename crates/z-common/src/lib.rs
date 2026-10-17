//! `z-common` — what the panel and its nodes say to each other.
//!
//! The node types here are plain data with both serde directions, so the
//! same shapes cross a process boundary unchanged when nodes become remote
//! (`RemoteNode`, v1.0) as they do today inside the panel (`LocalNode`).

pub mod node;

/// The panel's version, as built.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The product name, as shown to people.
pub const PRODUCT: &str = "Z-UI";
