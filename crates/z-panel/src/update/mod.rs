//! Updating the panel from its releases: look for a newer version, download
//! it, check that it is what the maintainers published, and put it in place
//! of the running program. A release carries one archive per kind of
//! machine, a `SHA256SUMS` file naming each archive's hash, and a signature
//! of that file made with a key whose public half is built into every
//! panel; so a changed archive, a changed sums file or a release signed by
//! anyone else is refused before anything is touched.

#[cfg(test)]
pub(crate) mod tests;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use axum::http::StatusCode;

use crate::error::ApiError;
use crate::notify::http::Proxy;
use crate::notify::Event;
use crate::outbound::{self, Request};
use crate::server::AppState;

pub const REPO: &str = "ZeroNetTeam/Z-UI";
/// The public half of the key releases are signed with: Ed25519, 32 bytes,
/// in base64. The private half is kept by whoever publishes.
pub const SIGNING_KEY: &str = "GwpOuUsuF4bQ2KIA4TEqJDS716OT2Q4PiSY9GOYHogs=";
const KEY: &str = "update.settings";

/// An archive is never larger than this.
const MAX_ARCHIVE: usize = 200 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Look for new versions now and then and say so.
    pub check: bool,
    /// An `http://` or `socks5://` proxy for reaching the release server
    /// (GitHub is filtered in some places); empty for none.
    pub proxy: String,
    /// Where releases are listed; only changed to point at a test double.
    pub api_base: String,
    pub repo: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            check: true,
            proxy: String::new(),
            api_base: "https://api.github.com".into(),
            repo: REPO.into(),
        }
    }
}

impl Settings {
    pub fn proxy(&self) -> Result<Option<Proxy>, String> {
        Proxy::parse(&self.proxy)
    }
}

pub async fn load(db: &z_db::Database) -> Settings {
    match db.setting(KEY).await {
        Ok(Some(text)) => serde_json::from_str(&text).unwrap_or_default(),
        _ => Settings::default(),
    }
}

pub async fn save(db: &z_db::Database, settings: &Settings) -> Result<(), z_db::DbError> {
    db.set_setting(KEY, &serde_json::to_string(settings).unwrap_or_default())
        .await
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// The newest release and what to do about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct Info {
    /// The version running.
    pub current: String,
    /// The newest published, without the `v`.
    pub version: String,
    pub tag: String,
    pub notes: String,
    pub url: String,
    pub published_at: String,
    /// Whether it is newer than the one running.
    pub available: bool,
    #[serde(skip)]
    pub assets: Vec<Asset>,
}

/// The archive name for this kind of machine.
pub fn archive_name() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("z-ui-linux-amd64.tar.gz"),
        "aarch64" => Some("z-ui-linux-arm64.tar.gz"),
        _ => None,
    }
}

/// Whether the panel runs in a container, where it is replaced by pulling a
/// new image, not by overwriting a program inside the old one.
pub fn in_container() -> bool {
    std::env::var_os("ZUI_CONTAINER").is_some()
        || Path::new("/.dockerenv").exists()
        || Path::new("/run/.containerenv").exists()
}

/// The path of this program: a binary replaced by an update is shown by the
/// system as `path (deleted)`, and what is meant is the new one at `path`.
pub fn own_executable() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| "z-ui".into());
    match exe.to_string_lossy().strip_suffix(" (deleted)") {
        Some(path) => PathBuf::from(path),
        None => exe,
    }
}

fn get<'a>(url: &'a str, proxy: Option<&'a Proxy>, max_bytes: usize, timeout: u64) -> Request<'a> {
    Request {
        method: "GET",
        url,
        headers: &[],
        body: None,
        proxy,
        timeout: Duration::from_secs(timeout),
        max_bytes,
        redirects: 5,
    }
}

/// Ask the release server for the newest release.
pub async fn check(settings: &Settings) -> Result<Info, String> {
    let proxy = settings.proxy()?;
    let url = format!(
        "{}/repos/{}/releases/latest",
        settings.api_base.trim_end_matches('/'),
        settings.repo
    );
    let reply = outbound::send(Request {
        headers: &[
            ("Accept", "application/vnd.github+json"),
            ("X-GitHub-Api-Version", "2022-11-28"),
        ],
        ..get(&url, proxy.as_ref(), outbound::DEFAULT_MAX, 30)
    })
    .await
    .map_err(|e| format!("the release server could not be reached: {e}"))?;
    if reply.status == 404 {
        return Err("there is no release published yet".into());
    }
    if !reply.is_success() {
        return Err(format!("the release server answered HTTP {}", reply.status));
    }
    let json: serde_json::Value = serde_json::from_slice(&reply.body)
        .map_err(|e| format!("the release list is unreadable: {e}"))?;
    let tag = json["tag_name"]
        .as_str()
        .ok_or("the release has no tag")?
        .to_string();
    let version = tag.trim_start_matches('v').to_string();
    let newest = semver::Version::parse(&version)
        .map_err(|_| format!("the release tag {tag:?} is not a version"))?;
    let current = semver::Version::parse(z_common::VERSION).map_err(|e| e.to_string())?;
    let assets = json["assets"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(Asset {
                        name: a["name"].as_str()?.to_string(),
                        url: a["browser_download_url"].as_str()?.to_string(),
                        size: a["size"].as_u64().unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Info {
        current: z_common::VERSION.into(),
        available: newest > current && !json["prerelease"].as_bool().unwrap_or(false),
        version,
        tag,
        notes: json["body"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(4000)
            .collect(),
        url: json["html_url"].as_str().unwrap_or_default().to_string(),
        published_at: json["published_at"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        assets,
    })
}

/// A signature, as the file holds it: 64 raw bytes, or base64 of them.
fn signature_bytes(file: &[u8]) -> Result<Vec<u8>, String> {
    use base64::Engine;
    if file.len() == 64 {
        return Ok(file.to_vec());
    }
    let text = String::from_utf8_lossy(file);
    base64::engine::general_purpose::STANDARD
        .decode(text.split_whitespace().collect::<String>())
        .map_err(|_| "the signature file is neither raw nor base64".to_string())
}

/// Whether `signature` is the maker's signature of `message`, given the
/// public key in base64.
pub fn verify(key_base64: &str, message: &[u8], signature: &[u8]) -> Result<(), String> {
    use base64::Engine;
    let key = base64::engine::general_purpose::STANDARD
        .decode(key_base64)
        .map_err(|_| "the built-in release key is not base64".to_string())?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(message, &signature_bytes(signature)?)
        .map_err(|_| "the signature of the release's checksums is not the maintainers'".to_string())
}

/// The hash `sums` (a `sha256sum` listing) gives for `name`.
fn hash_of(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name && hash.len() == 64)
            .then(|| hash.to_ascii_lowercase())
    })
}

/// The program inside a release archive (`.tar.gz`).
fn program_in(archive: &[u8]) -> Result<Vec<u8>, String> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let entries = tar
        .entries()
        .map_err(|e| format!("the archive is unreadable: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("the archive is unreadable: {e}"))?;
        let is_program = entry.header().entry_type().is_file()
            && entry
                .path()
                .ok()
                .and_then(|p| p.file_name().map(|n| n == "z-ui"))
                .unwrap_or(false);
        if is_program {
            let mut program = Vec::new();
            entry
                .by_ref()
                .take(MAX_ARCHIVE as u64)
                .read_to_end(&mut program)
                .map_err(|e| format!("the archive is unreadable: {e}"))?;
            return Ok(program);
        }
    }
    Err("the archive has no z-ui program in it".into())
}

/// Download the release's program for this machine and check it: the
/// signature on the checksums with `key_base64`, then the archive against
/// them. Returns the program.
pub async fn download_verified(
    settings: &Settings,
    info: &Info,
    key_base64: &str,
) -> Result<Vec<u8>, String> {
    let proxy = settings.proxy()?;
    let wanted = archive_name().ok_or("there is no build of the panel for this kind of machine")?;
    let asset = |name: &str| {
        info.assets
            .iter()
            .find(|a| a.name == name)
            .ok_or_else(|| format!("the release has no {name}"))
    };
    let fetch = |url: String, max: usize, seconds: u64| {
        let proxy = proxy.clone();
        async move {
            let reply = outbound::send(get(&url, proxy.as_ref(), max, seconds))
                .await
                .map_err(|e| format!("downloading {url}: {e}"))?;
            if reply.is_success() {
                Ok(reply.body)
            } else {
                Err(format!("downloading {url}: HTTP {}", reply.status))
            }
        }
    };
    let sums = fetch(asset("SHA256SUMS")?.url.clone(), 1024 * 1024, 30).await?;
    let signature = fetch(asset("SHA256SUMS.sig")?.url.clone(), 4096, 30).await?;
    verify(key_base64, &sums, &signature)?;
    let sums = String::from_utf8_lossy(&sums);
    let expected = hash_of(&sums, wanted)
        .ok_or_else(|| format!("the signed checksums do not list {wanted}"))?;
    let archive = fetch(asset(wanted)?.url.clone(), MAX_ARCHIVE, 600).await?;
    let got = hex::encode(Sha256::digest(&archive));
    if got != expected {
        return Err(format!(
            "{wanted} is not the file that was signed (its hash is {got})"
        ));
    }
    program_in(&archive)
}

/// Put `program` in place of `exe`, keeping what was there as `exe.prev`.
/// The new program is first run to see that it starts and says it is
/// `version`; the swap is a rename, so a program that is running keeps
/// running and nothing is ever half-written.
pub async fn install(program: &[u8], exe: &Path, version: &str) -> Result<(), String> {
    let dir = exe.parent().ok_or("the program has no directory")?;
    let staged = dir.join(".z-ui.new");
    let cleanup = |error: String| {
        let _ = std::fs::remove_file(&staged);
        error
    };
    std::fs::write(&staged, program)
        .map_err(|e| cleanup(format!("writing {}: {e}", staged.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| cleanup(e.to_string()))?;
    }
    let candidate = staged.clone();
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::task::spawn_blocking(move || {
            std::process::Command::new(candidate)
                .arg("--version")
                .output()
        }),
    )
    .await
    .map_err(|_| cleanup("the new program did not answer --version".into()))?
    .map_err(|e| cleanup(e.to_string()))?
    .map_err(|e| cleanup(format!("the new program does not start: {e}")))?;
    let said = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.status.success() || !said.contains(version) {
        return Err(cleanup(format!(
            "the new program does not say it is version {version} (it says {:?})",
            said.trim()
        )));
    }
    let prev = prev_path(exe);
    if exe.exists() {
        std::fs::copy(exe, &prev).map_err(|e| {
            cleanup(format!(
                "keeping the old program as {}: {e}",
                prev.display()
            ))
        })?;
    }
    std::fs::rename(&staged, exe).map_err(|e| cleanup(format!("replacing {}: {e}", exe.display())))
}

fn prev_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".prev");
    exe.with_file_name(name)
}

/// Put the program kept before the last update back.
pub fn rollback(exe: &Path) -> Result<(), String> {
    let prev = prev_path(exe);
    if !prev.exists() {
        return Err(format!(
            "there is no earlier program ({} is missing)",
            prev.display()
        ));
    }
    std::fs::rename(&prev, exe).map_err(|e| format!("putting {} back: {e}", prev.display()))
}

/// What the last look for a newer version found.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub info: Option<Info>,
    /// When it looked (Unix seconds); 0 for never.
    pub checked_at: i64,
    pub error: Option<String>,
}

fn failed(error: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "update_failed", error.to_string())
}

/// Look for a newer version now, keep what was found, and say so (once for
/// each version) if there is one and the settings want it told.
pub async fn refresh(state: &AppState) -> Status {
    let settings = load(state.db()).await;
    let status = match check(&settings).await {
        Ok(info) => Status {
            info: Some(info),
            checked_at: z_db::now(),
            error: None,
        },
        Err(error) => Status {
            info: state.update_status().info,
            checked_at: z_db::now(),
            error: Some(error),
        },
    };
    if let Some(info) = status.info.as_ref().filter(|info| info.available) {
        if state
            .db()
            .mark_notified(&format!("update:{}", info.version))
            .await
            .unwrap_or(false)
        {
            state.notify(Event::UpdateAvailable {
                version: info.version.clone(),
                url: info.url.clone(),
            });
        }
    }
    state.set_update_status(status.clone());
    status
}

/// Whether the program can be replaced where it is: the directory it is in
/// takes a new file.
pub fn can_replace(exe: &Path) -> bool {
    let Some(dir) = exe.parent() else {
        return false;
    };
    let probe = dir.join(".z-ui.probe");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Download the newest version, check it against `key_base64`, put it in
/// place of `exe` and restart into it. Nothing is changed unless every check
/// passes.
pub async fn apply(state: &AppState, key_base64: &str, exe: &Path) -> Result<Info, ApiError> {
    apply_in(state, key_base64, exe, in_container()).await
}

/// [`apply`], told whether this is a container.
pub async fn apply_in(
    state: &AppState,
    key_base64: &str,
    exe: &Path,
    container: bool,
) -> Result<Info, ApiError> {
    if container {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "container",
            "this panel runs in a container: pull the new image instead",
        ));
    }
    let settings = load(state.db()).await;
    let info = check(&settings).await.map_err(failed)?;
    if !info.available {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "up_to_date",
            format!("{} is the newest version", info.current),
        ));
    }
    let program = download_verified(&settings, &info, key_base64)
        .await
        .map_err(failed)?;
    install(&program, exe, &info.version)
        .await
        .map_err(failed)?;
    tracing::info!(version = %info.version, "the panel was updated; restarting into it");
    state.restart_soon(Duration::from_millis(1500));
    Ok(info)
}

/// Look for a newer version now and then.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5 * 60)).await;
        loop {
            if load(state.db()).await.check {
                refresh(&state).await;
            }
            tokio::time::sleep(Duration::from_secs(12 * 3600)).await;
        }
    });
}
