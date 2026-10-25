//! The steps of a takeover against a model of a machine: services that hold
//! ports while they run, a file tree under a temporary root, and a
//! database made the way each panel makes its own.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::detect::{detect, inspect, Service};
use super::host::Host;
use super::run::{self, Context, Tick};
use super::state::{self, Phase};
use crate::migrate::model::Kind;
use crate::migrate::tests::{TempDb, CLASSIC, PASARGUARD};
use crate::server::testing::{panel, Panel};

/// A machine: what each service holds while it runs, and what was asked of it.
#[derive(Default)]
pub(crate) struct Fake {
    log: Mutex<Vec<String>>,
    open: Mutex<HashSet<u16>>,
    owners: Mutex<HashMap<String, Vec<u16>>>,
    failing: Mutex<HashSet<String>>,
    stubborn: Mutex<HashSet<u16>>,
}

impl Fake {
    fn owns(&self, service: &str, ports: &[u16]) {
        self.owners
            .lock()
            .unwrap()
            .insert(service.into(), ports.to_vec());
    }

    fn set(&self, service: &str, up: bool) {
        let owners = self.owners.lock().unwrap();
        let mut open = self.open.lock().unwrap();
        for port in owners.get(service).into_iter().flatten() {
            if up {
                open.insert(*port);
            } else if !self.stubborn.lock().unwrap().contains(port) {
                open.remove(port);
            }
        }
    }

    fn fail(&self, command: &str) {
        self.failing.lock().unwrap().insert(command.into());
    }

    fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// Where in the log a command was given (the first time).
    fn at(&self, command: &str) -> Option<usize> {
        self.calls().iter().position(|c| c == command)
    }

    fn is_open(&self, port: u16) -> bool {
        self.open.lock().unwrap().contains(&port)
    }
}

impl Host for Fake {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let line = format!("{program} {}", args.join(" "));
        self.log.lock().unwrap().push(line.clone());
        if self.failing.lock().unwrap().contains(&line) {
            return Err(format!("{line} failed"));
        }
        match (program, args) {
            ("systemctl", [verb, unit]) => match *verb {
                "stop" => self.set(unit, false),
                "start" | "restart" => self.set(unit, true),
                _ => {}
            },
            ("docker", ["compose", "-f", file, verb, ..]) => match *verb {
                "stop" | "down" => self.set(file, false),
                "start" | "up" => self.set(file, true),
                _ => {}
            },
            _ => {}
        }
        Ok(String::new())
    }

    fn has_command(&self, _: &str) -> bool {
        true
    }

    fn tcp_open(&self, port: u16) -> bool {
        self.is_open(port)
    }

    fn spawn_detached(&self, program: &str, args: &[&str]) -> Result<(), String> {
        self.log
            .lock()
            .unwrap()
            .push(format!("detached {program} {}", args.join(" ")));
        Ok(())
    }

    fn sleep(&self, _: Duration) {}
}

fn temp_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "z-takeover-{}-{}",
        std::process::id(),
        rand::random::<u32>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn put(root: &Path, path: &str, bytes: &[u8]) {
    let target = root.join(path.trim_start_matches('/'));
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, bytes).unwrap();
}

/// 3X-UI the way its installer leaves it, with `database` as its database.
fn three_x_ui(root: &Path, database: &TempDb) {
    put(root, "/usr/local/x-ui/x-ui", b"binary");
    put(
        root,
        "/usr/local/x-ui/bin/xray-linux-amd64",
        &vec![9u8; 50_000],
    );
    put(root, "/usr/local/x-ui/bin/config.json", b"{\"log\":{}}");
    put(root, "/etc/x-ui/x-ui.db", &database.bytes());
    put(
        root,
        "/etc/systemd/system/x-ui.service",
        b"[Service]\nExecStart=/usr/local/x-ui/x-ui\n",
    );
    put(root, "/usr/bin/x-ui", b"#!/bin/sh\n");
    put(root, "/var/log/x-ui/x-ui.log", b"log line\n");
}

fn pasarguard(root: &Path, database: &TempDb, env: &str) {
    put(
        root,
        "/opt/pasarguard/docker-compose.yml",
        b"services:\n  pasarguard:\n    image: x\n",
    );
    put(root, "/opt/pasarguard/.env", env.as_bytes());
    put(root, "/var/lib/pasarguard/db.sqlite3", &database.bytes());
    put(root, "/var/lib/pasarguard/xray-core/xray", b"x");
    put(root, "/usr/local/bin/pasarguard", b"#!/bin/sh\n");
}

/// Every file under `paths` (as they are under `root`), by name.
fn tree(root: &Path, paths: &[&str]) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let meta = match dir.symlink_metadata() {
            Ok(m) => m,
            Err(_) => return,
        };
        if meta.is_dir() {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                walk(base, &entry.path(), out);
            }
        } else if meta.is_file() {
            out.insert(
                dir.strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                std::fs::read(dir).unwrap(),
            );
        }
    }
    let mut out = BTreeMap::new();
    for path in paths {
        walk(root, &root.join(path.trim_start_matches('/')), &mut out);
    }
    out
}

const X_UI_PATHS: [&str; 6] = [
    "/usr/local/x-ui",
    "/etc/x-ui",
    "/var/log/x-ui",
    "/usr/bin/x-ui",
    "/etc/systemd/system/x-ui.service",
    "/usr/lib/systemd/system/x-ui.service",
];

struct World {
    panel: Panel,
    root: PathBuf,
    fake: std::sync::Arc<Fake>,
}

impl World {
    async fn new() -> Self {
        Self {
            panel: panel("/").await,
            root: temp_root(),
            fake: std::sync::Arc::new(Fake::default()),
        }
    }

    fn cx(&self) -> Context<'_> {
        Context {
            root: &self.root,
            host: &*self.fake,
            state: &self.panel.state,
            say: &|_| {},
        }
    }

    async fn found(&self) -> super::detect::Detected {
        let mut all = detect(&self.root);
        assert_eq!(all.len(), 1, "{all:?}");
        let mut one = all.remove(0);
        inspect(&self.root, &mut one).await;
        one
    }

    /// 3X-UI installed and running, ready to be taken over.
    async fn with_three_x_ui() -> Self {
        let world = Self::new().await;
        three_x_ui(&world.root, &TempDb::new(CLASSIC).await);
        world
            .fake
            .owns("x-ui", &[2053, 2097, 443, 8443, 2096, 1080]);
        world.fake.set("x-ui", true);
        world
    }

    /// What this panel will hold once it runs: its own port, the inbounds' and
    /// the subscription's.
    fn z_ui_holds(&self, ports: &[u16]) {
        let mut all = vec![self.panel.state.config().listen.port()];
        all.extend_from_slice(ports);
        self.fake.owns("z-ui", &all);
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

async fn change_old_database(world: &World, sql: &str) {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    let options = SqliteConnectOptions::new()
        .filename(world.root.join("etc/x-ui/x-ui.db"))
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
async fn the_panels_that_can_be_replaced_are_found_by_what_their_installers_leave() {
    let root = temp_root();
    assert!(detect(&root).is_empty(), "an empty machine");
    three_x_ui(&root, &TempDb::new(CLASSIC).await);
    pasarguard(
        &root,
        &TempDb::new(PASARGUARD).await,
        "UVICORN_PORT = 8000\nUVICORN_HOST = \"127.0.0.1\"\n",
    );
    let mut found = detect(&root);
    found.sort_by_key(|d| d.kind.label());
    assert_eq!(
        found.iter().map(|d| d.kind).collect::<Vec<_>>(),
        [Kind::ThreeXUi, Kind::Pasarguard]
    );

    let x_ui = &found[0];
    assert_eq!(
        x_ui.services,
        [Service::Systemd {
            unit: "x-ui".into()
        }]
    );
    assert_eq!(
        x_ui.database.as_deref(),
        Some(Path::new("/etc/x-ui/x-ui.db"))
    );
    assert!(
        x_ui.remove.contains(&PathBuf::from("/usr/local/x-ui"))
            && x_ui.remove.contains(&PathBuf::from("/usr/bin/x-ui"))
    );
    assert!(x_ui.unsupported.is_none());

    let pg = &found[1];
    assert_eq!(
        pg.services,
        [Service::Compose {
            file: "/opt/pasarguard/docker-compose.yml".into()
        }]
    );
    assert_eq!(
        pg.database.as_deref(),
        Some(Path::new("/var/lib/pasarguard/db.sqlite3"))
    );
    assert_eq!(
        (pg.panel_port, pg.sub.as_ref().map(|s| s.text())),
        (Some(8000), Some("127.0.0.1:8000".into()))
    );
    assert_eq!(pg.remove.len(), 3);

    // 3X-UI says where it listens in its own database.
    let mut x_ui = found.remove(0);
    inspect(&root, &mut x_ui).await;
    assert_eq!(
        (x_ui.panel_port, x_ui.sub.as_ref().map(|s| s.text())),
        (Some(2053), Some("0.0.0.0:2097".into()))
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_panel_that_cannot_be_replaced_is_found_and_refused_with_the_reason() {
    let root = temp_root();
    // PasarGuard on MySQL: the data is not in a file.
    put(
        &root,
        "/opt/pasarguard/docker-compose.yml",
        b"services: {}\n",
    );
    put(
        &root,
        "/opt/pasarguard/.env",
        b"SQLALCHEMY_DATABASE_URL = \"mysql+asyncmy://u:pw@127.0.0.1/pg\"\n",
    );
    put(&root, "/var/lib/pasarguard/x", b"x");
    let found = detect(&root);
    assert_eq!(found.len(), 1);
    assert!(found[0].unsupported.as_ref().unwrap().contains("mysql"));
    let world = World::new().await;
    let refused = run::prepare(&world.cx(), found[0].clone())
        .await
        .err()
        .unwrap();
    assert_eq!(refused.code, "unsupported");
    assert!(world.fake.calls().is_empty(), "nothing was touched");
    std::fs::remove_dir_all(&root).unwrap();

    // Marzban is for the Migration page, not for replacing.
    let root = temp_root();
    put(&root, "/opt/marzban/docker-compose.yml", b"services: {}\n");
    put(&root, "/var/lib/marzban/db.sqlite3", b"x");
    let found = detect(&root);
    assert_eq!(found[0].kind, Kind::Marzban);
    assert!(found[0]
        .unsupported
        .as_ref()
        .unwrap()
        .contains("3X-UI and PasarGuard"));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn preparing_brings_everything_over_with_the_old_panel_untouched_and_running() {
    let world = World::with_three_x_ui().await;
    let before = tree(&world.root, &X_UI_PATHS);
    let prepared = run::prepare(&world.cx(), world.found().await)
        .await
        .unwrap();

    assert_eq!(prepared.counts.users, 10);
    assert_eq!(prepared.counts.inbounds, 4);
    assert_eq!(
        prepared.enable.len(),
        4,
        "every inbound that was on there will be switched on"
    );

    let db = world.panel.state.db();
    assert!(
        db.inbounds().await.unwrap().iter().all(|i| !i.enabled),
        "off until the cutover"
    );
    assert!(db.user_by_token("ali0sub1id").await.unwrap().is_some());

    // The old panel was not touched, not once, and still serves.
    assert!(world.fake.calls().is_empty(), "{:?}", world.fake.calls());
    assert_eq!(tree(&world.root, &X_UI_PATHS), before);
    assert!(world.fake.is_open(443) && world.fake.is_open(2053));

    let st = state::load(&world.panel.state.config().data_dir).unwrap();
    assert_eq!(st.phase, Phase::Prepared);
    assert_eq!(st.created_users.len(), 10);
    assert_eq!(st.inbound_ports.len(), 4);

    // Doing it again changes nothing that is there already.
    let again = run::prepare(&world.cx(), world.found().await)
        .await
        .unwrap();
    assert_eq!(again.counts.users, 0);
    assert_eq!(db.all_users().await.unwrap().len(), 10);
    assert_eq!(db.inbounds().await.unwrap().len(), 4);
    assert_eq!(
        state::load(&world.panel.state.config().data_dir)
            .unwrap()
            .created_users
            .len(),
        10
    );
}

#[tokio::test]
async fn the_cutover_stops_the_old_panel_first_brings_what_changed_and_starts_this_one() {
    let world = World::with_three_x_ui().await;
    run::prepare(&world.cx(), world.found().await)
        .await
        .unwrap();
    let before = tree(&world.root, &X_UI_PATHS);

    // Meanwhile at the old panel: one user used more, two went and two are new.
    change_old_database(
        &world,
        "UPDATE client_traffics SET up = 5000000000, down = 1000000000 WHERE email = 'bob@home';
         DELETE FROM client_traffics WHERE email = 'troj@home';
         UPDATE inbounds SET settings = replace(settings, '\"email\": \"troj@home\"', '\"email\": \"gone@home\"') WHERE tag = 'inbound-2096';
         UPDATE inbounds SET settings = replace(settings, '\"email\": \"ss@home\"', '\"email\": \"new@home\"') WHERE tag = 'inbound-1080';",
    )
    .await;
    let before = {
        // The old database is the one thing the test itself changed.
        let mut t = before;
        t.insert(
            "etc/x-ui/x-ui.db".into(),
            std::fs::read(world.root.join("etc/x-ui/x-ui.db")).unwrap(),
        );
        t
    };
    world.z_ui_holds(&[443, 8443, 2096, 1080, 2097]);

    let switched = run::cutover(&world.cx()).await.unwrap();
    assert_eq!(switched.refreshed, 10 - 2, "the others brought up to date");
    assert_eq!(
        switched.added_users, 2,
        "a user new at the old panel, and the renamed one"
    );
    assert_eq!(switched.dropped_users, 2);

    // The order is what keeps the cut short: the old one stops before this one starts.
    let stop = world.fake.at("systemctl stop x-ui").unwrap();
    let start = world.fake.at("systemctl restart z-ui").unwrap();
    assert!(stop < start, "{:?}", world.fake.calls());
    assert!(world.fake.at("systemctl disable x-ui").is_some());

    let db = world.panel.state.db();
    let users = db.all_users().await.unwrap();
    let bob = users.iter().find(|u| u.username == "bob@home").unwrap();
    assert_eq!(
        bob.used_traffic, 6_000_000_000,
        "what the old panel counted up to the last moment"
    );
    assert!(
        users.iter().any(|u| u.username == "new@home")
            && users.iter().any(|u| u.username == "gone@home")
    );
    assert!(!users
        .iter()
        .any(|u| u.username == "troj@home" || u.username == "ss@home"));
    assert!(
        db.inbounds().await.unwrap().iter().all(|i| i.enabled),
        "switched on"
    );

    // Subscriptions answer where they did.
    let env = std::fs::read_to_string(world.root.join("etc/z-ui/z-ui.env")).unwrap();
    assert!(env.contains("ZUI_SUB_LISTEN=0.0.0.0:2097"), "{env}");

    // The old panel is not touched: stopped, and every file where it was, for a week.
    assert_eq!(tree(&world.root, &X_UI_PATHS), before);
    let st = state::load(&world.panel.state.config().data_dir).unwrap();
    assert_eq!(st.phase, Phase::Switched);
    assert_eq!(
        st.expires_at,
        st.switched_at.map(|at| at + 7 * 86_400),
        "a week"
    );
    assert_eq!(switched.expires_at, st.expires_at.unwrap());
    assert!(world.fake.is_open(443) && world.fake.is_open(2097));
    assert_eq!(
        st.moved.len(),
        10 - 2 + 2,
        "every user that came over is known by the name it had"
    );
    assert!(st
        .moved
        .iter()
        .any(|m| m.username == "bob@home" && m.source_name == "bob@home"));
}

#[tokio::test]
async fn when_this_panel_does_not_answer_the_old_one_is_started_again() {
    let world = World::with_three_x_ui().await;
    run::prepare(&world.cx(), world.found().await)
        .await
        .unwrap();
    let before = tree(&world.root, &X_UI_PATHS);
    // This panel starts but holds none of the inbounds' ports.
    world
        .fake
        .owns("z-ui", &[world.panel.state.config().listen.port()]);

    let error = run::cutover(&world.cx()).await.err().unwrap();
    assert_eq!(error.code, "not_listening", "{error}");
    let calls = world.fake.calls();
    let stopped = calls
        .iter()
        .position(|c| c == "systemctl stop x-ui")
        .unwrap();
    let restarted = calls
        .iter()
        .rposition(|c| c == "systemctl start x-ui")
        .unwrap();
    assert!(stopped < restarted, "{calls:?}");
    assert!(world.fake.is_open(443), "the old panel serves again");
    assert_eq!(tree(&world.root, &X_UI_PATHS), before);
    assert!(world
        .panel
        .state
        .db()
        .inbounds()
        .await
        .unwrap()
        .iter()
        .all(|i| !i.enabled));
    assert!(
        std::fs::read_to_string(world.root.join("etc/z-ui/z-ui.env"))
            .unwrap_or_default()
            .is_empty(),
        "no subscription listener left behind"
    );
    assert_eq!(
        state::load(&world.panel.state.config().data_dir)
            .unwrap()
            .phase,
        Phase::Prepared
    );
    // It can be tried again.
    world.z_ui_holds(&[443, 8443, 2096, 1080, 2097]);
    run::cutover(&world.cx()).await.unwrap();
}

#[tokio::test]
async fn a_service_that_will_not_stop_or_a_port_that_stays_held_leaves_the_old_panel_running() {
    let world = World::with_three_x_ui().await;
    run::prepare(&world.cx(), world.found().await)
        .await
        .unwrap();
    world.z_ui_holds(&[443, 8443, 2096, 1080, 2097]);
    world.fake.fail("systemctl stop x-ui");
    let error = run::cutover(&world.cx()).await.err().unwrap();
    assert_eq!(error.code, "stop_failed");
    assert!(
        world.fake.at("systemctl restart z-ui").is_none(),
        "this panel was never started"
    );

    // Something else holds port 443 whatever the old panel does.
    world.fake.failing.lock().unwrap().clear();
    world.fake.stubborn.lock().unwrap().insert(443);
    let error = run::cutover(&world.cx()).await.err().unwrap();
    assert_eq!(error.code, "ports_busy");
    assert!(world.fake.is_open(8443), "the old panel runs again");
}

/// 3X-UI taken over, with `bob@home` having used something here too.
async fn switched_three_x_ui(extra_usage: i64) -> World {
    let world = World::with_three_x_ui().await;
    run::prepare(&world.cx(), world.found().await)
        .await
        .unwrap();
    world.z_ui_holds(&[443, 8443, 2096, 1080, 2097]);
    run::cutover(&world.cx()).await.unwrap();
    let db = world.panel.state.db();
    let bob = db
        .all_users()
        .await
        .unwrap()
        .into_iter()
        .find(|u| u.username == "bob@home")
        .unwrap();
    db.add_traffic(&[z_db::TrafficDelta {
        user_id: bob.id,
        uplink: extra_usage / 4,
        downlink: extra_usage - extra_usage / 4,
    }])
    .await
    .unwrap();
    world
}

async fn old_down(world: &World, email: &str) -> i64 {
    let source = z_db::Source::open(&world.root.join("etc/x-ui/x-ui.db").to_string_lossy())
        .await
        .unwrap();
    source
        .query(&format!(
            "SELECT CAST(down AS TEXT) FROM client_traffics WHERE email = '{email}'"
        ))
        .await
        .unwrap()[0][0]
        .clone()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn going_back_gives_the_old_panel_every_byte_used_here_and_starts_it() {
    let world = switched_three_x_ui(3_000_000_000).await;
    let before = old_down(&world, "bob@home").await;
    // A user who exists only here has nothing to give back to.
    let db = world.panel.state.db();
    let fields = z_db::UserFields {
        username: "made-here".into(),
        status: "active".into(),
        reset_strategy: "no_reset".into(),
        ..Default::default()
    };
    let here = db
        .create_user(&fields, "u", "p", "tok", None)
        .await
        .unwrap();
    db.add_traffic(&[z_db::TrafficDelta {
        user_id: here.id,
        uplink: 1,
        downlink: 1,
    }])
    .await
    .unwrap();

    let back = run::rollback(&world.cx()).await.unwrap();
    assert_eq!(
        back.given_back,
        (3_000_000_000, 1),
        "all of it, for the one that used some"
    );
    assert_eq!(
        old_down(&world, "bob@home").await,
        before + 3_000_000_000,
        "added to the counter the old panel keeps"
    );
    assert_eq!(
        old_down(&world, "ali@home").await,
        2 * 1024 * 1024 * 1024,
        "the others are as they were"
    );

    // The old panel runs, this one is stopped, disabled and holds nothing.
    let calls = world.fake.calls();
    let this_stopped = calls
        .iter()
        .rposition(|c| c == "systemctl stop z-ui")
        .unwrap();
    let old_started = calls
        .iter()
        .rposition(|c| c == "systemctl start x-ui")
        .unwrap();
    assert!(this_stopped < old_started, "{calls:?}");
    assert!(
        calls.contains(&"systemctl disable z-ui".to_string())
            && calls.contains(&"systemctl enable x-ui".to_string())
    );
    assert!(world.fake.is_open(2053) && world.fake.is_open(443));
    assert!(db.inbounds().await.unwrap().iter().all(|i| !i.enabled));
    assert!(
        !std::fs::read_to_string(world.root.join("etc/z-ui/z-ui.env"))
            .unwrap_or_default()
            .contains("SUB_LISTEN")
    );
    let st = state::load(&world.panel.state.config().data_dir).unwrap();
    assert_eq!(st.phase, Phase::RolledBack);
    // A copy of the old database as it was is kept beside, before anything was written.
    let kept: Vec<_> = std::fs::read_dir(state::dir(&world.panel.state.config().data_dir))
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("old-database-before-rollback")
        })
        .collect();
    assert_eq!(kept.len(), 1);

    // Once is enough: there is nothing to go back from now.
    assert_eq!(
        run::rollback(&world.cx()).await.err().unwrap().code,
        "wrong_phase"
    );
}

#[tokio::test]
async fn if_the_usage_cannot_be_given_back_nothing_changes_and_this_panel_runs_on() {
    let world = switched_three_x_ui(5_000).await;
    // The old database no longer has the table the counters are in.
    change_old_database(&world, "DROP TABLE client_traffics;").await;
    let error = run::rollback(&world.cx()).await.err().unwrap();
    assert_eq!(error.code, "usage_not_saved", "{error}");
    let calls = world.fake.calls();
    let stopped = calls
        .iter()
        .rposition(|c| c == "systemctl stop z-ui")
        .unwrap();
    let started = calls
        .iter()
        .rposition(|c| c == "systemctl start z-ui")
        .unwrap();
    assert!(stopped < started, "this panel runs again: {calls:?}");
    assert!(
        !calls.contains(&"systemctl start x-ui".to_string())
            || calls
                .iter()
                .filter(|c| *c == "systemctl start x-ui")
                .count()
                == 0,
        "the old panel was not started"
    );
    assert_eq!(
        state::load(&world.panel.state.config().data_dir)
            .unwrap()
            .phase,
        Phase::Switched,
        "still switched: it can be tried again"
    );
    assert!(world
        .panel
        .state
        .db()
        .inbounds()
        .await
        .unwrap()
        .iter()
        .all(|i| i.enabled));
}

#[tokio::test]
async fn deleting_the_old_files_ends_the_way_back_and_is_refused_while_a_file_is_in_use() {
    let world = switched_three_x_ui(10).await;
    // An inbound still names a certificate file inside the old panel's directory.
    let db = world.panel.state.db();
    let trojan = db
        .inbounds()
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.tag == "inbound-2096")
        .unwrap();
    let mut stream: serde_json::Value = serde_json::from_str(&trojan.stream).unwrap();
    stream["tlsSettings"]["certificates"] = serde_json::json!([{"certificateFile": "/usr/local/x-ui/cert/fullchain.pem", "keyFile": "/usr/local/x-ui/cert/key.pem"}]);
    db.update_inbound(
        trojan.id,
        &z_db::InboundFields {
            tag: trojan.tag.clone(),
            remark: trojan.remark.clone(),
            enabled: trojan.enabled,
            listen: trojan.listen.clone(),
            port: trojan.port,
            protocol: trojan.protocol.clone(),
            settings: trojan.settings.clone(),
            stream: stream.to_string(),
            sniffing: trojan.sniffing.clone(),
        },
    )
    .await
    .unwrap();
    let refused = run::delete_old(&world.cx()).await.err().unwrap();
    assert_eq!(refused.code, "file_in_use");
    assert!(
        world.root.join("usr/local/x-ui/x-ui").exists(),
        "nothing was deleted"
    );

    // Without that, they are deleted, and there is no way back.
    db.update_inbound(
        trojan.id,
        &z_db::InboundFields {
            tag: trojan.tag.clone(),
            remark: trojan.remark.clone(),
            enabled: trojan.enabled,
            listen: trojan.listen.clone(),
            port: trojan.port,
            protocol: trojan.protocol.clone(),
            settings: trojan.settings.clone(),
            stream: trojan.stream.clone(),
            sniffing: trojan.sniffing.clone(),
        },
    )
    .await
    .unwrap();
    run::delete_old(&world.cx()).await.unwrap();
    for path in X_UI_PATHS {
        assert!(
            !world.root.join(path.trim_start_matches('/')).exists(),
            "{path}"
        );
    }
    assert!(world.fake.at("systemctl daemon-reload").is_some());
    assert_eq!(
        state::load(&world.panel.state.config().data_dir)
            .unwrap()
            .phase,
        Phase::Finished
    );
    assert_eq!(
        run::rollback(&world.cx()).await.err().unwrap().code,
        "wrong_phase"
    );
    assert_eq!(
        run::delete_old(&world.cx()).await.err().unwrap().code,
        "wrong_phase"
    );
}

#[tokio::test]
async fn a_rollback_with_the_old_files_gone_says_so_and_changes_nothing() {
    let world = switched_three_x_ui(10).await;
    std::fs::remove_dir_all(world.root.join("etc/x-ui")).unwrap();
    let error = run::rollback(&world.cx()).await.err().unwrap();
    assert_eq!(error.code, "old_panel_gone");
    assert!(
        !world
            .fake
            .calls()
            .contains(&"systemctl stop z-ui".to_string()),
        "this panel keeps serving"
    );
}

#[tokio::test]
async fn after_the_week_the_old_files_are_deleted_by_themselves_with_a_notice_the_day_before() {
    let world = switched_three_x_ui(10).await;
    let data = world.panel.state.config().data_dir.clone();
    let set_expiry = |seconds_from_now: i64| {
        let mut st = state::load(&data).unwrap();
        st.expires_at = Some(z_db::now() + seconds_from_now);
        state::save(&data, &st).unwrap();
    };
    // Days left: nothing.
    set_expiry(3 * 86_400);
    assert_eq!(run::tick(&world.cx()).await, Tick::Idle);
    // Less than a day: told, once.
    set_expiry(20 * 3600);
    assert!(
        matches!(run::tick(&world.cx()).await, Tick::Expiring { hours } if (19..=20).contains(&hours))
    );
    assert_eq!(run::tick(&world.cx()).await, Tick::Idle);
    assert!(world.root.join("usr/local/x-ui/x-ui").exists());
    // Over: deleted.
    set_expiry(-5);
    assert_eq!(run::tick(&world.cx()).await, Tick::Deleted);
    assert!(!world.root.join("usr/local/x-ui").exists());
    assert_eq!(
        run::tick(&world.cx()).await,
        Tick::Idle,
        "nothing more to do"
    );
}

#[tokio::test]
async fn the_week_ending_with_a_file_in_use_keeps_the_files_and_says_why() {
    let world = switched_three_x_ui(10).await;
    let data = world.panel.state.config().data_dir.clone();
    let db = world.panel.state.db();
    let trojan = db
        .inbounds()
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.tag == "inbound-2096")
        .unwrap();
    db.update_inbound(
        trojan.id,
        &z_db::InboundFields {
            tag: trojan.tag.clone(),
            remark: trojan.remark.clone(),
            enabled: trojan.enabled,
            listen: trojan.listen.clone(),
            port: trojan.port,
            protocol: trojan.protocol.clone(),
            settings: trojan.settings.clone(),
            stream: "{\"tlsSettings\":{\"certificates\":[{\"certificateFile\":\"/etc/x-ui/cert.pem\"}]}}".into(),
            sniffing: trojan.sniffing.clone(),
        },
    )
    .await
    .unwrap();
    let mut st = state::load(&data).unwrap();
    st.expires_at = Some(z_db::now() - 1);
    state::save(&data, &st).unwrap();
    match run::tick(&world.cx()).await {
        Tick::Kept { why } => assert!(why.contains("/etc/x-ui"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert!(world.root.join("etc/x-ui/x-ui.db").exists());
    assert_eq!(
        run::tick(&world.cx()).await,
        Tick::Idle,
        "not tried again by itself"
    );
    assert_eq!(
        state::load(&data).unwrap().phase,
        Phase::Switched,
        "the button still can"
    );
}

#[tokio::test]
async fn pasarguard_in_docker_is_stopped_taken_down_and_brought_back_with_compose() {
    let world = World::new().await;
    pasarguard(
        &world.root,
        &TempDb::new(PASARGUARD).await,
        "SQLALCHEMY_DATABASE_URL = \"sqlite+aiosqlite:////var/lib/pasarguard/db.sqlite3\"\nUVICORN_PORT = 8000\n",
    );
    let compose = "/opt/pasarguard/docker-compose.yml";
    world.fake.owns(compose, &[8000, 443, 8443, 8080, 1080]);
    world.fake.set(compose, true);
    let paths = [
        "/opt/pasarguard",
        "/var/lib/pasarguard",
        "/usr/local/bin/pasarguard",
    ];
    let before = tree(&world.root, &paths);

    let prepared = run::prepare(&world.cx(), world.found().await)
        .await
        .unwrap();
    assert_eq!(prepared.counts.users, 6);
    assert!(prepared.enable.len() >= 3);
    world.z_ui_holds(&[443, 8443, 8080, 1080, 8000]);
    run::cutover(&world.cx()).await.unwrap();
    let calls = world.fake.calls();
    let stop = calls
        .iter()
        .position(|c| c == &format!("docker compose -f {compose} stop"))
        .unwrap();
    let start = calls
        .iter()
        .position(|c| c == "systemctl restart z-ui")
        .unwrap();
    assert!(
        stop < start && calls.contains(&format!("docker compose -f {compose} down")),
        "{calls:?}"
    );
    assert_eq!(
        tree(&world.root, &paths),
        before,
        "its files stay for a week"
    );
    assert!(world
        .panel
        .state
        .db()
        .legacy_secret(crate::migrate::legacy::KIND)
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        std::fs::read_to_string(world.root.join("etc/z-ui/z-ui.env"))
            .unwrap()
            .trim(),
        "ZUI_SUB_LISTEN=0.0.0.0:8000"
    );

    // alice used 2 GB here; PasarGuard counts it in `users.used_traffic`.
    let db = world.panel.state.db();
    let alice = db
        .all_users()
        .await
        .unwrap()
        .into_iter()
        .find(|u| u.username == "alice")
        .unwrap();
    db.add_traffic(&[z_db::TrafficDelta {
        user_id: alice.id,
        uplink: 0,
        downlink: 2_000_000_000,
    }])
    .await
    .unwrap();
    let before_used = old_pasarguard_used(&world, "alice").await;
    let back = run::rollback(&world.cx()).await.unwrap();
    assert_eq!(back.given_back, (2_000_000_000, 1));
    assert_eq!(
        old_pasarguard_used(&world, "alice").await,
        before_used + 2_000_000_000
    );
    assert!(world
        .fake
        .calls()
        .contains(&format!("docker compose -f {compose} up -d")));
    assert!(world.fake.is_open(8000));
}

async fn old_pasarguard_used(world: &World, username: &str) -> i64 {
    let source = z_db::Source::open(
        &world
            .root
            .join("var/lib/pasarguard/db.sqlite3")
            .to_string_lossy(),
    )
    .await
    .unwrap();
    source
        .query(&format!(
            "SELECT CAST(used_traffic AS TEXT) FROM users WHERE username = '{username}'"
        ))
        .await
        .unwrap()[0][0]
        .clone()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn the_certificate_the_old_panel_served_with_becomes_this_panels() {
    let world = World::with_three_x_ui().await;
    let key = rcgen::generate_simple_self_signed(vec!["panel.example.com".to_string()]).unwrap();
    put(
        &world.root,
        "/root/cert/fullchain.pem",
        key.cert.pem().as_bytes(),
    );
    put(
        &world.root,
        "/root/cert/key.pem",
        key.signing_key.serialize_pem().as_bytes(),
    );
    change_old_database(
        &world,
        "INSERT INTO settings (id, key, value) VALUES (90, 'webCertFile', '/root/cert/fullchain.pem'), (91, 'webKeyFile', '/root/cert/key.pem');",
    )
    .await;
    let found = world.found().await;
    assert!(found.cert.is_some());
    run::prepare(&world.cx(), found).await.unwrap();
    let certificates = world.panel.state.db().certificates().await.unwrap();
    assert!(certificates
        .iter()
        .any(|c| c.domains.contains("panel.example.com")));
    let stored = crate::panel_settings::load(world.panel.state.db()).await;
    assert!(
        stored.certificate_id.is_some(),
        "the panel serves HTTPS with it"
    );
}

#[tokio::test]
async fn the_panel_shows_the_takeover_and_goes_back_or_lets_go_when_asked() {
    use axum::http::StatusCode;
    let world = World::with_three_x_ui().await;
    world
        .panel
        .state
        .set_host(world.fake.clone(), world.root.clone());
    let owner = world.panel.set_up("").await;
    let panel = &world.panel;
    assert_eq!(
        panel.get("/api/takeover", Some(&owner)).await.json(),
        serde_json::Value::Null
    );
    let nothing = panel
        .send("POST", "/api/takeover/rollback", Some(&owner), None)
        .await;
    assert_eq!(nothing.status, StatusCode::NOT_FOUND);

    run::prepare(&world.cx(), world.found().await)
        .await
        .unwrap();
    let prepared = panel.get("/api/takeover", Some(&owner)).await.json();
    assert_eq!(
        (prepared["phase"].clone(), prepared["can_rollback"].clone()),
        ("prepared".into(), false.into())
    );
    let early = panel
        .send("POST", "/api/takeover/rollback", Some(&owner), None)
        .await;
    assert_eq!(
        (early.status, early.json()["error"].clone()),
        (StatusCode::CONFLICT, "wrong_phase".into())
    );

    world.z_ui_holds(&[443, 8443, 2096, 1080, 2097]);
    run::cutover(&world.cx()).await.unwrap();
    let view = panel.get("/api/takeover", Some(&owner)).await.json();
    assert_eq!(view["phase"], "switched");
    assert_eq!(view["kind"], "3x-ui");
    assert_eq!(
        (
            view["can_rollback"].clone(),
            view["can_delete"].clone(),
            view["users"].clone()
        ),
        (true.into(), true.into(), 10.into())
    );
    assert!(view["expires_at"].as_i64().unwrap() > z_db::now() + 6 * 86_400);

    // "I made a mistake" starts the way back on its own, outside this panel.
    let started = panel
        .send("POST", "/api/takeover/rollback", Some(&owner), None)
        .await;
    assert_eq!(started.status, StatusCode::ACCEPTED, "{}", started.body);
    assert_eq!(started.json()["started"], true);
    let spawned = world
        .fake
        .calls()
        .into_iter()
        .find(|c| c.starts_with("detached "))
        .unwrap();
    assert!(
        spawned.ends_with("takeover rollback --yes") && spawned.contains("--data-dir"),
        "{spawned}"
    );

    // Deleting the old files ends the way back.
    let done = panel
        .send("POST", "/api/takeover/delete-old", Some(&owner), None)
        .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert_eq!(
        (
            done.json()["phase"].clone(),
            done.json()["can_rollback"].clone()
        ),
        ("finished".into(), false.into())
    );
    let late = panel
        .send("POST", "/api/takeover/rollback", Some(&owner), None)
        .await;
    assert_eq!(late.status, StatusCode::CONFLICT);
}
