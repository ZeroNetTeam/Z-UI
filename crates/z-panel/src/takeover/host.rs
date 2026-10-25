//! What a takeover does to the machine: run `systemctl` and `docker`, ask
//! whether a port answers, wait. Behind a trait so the order of the steps can
//! be tried without a machine to break (the tests give it a model of one).

use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

pub trait Host: Send + Sync {
    /// Run a program: its output when it exits well, what it said when not.
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String>;
    fn has_command(&self, program: &str) -> bool;
    /// Whether something on this machine accepts connections on `port`.
    fn tcp_open(&self, port: u16) -> bool;
    /// Start a program that goes on after this one is gone, and after the
    /// service that started it is stopped (a rollback asked for from the
    /// panel stops the panel).
    fn spawn_detached(&self, program: &str, args: &[&str]) -> Result<(), String>;
    fn sleep(&self, duration: Duration);
    /// Whether nothing real is done (so nothing is waited for).
    fn dry(&self) -> bool {
        false
    }
}

/// The machine itself.
pub struct System;

impl Host for System {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let output = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("{program}: {error}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            let said = String::from_utf8_lossy(&output.stderr);
            let said = said.trim();
            Err(format!(
                "{program} {} failed: {}",
                args.join(" "),
                if said.is_empty() { "no message" } else { said }
            ))
        }
    }

    fn has_command(&self, program: &str) -> bool {
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
            .unwrap_or(false)
    }

    fn tcp_open(&self, port: u16) -> bool {
        TcpStream::connect_timeout(
            &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            Duration::from_millis(300),
        )
        .is_ok()
    }

    fn spawn_detached(&self, program: &str, args: &[&str]) -> Result<(), String> {
        // A transient unit of its own is outside the cgroup of the service
        // that asked, so stopping that service does not stop this.
        if self.has_command("systemd-run") {
            let unit = format!("z-ui-takeover-{}", z_db::now());
            let mut all = vec!["--no-block", "--collect", "--unit", &unit, program];
            all.extend_from_slice(args);
            return self.run("systemd-run", &all).map(|_| ());
        }
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("{program}: {error}"))
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// For a root that is a directory made for trying (the installer's tests):
/// no service or container is touched, and nothing is waited for.
pub struct Dry;

impl Host for Dry {
    fn run(&self, _: &str, _: &[&str]) -> Result<String, String> {
        Ok(String::new())
    }

    fn has_command(&self, _: &str) -> bool {
        true
    }

    fn tcp_open(&self, _: u16) -> bool {
        true
    }

    fn spawn_detached(&self, _: &str, _: &[&str]) -> Result<(), String> {
        Ok(())
    }

    fn sleep(&self, _: Duration) {}

    fn dry(&self) -> bool {
        true
    }
}

/// The host to use: [`Dry`] when `ZUI_NO_SYSTEMD=1`, as the installer's
/// switch of the same name says, else the machine.
pub fn from_env() -> std::sync::Arc<dyn Host> {
    if std::env::var("ZUI_NO_SYSTEMD").is_ok_and(|v| v == "1") {
        std::sync::Arc::new(Dry)
    } else {
        std::sync::Arc::new(System)
    }
}

/// `systemctl <args>`.
pub fn systemctl(host: &dyn Host, args: &[&str]) -> Result<String, String> {
    host.run("systemctl", args)
}

/// `docker compose -f <file> <args>`, or `docker-compose` where that is what
/// there is.
pub fn compose(host: &dyn Host, file: &str, args: &[&str]) -> Result<String, String> {
    if host.run("docker", &["compose", "version"]).is_ok() {
        let mut all = vec!["compose", "-f", file];
        all.extend_from_slice(args);
        host.run("docker", &all)
    } else {
        let mut all = vec!["-f", file];
        all.extend_from_slice(args);
        host.run("docker-compose", &all)
    }
}
