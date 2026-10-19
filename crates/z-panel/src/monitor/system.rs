//! The host's figures: processor, memory, disk, network, and the panel's own
//! memory. Read straight from `/proc` on Linux, with no library between: a
//! few small files every ten seconds. Elsewhere every figure is absent, and
//! the dashboard says so instead of showing zeros.
//!
//! Reading and arithmetic are apart so the arithmetic can be tested with
//! text in hand: `parse_*` turn a file's text into numbers, [`read`] does the
//! reading, and [`Sampler::step`] turns two readings into rates.

use std::collections::VecDeque;
use std::path::Path;
use std::time::Instant;

use serde::Serialize;

/// How many samples are kept: a day at one every ten seconds.
pub const KEEP: usize = 8640;

/// Cumulative processor time, as `/proc/stat` counts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    pub idle: u64,
    pub total: u64,
}

/// The first line of `/proc/stat`: all processors together.
pub fn parse_cpu(stat: &str) -> Option<CpuTimes> {
    let line = stat.lines().next()?;
    let mut fields = line.strip_prefix("cpu")?.split_whitespace();
    // user nice system idle iowait irq softirq steal; guest time is already
    // inside user, so it is not added again.
    let values: Vec<u64> = fields
        .by_ref()
        .take(8)
        .filter_map(|f| f.parse().ok())
        .collect();
    if values.len() < 4 {
        return None;
    }
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    Some(CpuTimes {
        idle,
        total: values.iter().sum(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    pub total: u64,
    pub used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

/// `/proc/meminfo`. "Used" is what is not available to programs, so the
/// page cache does not count against it, which is what `free` and every
/// monitor shows.
pub fn parse_meminfo(text: &str) -> Option<Memory> {
    let kb = |key: &str| -> Option<u64> {
        let line = text.lines().find(|line| line.starts_with(key))?;
        let value: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(value * 1024)
    };
    let total = kb("MemTotal:")?;
    let available = kb("MemAvailable:").or_else(|| kb("MemFree:"))?;
    let swap_total = kb("SwapTotal:").unwrap_or(0);
    let swap_free = kb("SwapFree:").unwrap_or(0);
    Some(Memory {
        total,
        used: total.saturating_sub(available),
        swap_total,
        swap_used: swap_total.saturating_sub(swap_free),
    })
}

/// Interfaces whose bytes are another interface's bytes again (loopback,
/// containers' and bridges' virtual links) would count traffic twice.
fn counts_for_network(name: &str) -> bool {
    !(name == "lo"
        || name.starts_with("veth")
        || name.starts_with("docker")
        || name.starts_with("br-")
        || name.starts_with("virbr")
        || name.starts_with("cni")
        || name.starts_with("flannel"))
}

/// `/proc/net/dev`: bytes received and sent by every real interface.
pub fn parse_net_dev(text: &str) -> Option<(u64, u64)> {
    let mut found = false;
    let (mut rx, mut tx) = (0u64, 0u64);
    for line in text.lines().skip(2) {
        let Some((name, counters)) = line.split_once(':') else {
            continue;
        };
        if !counts_for_network(name.trim()) {
            continue;
        }
        let fields: Vec<u64> = counters
            .split_whitespace()
            .filter_map(|f| f.parse().ok())
            .collect();
        // Receive bytes are the first column, transmit bytes the ninth.
        if let (Some(received), Some(sent)) = (fields.first(), fields.get(8)) {
            rx = rx.saturating_add(*received);
            tx = tx.saturating_add(*sent);
            found = true;
        }
    }
    found.then_some((rx, tx))
}

pub fn parse_loadavg(text: &str) -> Option<[f64; 3]> {
    let mut parts = text.split_whitespace().map(|p| p.parse::<f64>().ok());
    Some([parts.next()??, parts.next()??, parts.next()??])
}

pub fn parse_uptime(text: &str) -> Option<u64> {
    let seconds: f64 = text.split_whitespace().next()?.parse().ok()?;
    Some(seconds as u64)
}

/// The panel process's resident memory, from `/proc/self/status`.
pub fn parse_rss(text: &str) -> Option<u64> {
    let line = text.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

/// Total and used bytes of the file system `path` is on.
#[cfg(unix)]
pub fn disk(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: `c_path` is a valid NUL-terminated string, and `stat` is a
    // statvfs-sized buffer that the call fills in when it returns 0.
    let stat = unsafe {
        if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return None;
        }
        stat.assume_init()
    };
    let block = stat.f_frsize as u64;
    let total = stat.f_blocks as u64 * block;
    // Free to an ordinary user, as `df` reports it.
    let free = stat.f_bavail as u64 * block;
    (total > 0).then(|| (total, total.saturating_sub(free)))
}

#[cfg(not(unix))]
pub fn disk(_: &Path) -> Option<(u64, u64)> {
    None
}

/// One reading of everything.
#[derive(Debug, Clone, Default)]
pub struct Raw {
    pub cpu: Option<CpuTimes>,
    pub memory: Option<Memory>,
    pub net: Option<(u64, u64)>,
    pub load: Option<[f64; 3]>,
    pub uptime: Option<u64>,
    pub rss: Option<u64>,
    pub disk: Option<(u64, u64)>,
}

#[cfg(target_os = "linux")]
pub fn read(data_dir: &Path) -> Raw {
    let file = |path: &str| std::fs::read_to_string(path).ok();
    Raw {
        cpu: file("/proc/stat").and_then(|t| parse_cpu(&t)),
        memory: file("/proc/meminfo").and_then(|t| parse_meminfo(&t)),
        net: file("/proc/net/dev").and_then(|t| parse_net_dev(&t)),
        load: file("/proc/loadavg").and_then(|t| parse_loadavg(&t)),
        uptime: file("/proc/uptime").and_then(|t| parse_uptime(&t)),
        rss: file("/proc/self/status").and_then(|t| parse_rss(&t)),
        disk: disk(data_dir),
    }
}

#[cfg(not(target_os = "linux"))]
pub fn read(data_dir: &Path) -> Raw {
    Raw {
        disk: disk(data_dir),
        ..Raw::default()
    }
}

/// The figures as the dashboard shows them now.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Snapshot {
    /// Unix seconds.
    pub at: i64,
    pub cores: usize,
    /// Percent of all processors busy since the last sample.
    pub cpu: Option<f32>,
    /// Load averages over 1, 5 and 15 minutes.
    pub load: Option<[f64; 3]>,
    pub mem_total: Option<u64>,
    pub mem_used: Option<u64>,
    pub swap_total: Option<u64>,
    pub swap_used: Option<u64>,
    pub disk_total: Option<u64>,
    pub disk_used: Option<u64>,
    /// Bytes per second, over all real interfaces.
    pub rx_rate: Option<f64>,
    pub tx_rate: Option<f64>,
    /// Bytes since the machine started.
    pub rx_total: Option<u64>,
    pub tx_total: Option<u64>,
    pub uptime: Option<u64>,
    pub panel_rss: Option<u64>,
}

/// One point of the history the charts draw.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Point {
    pub t: i64,
    pub cpu: Option<f32>,
    pub mem: Option<u64>,
    pub rx: Option<f32>,
    pub tx: Option<f32>,
}

/// Turns readings into rates, and keeps the history.
#[derive(Default)]
pub struct Sampler {
    previous: Option<(Instant, Raw)>,
    history: VecDeque<Point>,
}

impl Sampler {
    /// Take `raw`, read at `now` (Unix `at`), and say what it makes of the
    /// time since the last one.
    pub fn step(&mut self, raw: Raw, now: Instant, at: i64, cores: usize) -> Snapshot {
        let mut snapshot = Snapshot {
            at,
            cores,
            load: raw.load,
            mem_total: raw.memory.map(|m| m.total),
            mem_used: raw.memory.map(|m| m.used),
            swap_total: raw.memory.map(|m| m.swap_total),
            swap_used: raw.memory.map(|m| m.swap_used),
            disk_total: raw.disk.map(|d| d.0),
            disk_used: raw.disk.map(|d| d.1),
            rx_total: raw.net.map(|n| n.0),
            tx_total: raw.net.map(|n| n.1),
            uptime: raw.uptime,
            panel_rss: raw.rss,
            ..Snapshot::default()
        };
        if let Some((then, before)) = &self.previous {
            let seconds = now.duration_since(*then).as_secs_f64();
            if let (Some(a), Some(b)) = (before.cpu, raw.cpu) {
                let total = b.total.saturating_sub(a.total);
                let idle = b.idle.saturating_sub(a.idle);
                if total > 0 {
                    let busy = total.saturating_sub(idle) as f64 / total as f64 * 100.0;
                    snapshot.cpu = Some(busy.clamp(0.0, 100.0) as f32);
                }
            }
            if let (Some(a), Some(b)) = (before.net, raw.net) {
                // A counter that went down was reset (an interface came
                // back): that second's rate is unknown, not enormous.
                if seconds > 0.0 && b.0 >= a.0 && b.1 >= a.1 {
                    snapshot.rx_rate = Some((b.0 - a.0) as f64 / seconds);
                    snapshot.tx_rate = Some((b.1 - a.1) as f64 / seconds);
                }
            }
        }
        self.previous = Some((now, raw));
        if self.history.len() == KEEP {
            self.history.pop_front();
        }
        self.history.push_back(Point {
            t: at,
            cpu: snapshot.cpu,
            mem: snapshot.mem_used,
            rx: snapshot.rx_rate.map(|r| r as f32),
            tx: snapshot.tx_rate.map(|r| r as f32),
        });
        snapshot
    }

    /// The last `minutes` of history, in at most `max` points: neighbours
    /// are averaged, so a day fits a chart as faithfully as an hour does.
    pub fn history(&self, minutes: i64, max: usize, now: i64) -> Vec<Point> {
        let since = now - minutes * 60;
        let points: Vec<Point> = self
            .history
            .iter()
            .filter(|p| p.t >= since)
            .copied()
            .collect();
        downsample(&points, max)
    }
}

fn mean<T: Copy>(
    values: impl Iterator<Item = Option<T>>,
    to_f64: impl Fn(T) -> f64,
) -> Option<f64> {
    let (sum, count) = values
        .flatten()
        .fold((0.0, 0u32), |(sum, count), v| (sum + to_f64(v), count + 1));
    (count > 0).then(|| sum / f64::from(count))
}

/// `points` in at most `max` points, each the mean of a run of neighbours.
pub fn downsample(points: &[Point], max: usize) -> Vec<Point> {
    if max == 0 || points.len() <= max {
        return points.to_vec();
    }
    let size = points.len().div_ceil(max);
    points
        .chunks(size)
        .map(|chunk| Point {
            t: chunk[chunk.len() / 2].t,
            cpu: mean(chunk.iter().map(|p| p.cpu), f64::from).map(|v| v as f32),
            mem: mean(chunk.iter().map(|p| p.mem), |v| v as f64).map(|v| v as u64),
            rx: mean(chunk.iter().map(|p| p.rx), f64::from).map(|v| v as f32),
            tx: mean(chunk.iter().map(|p| p.tx), f64::from).map(|v| v as f32),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const STAT: &str = "cpu  4705 150 1120 16250 520 30 45 10 0 0\ncpu0 2000 70 500 8000 200 10 20 5 0 0\nintr 1\n";
    const MEMINFO: &str = "MemTotal:       16384000 kB\nMemFree:         1000000 kB\nMemAvailable:    8192000 kB\nBuffers:          500000 kB\nSwapTotal:       2097152 kB\nSwapFree:        1048576 kB\n";
    const NET: &str = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 9000000   100    0    0    0     0          0         0  9000000   100    0    0    0     0       0          0\n  eth0: 1000000   500    0    0    0     0          0         0  250000   300    0    0    0     0       0          0\nveth1a: 777777   5    0    0    0     0          0         0  777777   5    0    0    0     0       0          0\n";

    #[test]
    fn proc_files_read_into_numbers() {
        assert_eq!(
            parse_cpu(STAT),
            Some(CpuTimes {
                idle: 16250 + 520,
                total: 4705 + 150 + 1120 + 16250 + 520 + 30 + 45 + 10
            })
        );
        assert_eq!(parse_cpu("nonsense"), None);
        let memory = parse_meminfo(MEMINFO).unwrap();
        assert_eq!(
            (memory.total, memory.used),
            (16384000 * 1024, (16384000 - 8192000) * 1024)
        );
        assert_eq!(
            (memory.swap_total, memory.swap_used),
            (2097152 * 1024, 1048576 * 1024)
        );
        assert_eq!(
            parse_net_dev(NET),
            Some((1_000_000, 250_000)),
            "lo and veth are left out"
        );
        assert_eq!(
            parse_loadavg("0.52 0.40 0.30 1/200 12345"),
            Some([0.52, 0.40, 0.30])
        );
        assert_eq!(parse_uptime("12345.67 9999.00"), Some(12345));
        assert_eq!(
            parse_rss("Name:\tz-ui\nVmRSS:\t   20480 kB\n"),
            Some(20480 * 1024)
        );
    }

    fn raw(idle: u64, total: u64, rx: u64, tx: u64) -> Raw {
        Raw {
            cpu: Some(CpuTimes { idle, total }),
            net: Some((rx, tx)),
            ..Raw::default()
        }
    }

    #[test]
    fn rates_come_from_two_readings() {
        let start = Instant::now();
        let mut sampler = Sampler::default();
        let first = sampler.step(raw(100, 200, 1000, 500), start, 1000, 4);
        assert_eq!(
            (first.cpu, first.rx_rate),
            (None, None),
            "one reading has no rate"
        );
        // 100 ticks passed, 25 of them idle; 5000 bytes in and 1000 out in 10 s.
        let second = sampler.step(
            raw(125, 300, 6000, 1500),
            start + Duration::from_secs(10),
            1010,
            4,
        );
        assert_eq!(second.cpu, Some(75.0));
        assert_eq!((second.rx_rate, second.tx_rate), (Some(500.0), Some(100.0)));
        // A counter that went back down is not a huge rate.
        let third = sampler.step(
            raw(150, 400, 10, 10),
            start + Duration::from_secs(20),
            1020,
            4,
        );
        assert_eq!(third.rx_rate, None);
        assert_eq!(sampler.history(60, 100, 1020).len(), 3);
    }

    #[test]
    fn history_is_bounded_and_downsampled() {
        let mut sampler = Sampler::default();
        let start = Instant::now();
        for i in 0..(KEEP + 50) {
            sampler.step(
                raw(0, 0, 0, 0),
                start + Duration::from_secs(i as u64),
                i as i64,
                1,
            );
        }
        assert_eq!(sampler.history.len(), KEEP);
        let day = sampler.history(24 * 60, 240, (KEEP + 50) as i64);
        assert!(day.len() <= 240 && day.len() > 200, "{}", day.len());
        let points: Vec<Point> = (0..10)
            .map(|i| Point {
                t: i,
                cpu: Some(i as f32 * 10.0),
                mem: None,
                rx: None,
                tx: None,
            })
            .collect();
        let two = downsample(&points, 2);
        assert_eq!(two.len(), 2);
        assert_eq!((two[0].cpu, two[1].cpu), (Some(20.0), Some(70.0)));
        assert_eq!(two[0].mem, None, "no readings stay none");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_machine_can_be_read() {
        let raw = read(Path::new("."));
        assert!(raw.cpu.is_some() && raw.memory.is_some() && raw.net.is_some());
        let (total, used) = raw.disk.expect("a disk");
        assert!(total > 0 && used <= total);
        assert!(raw.rss.unwrap() > 0);
    }
}
