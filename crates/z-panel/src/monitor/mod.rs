//! What the dashboard watches: the host's figures ([`system`]) and the live
//! log ([`logs`]). Both live in memory, in one [`Monitor`] that the whole
//! panel shares; a restart starts them afresh (the users' traffic, which must
//! survive, is in the database).

pub mod logs;
pub mod system;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

pub use logs::LogBuffer;
use system::{Point, Sampler, Snapshot};

/// How often the host is read.
pub const INTERVAL: Duration = Duration::from_secs(10);

pub struct Monitor {
    pub logs: LogBuffer,
    data_dir: PathBuf,
    sampler: Mutex<Sampler>,
    latest: RwLock<Option<Snapshot>>,
    started: Instant,
}

impl Monitor {
    pub fn new(logs: LogBuffer, data_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            logs,
            data_dir,
            sampler: Mutex::new(Sampler::default()),
            latest: RwLock::new(None),
            started: Instant::now(),
        })
    }

    /// Read the host once and record it.
    pub fn sample(&self) {
        let raw = system::read(&self.data_dir);
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        let at = z_db::now();
        let snapshot = self.sampler.lock().unwrap_or_else(|p| p.into_inner()).step(
            raw,
            Instant::now(),
            at,
            cores,
        );
        *self.latest.write().unwrap_or_else(|p| p.into_inner()) = Some(snapshot);
    }

    /// Make `snapshot` the latest reading, for a test.
    #[cfg(test)]
    pub fn inject(&self, snapshot: Snapshot) {
        *self.latest.write().unwrap_or_else(|p| p.into_inner()) = Some(snapshot);
    }

    pub fn latest(&self) -> Option<Snapshot> {
        self.latest
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn history(&self, minutes: i64, max: usize) -> Vec<Point> {
        self.sampler
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .history(minutes, max, z_db::now())
    }

    /// Seconds since the panel started.
    pub fn uptime(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// Sample now, then every [`INTERVAL`], until the process ends.
    pub fn spawn(self: &Arc<Self>) {
        let monitor = Arc::clone(self);
        tokio::spawn(async move {
            // A rate needs two readings: the second comes soon, so the
            // dashboard has figures within seconds of opening.
            monitor.sample();
            tokio::time::sleep(Duration::from_secs(2)).await;
            let mut ticker = tokio::time::interval(INTERVAL);
            loop {
                ticker.tick().await;
                let monitor = Arc::clone(&monitor);
                // Reading /proc is quick, but not async: keep it off the
                // workers that serve requests.
                let _ = tokio::task::spawn_blocking(move || monitor.sample()).await;
            }
        });
    }
}
