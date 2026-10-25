//! The steps of replacing a panel with this one, in the order that keeps
//! the time users are cut off short and every step undoable:
//!
//!  * [`prepare`] — while the old panel still serves everyone: copy its
//!    database and bring users, groups, inbounds (switched off) and settings
//!    over;
//!  * [`cutover`] — the short part: stop the old panel, bring over what
//!    changed since (usage, new users), switch the inbounds on, start this
//!    panel, and check that every port answers. If anything fails, the old
//!    panel is started again. The old panel's files are not touched: they
//!    stay where they are for [`WINDOW`], stopped;
//!  * [`rollback`] — "I made a mistake": stop this panel, add what the users
//!    used here to the old panel's own counters, and start the old panel;
//!  * [`delete_old`] — delete the old panel's files (also done by itself when
//!    the week is over), which ends the way back.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use z_db::{Counters, Inbound, InboundFields, NewAudit, Source};

use super::detect::{at, Detected, Service};
use super::host::{compose, systemctl, Host};
use super::state::{self, MovedUser, Phase, State, WINDOW};
use crate::migrate::apply::{apply, certificate_id, Conflict, Options};
use crate::migrate::model::{Kind, PlannedCertificate, Warning};
use crate::server::AppState;

/// The unit this panel runs as.
const UNIT: &str = "z-ui";
/// Where the service reads its environment from, under the root.
const ENV_FILE: &str = "etc/z-ui/z-ui.env";

/// Why a step did not happen: a word for scripts, and what to tell people.
#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
}

impl Failure {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}

impl From<z_db::DbError> for Failure {
    fn from(error: z_db::DbError) -> Self {
        Failure::new("database", error.to_string())
    }
}

impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Failure::new("io", error.to_string())
    }
}

type Result<T> = std::result::Result<T, Failure>;

/// What a step works with.
pub struct Context<'a> {
    /// Where the machine's files are (`/`, or a directory made for trying).
    pub root: &'a Path,
    pub host: &'a dyn Host,
    /// This panel, offline: its database and settings, no listener or core.
    pub state: &'a AppState,
    /// Where progress is told.
    pub say: &'a (dyn Fn(&str) + Sync),
}

impl Context<'_> {
    fn data(&self) -> PathBuf {
        self.state.config().data_dir.clone()
    }

    fn at(&self, path: &Path) -> PathBuf {
        at(self.root, path)
    }

    fn say(&self, text: &str) {
        (self.say)(text);
    }

    async fn audit(&self, action: &str, target: &str, detail: String) {
        let _ = self
            .state
            .db()
            .add_audit(&NewAudit {
                admin_id: None,
                admin_name: "server".into(),
                via: "takeover".into(),
                ip: String::new(),
                action: action.into(),
                target: target.into(),
                detail,
                status: 200,
            })
            .await;
    }
}

/// `tries` looks, a quarter of a second apart, until `done` is true.
fn wait(cx: &Context<'_>, tries: u32, mut done: impl FnMut() -> bool) -> bool {
    if cx.host.dry() {
        return true;
    }
    for _ in 0..tries {
        if done() {
            return true;
        }
        cx.host.sleep(Duration::from_millis(250));
    }
    done()
}

/// Paths that are never removed, whatever a state file says.
const NEVER: [&str; 14] = [
    "/",
    "/etc",
    "/usr",
    "/var",
    "/opt",
    "/root",
    "/home",
    "/usr/local",
    "/var/lib",
    "/usr/bin",
    "/usr/local/bin",
    "/etc/systemd/system",
    "/usr/lib/systemd/system",
    "/var/log",
];

fn safe_to_remove(path: &Path) -> bool {
    path.is_absolute()
        && path.components().count() >= 3
        && !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        && !NEVER.iter().any(|never| path == Path::new(never))
}

fn set_env(root: &Path, key: &str, value: Option<&str>) -> std::io::Result<()> {
    let path = root.join(ENV_FILE);
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let prefix = format!("{key}=");
    let mut lines: Vec<String> = old
        .lines()
        .filter(|line| !line.trim_start().starts_with(&prefix))
        .map(str::to_string)
        .collect();
    if let Some(value) = value {
        lines.push(format!("{key}={value}"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    std::fs::write(path, text)
}

fn inbound_fields(inbound: &Inbound, enabled: bool) -> InboundFields {
    InboundFields {
        tag: inbound.tag.clone(),
        remark: inbound.remark.clone(),
        enabled,
        listen: inbound.listen.clone(),
        port: inbound.port,
        protocol: inbound.protocol.clone(),
        settings: inbound.settings.clone(),
        stream: inbound.stream.clone(),
        sniffing: inbound.sniffing.clone(),
    }
}

async fn set_enabled(cx: &Context<'_>, tags: &[String], on: bool) -> Result<()> {
    let db = cx.state.db();
    for inbound in db.inbounds().await? {
        if tags.contains(&inbound.tag) && inbound.enabled != on {
            db.update_inbound(inbound.id, &inbound_fields(&inbound, on))
                .await?;
        }
    }
    Ok(())
}

/// Of the inbounds wanted on, those that can be switched on together (the
/// core's own check, port by port), and why the others cannot.
async fn enable_set(cx: &Context<'_>, wanted: &[String]) -> Result<(Vec<String>, Vec<Warning>)> {
    let db = cx.state.db();
    let settings = crate::core_settings::load(db).await;
    let mut inbounds = db.inbounds().await?;
    for inbound in inbounds.iter_mut().filter(|i| wanted.contains(&i.tag)) {
        inbound.enabled = false;
    }
    let (mut ok, mut warnings) = (Vec::new(), Vec::new());
    for tag in wanted {
        let Some(position) = inbounds.iter().position(|i| &i.tag == tag) else {
            continue;
        };
        inbounds[position].enabled = true;
        match crate::core_config::compile(cx.state, &inbounds, &settings).await {
            Ok(_) => ok.push(tag.clone()),
            Err(error) => {
                inbounds[position].enabled = false;
                warnings.push(Warning::new(
                    "inbound",
                    tag.as_str(),
                    "takeover_not_enabled",
                    format!("it stays switched off: {}", error.message),
                ));
            }
        }
    }
    Ok((ok, warnings))
}

async fn ports_of(cx: &Context<'_>, tags: &[String]) -> Result<Vec<(u16, bool)>> {
    let inbounds = cx.state.db().inbounds().await?;
    Ok(inbounds
        .iter()
        .filter(|i| tags.contains(&i.tag))
        .filter_map(|i| {
            let stream: serde_json::Value = serde_json::from_str(&i.stream).unwrap_or_default();
            Some((
                u16::try_from(i.port).ok()?,
                crate::core_config::uses_udp(&i.protocol, &stream),
            ))
        })
        .collect())
}

/// What a panel is stopped or started with.
fn old_services(cx: &Context<'_>, detected: &Detected, verb: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for service in &detected.services {
        let done = match service {
            Service::Systemd { unit } => systemctl(cx.host, &[verb, unit]),
            Service::Compose { file } => compose(cx.host, &file.to_string_lossy(), &[verb]),
        };
        if let Err(error) = done {
            problems.push(error);
        }
    }
    problems
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn reading_error<E: fmt::Debug>(error: E) -> Failure {
    Failure::new(
        "unreadable",
        format!("the old database cannot be read: {error:?}"),
    )
}

// ------------------------------------------------------------------ prepare

pub struct Prepared {
    pub counts: crate::migrate::apply::Counts,
    pub enable: Vec<String>,
    pub warnings: Vec<Warning>,
}

async fn import_panel_certificate(
    cx: &Context<'_>,
    detected: &Detected,
    warnings: &mut Vec<Warning>,
) -> Option<i64> {
    let (cert, key) = detected.cert.as_ref()?;
    let planned = PlannedCertificate::Files {
        cert: text(&cx.at(cert)),
        key: text(&cx.at(key)),
    };
    let mut known = cx.state.db().certificates().await.ok()?;
    let mut added = 0;
    match certificate_id(cx.state, &planned, "old panel", &mut known, &mut added).await {
        Ok(id) => {
            let mut stored = crate::panel_settings::load(cx.state.db()).await;
            if stored.certificate_id.is_none() {
                stored.certificate_id = Some(id);
                let _ = crate::panel_settings::save(cx.state.db(), &stored).await;
            }
            Some(id)
        }
        Err(why) => {
            warnings.push(Warning::new(
                "settings",
                "certificate",
                "takeover_certificate",
                format!("the old panel's certificate was not taken, so this panel serves plain HTTP until one is made: {why}"),
            ));
            None
        }
    }
}

/// Everything that can be done while the old panel still serves users.
pub async fn prepare(cx: &Context<'_>, detected: Detected) -> Result<Prepared> {
    if let Some(why) = &detected.unsupported {
        return Err(Failure::new("unsupported", why.clone()));
    }
    let data = cx.data();
    let previous = state::load(&data);
    if let Some(st) = &previous {
        if st.phase == Phase::Switched {
            return Err(Failure::new(
                "already_switched",
                "this panel already took over; go back first (`z-ui takeover rollback`), or delete the old panel's data to keep it",
            ));
        }
    }
    let database = detected
        .database
        .clone()
        .ok_or_else(|| Failure::new("no_database", "no database was found"))?;
    let folder = state::dir(&data);
    std::fs::create_dir_all(&folder)?;
    // Two steps in one second must not share a file name.
    let stamp = format!("{}-{:04x}", z_db::now(), rand::random::<u16>());

    cx.say("copying the old panel's database");
    let snapshot = folder.join(format!("snapshot-{stamp}.db"));
    Source::open(&text(&cx.at(&database)))
        .await
        .map_err(|e| {
            Failure::new(
                "unreadable",
                format!("the old database cannot be opened: {e}"),
            )
        })?
        .snapshot(&snapshot)
        .await?;
    let xray: Option<serde_json::Value> = detected
        .xray_config
        .as_ref()
        .and_then(|p| std::fs::read(cx.at(p)).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let plan = crate::migrate::read(&text(&snapshot), Some(detected.kind), xray.as_ref())
        .await
        .map_err(reading_error)?;

    let db = cx.state.db();
    let before: HashSet<String> = db
        .all_users()
        .await?
        .into_iter()
        .map(|u| u.username)
        .collect();
    let refresh: HashSet<String> = previous
        .as_ref()
        .map(|s| s.created_users.iter().cloned().collect())
        .unwrap_or_default();
    let mut warnings = Vec::new();
    cx.say("bringing users, groups, inbounds and settings over (the inbounds stay off)");
    let certificate_id = import_panel_certificate(cx, &detected, &mut warnings).await;
    let report = apply(
        cx.state,
        plan.clone(),
        &Options {
            enable_inbounds: false,
            on_conflict: Conflict::Skip,
            sync_core: false,
            refresh_usernames: refresh,
            ..Options::default()
        },
    )
    .await
    .map_err(|e| Failure::new("import", e.message))?;
    warnings.extend(report.warnings.clone());

    let mut created: Vec<String> = db
        .all_users()
        .await?
        .into_iter()
        .map(|u| u.username)
        .filter(|name| !before.contains(name))
        .collect();
    if let Some(previous) = &previous {
        created.extend(previous.created_users.iter().cloned());
    }
    created.sort();
    created.dedup();

    let wanted: Vec<String> = plan
        .inbounds
        .iter()
        .filter(|p| p.enabled)
        .filter_map(|p| report.tag_map.iter().find(|(from, _)| *from == p.tag))
        .map(|(_, tag)| tag.clone())
        .collect();
    let (enable, more) = enable_set(cx, &wanted).await?;
    warnings.extend(more);

    if let Some(old) = previous.as_ref() {
        let _ = std::fs::remove_file(&old.snapshot);
    }
    let st = State {
        phase: Phase::Prepared,
        prepared_at: z_db::now(),
        switched_at: None,
        expires_at: None,
        ended_at: None,
        snapshot,
        created_users: created,
        moved: Vec::new(),
        inbound_ports: ports_of(cx, &enable).await?,
        enable_tags: enable.clone(),
        sub_listen: None,
        certificate_id,
        notified: false,
        detected: detected.clone(),
    };
    state::save(&data, &st)?;
    cx.audit(
        "takeover.prepare",
        detected.kind.label(),
        format!(
            "{} users, {} inbounds",
            report.imported.users, report.imported.inbounds
        ),
    )
    .await;
    Ok(Prepared {
        counts: report.imported,
        enable,
        warnings,
    })
}

// ------------------------------------------------------------------ cutover

pub struct Switched {
    pub warnings: Vec<Warning>,
    pub refreshed: usize,
    pub added_users: usize,
    pub dropped_users: usize,
    /// When the old panel's files are deleted by themselves.
    pub expires_at: i64,
}

struct Moved {
    refreshed: usize,
    added: usize,
    dropped: usize,
    enable: Vec<String>,
    warnings: Vec<Warning>,
}

/// What happens while no panel serves: the last of the old panel's data, this
/// panel started, and every port asked.
async fn move_over(cx: &Context<'_>, st: &mut State, detected: &Detected) -> Result<Moved> {
    let data = cx.data();
    let database = detected.database.clone().expect("prepared with a database");
    let folder = state::dir(&data);
    let stamp = format!("{}-{:04x}", z_db::now(), rand::random::<u16>());

    // The old database as it was left, read once more for what changed since
    // the first copy.
    let snapshot = folder.join(format!("final-{stamp}.db"));
    Source::open(&text(&cx.at(&database)))
        .await?
        .snapshot(&snapshot)
        .await?;
    let xray: Option<serde_json::Value> = detected
        .xray_config
        .as_ref()
        .and_then(|p| std::fs::read(cx.at(p)).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let plan = crate::migrate::read(&text(&snapshot), Some(detected.kind), xray.as_ref())
        .await
        .map_err(reading_error)?;
    let _ = std::fs::remove_file(&snapshot);

    let db = cx.state.db();
    let before: HashMap<String, i64> = db
        .all_users()
        .await?
        .into_iter()
        .map(|u| (u.username, u.id))
        .collect();
    let report = apply(
        cx.state,
        plan.clone(),
        &Options {
            enable_inbounds: false,
            on_conflict: Conflict::Skip,
            sync_core: false,
            refresh_usernames: st.created_users.iter().cloned().collect(),
            ..Options::default()
        },
    )
    .await
    .map_err(|e| Failure::new("import", e.message))?;

    // A user the old panel dropped meanwhile is dropped here too.
    let in_plan: HashSet<&str> = plan.users.iter().map(|u| u.username.as_str()).collect();
    let mut dropped = 0;
    for name in &st.created_users {
        if let (false, Some(id)) = (in_plan.contains(name.as_str()), before.get(name)) {
            db.delete_user(*id).await?;
            dropped += 1;
        }
    }
    let after = db.all_users().await?;
    let new_users: Vec<String> = after
        .iter()
        .filter(|u| !before.contains_key(&u.username))
        .map(|u| u.username.clone())
        .collect();
    let added = new_users.len();
    let names: HashSet<&str> = after.iter().map(|u| u.username.as_str()).collect();
    st.created_users.retain(|n| names.contains(n.as_str()));
    st.created_users.extend(new_users);
    // What each user used, as of now: what it uses from here on is what would
    // have to be given back.
    let lifetime: HashMap<&str, i64> = after
        .iter()
        .map(|u| (u.username.as_str(), u.lifetime_traffic))
        .collect();
    st.moved = plan
        .users
        .iter()
        .filter(|u| st.created_users.contains(&u.username))
        .filter_map(|u| {
            Some(MovedUser {
                username: u.username.clone(),
                source_name: u.source_name.clone(),
                lifetime: *lifetime.get(u.username.as_str())?,
            })
        })
        .collect();

    // Inbounds: what was meant to be on, and any the old panel added meanwhile.
    let mut wanted = st.enable_tags.clone();
    for planned in plan.inbounds.iter().filter(|p| p.enabled) {
        if let Some((_, tag)) = report.tag_map.iter().find(|(from, _)| *from == planned.tag) {
            if !wanted.contains(tag) {
                wanted.push(tag.clone());
            }
        }
    }
    let (enable, mut warnings) = enable_set(cx, &wanted).await?;
    warnings.extend(
        report
            .warnings
            .iter()
            .filter(|w| w.code.starts_with("skipped_") && w.scope != "user")
            .cloned(),
    );
    set_enabled(cx, &enable, true).await?;
    st.inbound_ports = ports_of(cx, &enable).await?;
    st.enable_tags = enable.clone();

    // Subscription links keep their address: this panel answers on the port
    // the old one did.
    let panel_port = cx.state.config().listen.port();
    st.sub_listen = detected
        .sub
        .as_ref()
        .filter(|sub| sub.port != panel_port)
        .filter(|sub| {
            let taken = st
                .inbound_ports
                .iter()
                .any(|(port, udp)| !udp && *port == sub.port);
            if taken {
                warnings.push(Warning::new(
                    "settings",
                    "subscription",
                    "takeover_sub_port_taken",
                    format!("an inbound listens on port {}, where the old subscriptions were; they are served on this panel's own port instead", sub.port),
                ));
            }
            !taken
        })
        .map(|sub| sub.text());
    set_env(cx.root, "ZUI_SUB_LISTEN", st.sub_listen.as_deref())?;

    cx.say("starting this panel");
    systemctl(cx.host, &["restart", UNIT]).map_err(|e| Failure::new("start_failed", e))?;
    if !wait(cx, 120, || cx.host.tcp_open(panel_port)) {
        return Err(Failure::new(
            "not_listening",
            format!("this panel did not answer on port {panel_port}"),
        ));
    }
    let mut ports: Vec<u16> = st
        .inbound_ports
        .iter()
        .filter(|(_, udp)| !udp)
        .map(|(p, _)| *p)
        .collect();
    if let Some(sub) = detected.sub.as_ref().filter(|_| st.sub_listen.is_some()) {
        ports.push(sub.port);
    }
    if !wait(cx, 60, || ports.iter().all(|p| cx.host.tcp_open(*p))) {
        let closed: Vec<String> = ports
            .iter()
            .filter(|p| !cx.host.tcp_open(**p))
            .map(u16::to_string)
            .collect();
        return Err(Failure::new(
            "not_listening",
            format!(
                "nothing answers on port {} of this panel",
                closed.join(", ")
            ),
        ));
    }
    Ok(Moved {
        refreshed: report.imported.refreshed,
        added,
        dropped,
        enable,
        warnings,
    })
}

/// The old panel started again and this one put aside, after a step failed.
async fn undo(cx: &Context<'_>, st: &State, detected: &Detected) {
    cx.say("something failed: starting the old panel again");
    let _ = systemctl(cx.host, &["stop", UNIT]);
    let _ = set_enabled(cx, &st.enable_tags, false).await;
    let _ = set_env(cx.root, "ZUI_SUB_LISTEN", None);
    old_services(cx, detected, "start");
}

/// The old services off for good (not to start again at boot): units
/// disabled, containers taken down. Files stay.
fn retire_old(cx: &Context<'_>, detected: &Detected) {
    for service in &detected.services {
        match service {
            Service::Systemd { unit } => {
                let _ = systemctl(cx.host, &["disable", unit]);
            }
            Service::Compose { file } => {
                let _ = compose(cx.host, &file.to_string_lossy(), &["down"]);
            }
        }
    }
}

/// The short part. Users are cut off from the moment the old panel stops
/// until this one answers: a few seconds.
pub async fn cutover(cx: &Context<'_>) -> Result<Switched> {
    let data = cx.data();
    let mut st = state::load(&data).ok_or_else(|| {
        Failure::new(
            "not_prepared",
            "nothing is prepared; run `z-ui takeover prepare` first",
        )
    })?;
    if st.phase != Phase::Prepared {
        return Err(Failure::new(
            "wrong_phase",
            format!("the takeover is {:?}, not prepared", st.phase).to_lowercase(),
        ));
    }
    let detected = st.detected.clone();
    cx.say("checking that the inbounds can start here");
    let (enable, _) = enable_set(cx, &st.enable_tags).await?;
    let mut hold: Vec<u16> = ports_of(cx, &enable)
        .await?
        .iter()
        .filter(|(_, udp)| !udp)
        .map(|(p, _)| *p)
        .collect();
    if let Some(sub) = &detected.sub {
        hold.push(sub.port);
    }

    cx.say("stopping the old panel: its users are cut off now, until this panel answers");
    let problems = old_services(cx, &detected, "stop");
    if !problems.is_empty() {
        old_services(cx, &detected, "start");
        return Err(Failure::new("stop_failed", problems.join("; ")));
    }
    // Its own port is not wanted here; the inbounds' and the subscription's are.
    if !wait(cx, 60, || hold.iter().all(|p| !cx.host.tcp_open(*p))) {
        old_services(cx, &detected, "start");
        return Err(Failure::new(
            "ports_busy",
            "the ports of the inbounds are still held after the old panel was stopped (something else uses them); the old panel is running again",
        ));
    }
    let moved = match move_over(cx, &mut st, &detected).await {
        Ok(moved) => moved,
        Err(error) => {
            undo(cx, &st, &detected).await;
            return Err(error);
        }
    };

    cx.say("this panel answers; the old one stays where it is, stopped, for a week");
    retire_old(cx, &detected);
    let now = z_db::now();
    st.phase = Phase::Switched;
    st.switched_at = Some(now);
    st.expires_at = Some(now + WINDOW);
    st.enable_tags = moved.enable;
    state::save(&data, &st)?;
    cx.audit(
        "takeover.cutover",
        detected.kind.label(),
        format!(
            "{} refreshed, {} new, {} dropped",
            moved.refreshed, moved.added, moved.dropped
        ),
    )
    .await;
    Ok(Switched {
        warnings: moved.warnings,
        refreshed: moved.refreshed,
        added_users: moved.added,
        dropped_users: moved.dropped,
        expires_at: now + WINDOW,
    })
}

// ----------------------------------------------------------------- rollback

pub struct RolledBack {
    /// Bytes of usage given back to the old panel, and for how many users.
    pub given_back: (i64, usize),
    pub warnings: Vec<Warning>,
}

/// What the old panel needs to still be there to be started again: its
/// database and what starts it.
pub fn old_panel_present(root: &Path, detected: &Detected) -> bool {
    let database = detected
        .database
        .as_ref()
        .is_some_and(|p| at(root, p).is_file());
    let services = detected.services.iter().all(|service| match service {
        Service::Compose { file } => at(root, file).is_file(),
        Service::Systemd { .. } => detected
            .remove
            .iter()
            .any(|p| p.to_string_lossy().ends_with(".service") && at(root, p).is_file()),
    });
    database && services
}

/// Usage counted here since this panel took over, per user, as the old panel
/// names them.
async fn used_here(cx: &Context<'_>, st: &State) -> Result<Vec<(String, i64)>> {
    let now: HashMap<String, i64> = cx
        .state
        .db()
        .all_users()
        .await?
        .into_iter()
        .map(|u| (u.username, u.lifetime_traffic))
        .collect();
    Ok(st
        .moved
        .iter()
        .filter_map(|m| {
            let delta = now.get(&m.username)? - m.lifetime;
            (delta > 0).then(|| (m.source_name.clone(), delta))
        })
        .collect())
}

/// "I made a mistake": the old panel back, with everything its users used
/// meanwhile, and this one stopped.
pub async fn rollback(cx: &Context<'_>) -> Result<RolledBack> {
    let data = cx.data();
    let mut st = state::load(&data)
        .ok_or_else(|| Failure::new("no_takeover", "there is no takeover to go back from"))?;
    let detected = st.detected.clone();
    match st.phase {
        Phase::Prepared => {
            // The old panel was never stopped: nothing to put back.
            st.phase = Phase::RolledBack;
            st.ended_at = Some(z_db::now());
            state::save(&data, &st)?;
            return Ok(RolledBack {
                given_back: (0, 0),
                warnings: Vec::new(),
            });
        }
        Phase::Switched => {}
        other => {
            return Err(Failure::new(
                "wrong_phase",
                format!("the takeover is {other:?}; there is nothing to go back from")
                    .to_lowercase(),
            ))
        }
    }
    if !old_panel_present(cx.root, &detected) {
        return Err(Failure::new(
            "old_panel_gone",
            "the old panel's files are gone, so there is nothing to go back to",
        ));
    }
    let database = detected.database.clone().expect("prepared with a database");
    let old_database = cx.at(&database);

    cx.say("stopping this panel, so nothing more is counted and the old one gets its ports");
    let _ = systemctl(cx.host, &["stop", UNIT]);
    let amounts = used_here(cx, &st).await?;

    // Nothing used here is lost: a copy of the old database first, then the
    // counters. If they cannot be written, this panel runs again.
    cx.say("giving the old panel what its users used here");
    let folder = state::dir(&data);
    std::fs::create_dir_all(&folder)?;
    let copy = folder.join(format!("old-database-before-rollback-{}", z_db::now()));
    std::fs::create_dir_all(&copy)?;
    if let Some(name) = old_database.file_name() {
        std::fs::copy(&old_database, copy.join(name))?;
    }
    let counters = match detected.kind {
        Kind::ThreeXUi => Counters::ThreeXUi,
        _ => Counters::Users,
    };
    let found = match z_db::add_usage(&old_database, counters, &amounts).await {
        Ok(found) => found,
        Err(error) => {
            cx.say("the usage could not be given back: this panel keeps serving");
            let _ = systemctl(cx.host, &["start", UNIT]);
            return Err(Failure::new(
                "usage_not_saved",
                format!("what the users used here could not be written to the old panel ({error}); nothing was changed, and this panel runs again"),
            ));
        }
    };
    let _ = systemctl(cx.host, &["disable", UNIT]);
    set_enabled(cx, &st.enable_tags, false).await?;
    set_env(cx.root, "ZUI_SUB_LISTEN", None)?;

    cx.say("starting the old panel");
    let _ = systemctl(cx.host, &["daemon-reload"]);
    let mut warnings = Vec::new();
    if found < amounts.len() {
        warnings.push(Warning::new(
            "user",
            "",
            "takeover_usage_unmatched",
            format!(
                "{} users that used something here are not in the old panel any more",
                amounts.len() - found
            ),
        ));
    }
    for service in &detected.services {
        let done = match service {
            Service::Systemd { unit } => {
                let _ = systemctl(cx.host, &["enable", unit]);
                systemctl(cx.host, &["start", unit])
            }
            Service::Compose { file } => compose(cx.host, &file.to_string_lossy(), &["up", "-d"]),
        };
        if let Err(error) = done {
            warnings.push(Warning::new(
                "settings",
                "old panel",
                "takeover_start_failed",
                error,
            ));
        }
    }
    let ports: Vec<u16> = st
        .inbound_ports
        .iter()
        .filter(|(_, udp)| !udp)
        .map(|(p, _)| *p)
        .collect();
    if !wait(cx, 120, || ports.iter().all(|p| cx.host.tcp_open(*p))) {
        warnings.push(Warning::new(
            "settings",
            "old panel",
            "takeover_not_listening",
            "the old panel was started but not every inbound port answers yet",
        ));
    }
    st.phase = Phase::RolledBack;
    st.ended_at = Some(z_db::now());
    state::save(&data, &st)?;
    let total: i64 = amounts.iter().map(|(_, bytes)| bytes).sum();
    cx.audit(
        "takeover.rollback",
        detected.kind.label(),
        format!("{total} bytes of usage given back for {found} users"),
    )
    .await;
    Ok(RolledBack {
        given_back: (total, found),
        warnings,
    })
}

// --------------------------------------------------------------- delete old

/// Delete the old panel's files, which ends the way back. Refused while an
/// inbound still names a file in them.
pub async fn delete_old(cx: &Context<'_>) -> Result<Vec<Warning>> {
    let data = cx.data();
    let mut st = state::load(&data)
        .ok_or_else(|| Failure::new("no_takeover", "there is no takeover to finish"))?;
    if st.phase != Phase::Switched {
        return Err(Failure::new(
            "wrong_phase",
            "the old panel's data is only deleted after this panel has taken over, and once",
        ));
    }
    let detected = st.detected.clone();
    let streams: Vec<String> = cx
        .state
        .db()
        .inbounds()
        .await?
        .into_iter()
        .map(|i| i.stream)
        .collect();
    for path in detected.remove.iter().filter(|p| cx.at(p).is_dir()) {
        let name = text(path);
        if streams.iter().any(|s| s.contains(&name)) {
            return Err(Failure::new(
                "file_in_use",
                format!("an inbound still names a file in {name}, so it is kept; move the certificate to the store first"),
            ));
        }
    }
    for path in &detected.remove {
        if !safe_to_remove(path) {
            return Err(Failure::new(
                "unsafe_path",
                format!("{} is never removed", path.display()),
            ));
        }
        if cx.data().starts_with(cx.at(path)) {
            return Err(Failure::new(
                "unsafe_path",
                format!(
                    "this panel's data is inside {}, which would be removed",
                    path.display()
                ),
            ));
        }
    }
    retire_old(cx, &detected);
    let mut warnings = Vec::new();
    for path in &detected.remove {
        let target = cx.at(path);
        let is_dir = target
            .symlink_metadata()
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
        let done = if is_dir {
            std::fs::remove_dir_all(&target)
        } else {
            std::fs::remove_file(&target)
        };
        if let Err(error) = done {
            if target.symlink_metadata().is_ok() {
                warnings.push(Warning::new(
                    "settings",
                    text(path),
                    "takeover_not_removed",
                    format!("{} could not be removed: {error}", path.display()),
                ));
            }
        }
    }
    let _ = systemctl(cx.host, &["daemon-reload"]);
    let _ = std::fs::remove_file(&st.snapshot);
    st.phase = Phase::Finished;
    st.ended_at = Some(z_db::now());
    st.expires_at = None;
    state::save(&data, &st)?;
    cx.audit("takeover.delete_old", detected.kind.label(), String::new())
        .await;
    Ok(warnings)
}

/// What the periodic check found.
#[derive(Debug, PartialEq, Eq)]
pub enum Tick {
    Idle,
    /// Less than a day is left of the week.
    Expiring {
        hours: i64,
    },
    /// The week was over and the old panel's files were deleted.
    Deleted,
    /// The week was over but the files are kept (see why); only the button
    /// deletes them now.
    Kept {
        why: String,
    },
}

/// Once the week is over the old panel's files are deleted, and a day before
/// the admin is told.
pub async fn tick(cx: &Context<'_>) -> Tick {
    let data = cx.data();
    let Some(mut st) = state::load(&data) else {
        return Tick::Idle;
    };
    let (Phase::Switched, Some(expires)) = (st.phase, st.expires_at) else {
        return Tick::Idle;
    };
    let now = z_db::now();
    if now >= expires {
        return match delete_old(cx).await {
            Ok(_) => Tick::Deleted,
            Err(error) => {
                // Not tried again by itself.
                st.expires_at = None;
                let _ = state::save(&data, &st);
                Tick::Kept { why: error.message }
            }
        };
    }
    if expires - now <= 86_400 && !st.notified {
        st.notified = true;
        let _ = state::save(&data, &st);
        return Tick::Expiring {
            hours: ((expires - now) / 3600).max(1),
        };
    }
    Tick::Idle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_of_the_system_is_ever_a_path_to_remove() {
        for bad in [
            "/",
            "/etc",
            "/usr",
            "/var/lib",
            "/opt",
            "/usr/local/bin",
            "relative/x",
            "/etc/../usr/x",
            "/etc/systemd/system",
            "/var/log",
        ] {
            assert!(!safe_to_remove(Path::new(bad)), "{bad}");
        }
        for good in [
            "/etc/x-ui",
            "/usr/local/x-ui",
            "/opt/pasarguard",
            "/var/lib/marzban",
            "/usr/bin/x-ui",
            "/etc/systemd/system/x-ui.service",
        ] {
            assert!(safe_to_remove(Path::new(good)), "{good}");
        }
    }

    #[test]
    fn the_environment_file_gets_a_line_and_loses_it_without_touching_the_others() {
        let root = std::env::temp_dir().join(format!(
            "z-env-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(root.join("etc/z-ui")).unwrap();
        std::fs::write(
            root.join(ENV_FILE),
            "ZUI_LANG=fa\nZUI_SUB_LISTEN=0.0.0.0:1\n",
        )
        .unwrap();
        set_env(&root, "ZUI_SUB_LISTEN", Some("0.0.0.0:2096")).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join(ENV_FILE)).unwrap(),
            "ZUI_LANG=fa\nZUI_SUB_LISTEN=0.0.0.0:2096\n"
        );
        set_env(&root, "ZUI_SUB_LISTEN", None).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join(ENV_FILE)).unwrap(),
            "ZUI_LANG=fa\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
