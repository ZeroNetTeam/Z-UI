//! Replacing a panel that is installed on this machine (3X-UI or PasarGuard)
//! with this one: its users, inbounds and settings move over, the
//! users' connections come back by themselves within seconds, its files stay
//! where they are, stopped, for a week, and a way back stays open until they
//! are deleted. See [`run`] for the steps.

pub mod detect;
pub mod host;
pub mod run;
pub mod state;

use std::path::PathBuf;
use std::time::Duration;

use crate::notify::Event;
use crate::server::AppState;

/// Where the machine's files are: `/`, unless `ZUI_PREFIX` says a directory
/// made for trying, as the installer's tests use.
pub fn root() -> PathBuf {
    std::env::var_os("ZUI_PREFIX")
        .filter(|p| !p.is_empty())
        .map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// Once the week is over the old panel's files are deleted, and a day before
/// the admins are told. Looked at every half hour.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut delay = Duration::from_secs(60);
        loop {
            tokio::time::sleep(delay).await;
            delay = Duration::from_secs(1800);
            let host = state.host();
            let root = state.takeover_root();
            let cx = run::Context {
                root: &root,
                host: &*host,
                state: &state,
                say: &|_| {},
            };
            match run::tick(&cx).await {
                run::Tick::Idle => {}
                run::Tick::Expiring { hours } => state.notify(Event::TakeoverExpiring { hours }),
                run::Tick::Deleted => state.notify(Event::TakeoverCleaned { kept: None }),
                run::Tick::Kept { why } => state.notify(Event::TakeoverCleaned { kept: Some(why) }),
            }
        }
    });
}

#[cfg(test)]
mod tests;
