//! Updating, against a release server that is only in this process: a
//! release signed with a key of the test's own, an archive with a program in
//! it that says its version, and every way the files can be wrong.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use base64::Engine;
use ring::signature::{Ed25519KeyPair, KeyPair};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

use super::*;

pub(crate) fn keypair() -> (Ed25519KeyPair, String) {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let public = base64::engine::general_purpose::STANDARD.encode(pair.public_key().as_ref());
    (pair, public)
}

pub(crate) fn script(version: &str) -> Vec<u8> {
    format!("#!/bin/sh\necho \"z-ui {version}\"\n").into_bytes()
}

/// A `.tar.gz` holding `program` as `z-ui-<dir>/z-ui`, with a licence beside it.
pub(crate) fn archive(program: &[u8]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, bytes, mode) in [
        ("z-ui-release/LICENSE", &b"GPL"[..], 0o644),
        ("z-ui-release/z-ui", program, 0o755),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        tar.append_data(&mut header, path, bytes).unwrap();
    }
    let raw = tar.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut gz, &raw).unwrap();
    gz.finish().unwrap()
}

/// What the release server holds, and how it is wrong.
pub(crate) struct Release {
    pub(crate) tag: String,
    pub(crate) prerelease: bool,
    archive: Vec<u8>,
    /// What the checksums say the archive is; the archive itself when `None`.
    listed_hash: Option<String>,
    signer: Ed25519KeyPair,
    with_signature: bool,
    raw_signature: bool,
}

impl Release {
    pub(crate) fn new(version: &str, signer: Ed25519KeyPair) -> Self {
        Self {
            tag: format!("v{version}"),
            prerelease: false,
            archive: archive(&script(version)),
            listed_hash: None,
            signer,
            with_signature: true,
            raw_signature: false,
        }
    }
}

pub(crate) struct Server {
    pub(crate) base: String,
}

pub(crate) async fn serve(release: Option<Release>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let release = Arc::new((release, base.clone()));
    let app = Router::new().fallback(handle).with_state(release);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Server { base }
}

async fn handle(State(shared): State<Arc<(Option<Release>, String)>>, uri: Uri) -> Response {
    let (release, base) = (&shared.0, &shared.1);
    let Some(release) = release else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let sums = {
        let hash = release
            .listed_hash
            .clone()
            .unwrap_or_else(|| hex::encode(Sha256::digest(&release.archive)));
        format!("{hash}  z-ui-linux-amd64.tar.gz\n{hash}  z-ui-linux-arm64.tar.gz\n")
    };
    match uri.path() {
        "/repos/ZeroNetTeam/Z-UI/releases/latest" => {
            let mut assets = vec![
                ("z-ui-linux-amd64.tar.gz", "/download/archive"),
                ("z-ui-linux-arm64.tar.gz", "/download/archive"),
                ("SHA256SUMS", "/download/sums"),
            ];
            if release.with_signature {
                assets.push(("SHA256SUMS.sig", "/download/sig"));
            }
            axum::Json(serde_json::json!({
                "tag_name": release.tag,
                "prerelease": release.prerelease,
                "body": "What is new",
                "html_url": format!("{base}/release"),
                "published_at": "2026-10-23T00:00:00Z",
                "assets": assets.iter().map(|(name, path)| serde_json::json!({
                    "name": name, "browser_download_url": format!("{base}{path}"), "size": 1,
                })).collect::<Vec<_>>(),
            }))
            .into_response()
        }
        // A download is passed on to where the file is kept, as on GitHub.
        "/download/archive" => {
            (StatusCode::FOUND, [(header::LOCATION, "/storage/archive")]).into_response()
        }
        "/storage/archive" => release.archive.clone().into_response(),
        "/download/sums" => sums.into_response(),
        "/download/sig" => {
            let signature = release.signer.sign(sums.as_bytes());
            if release.raw_signature {
                signature.as_ref().to_vec().into_response()
            } else {
                base64::engine::general_purpose::STANDARD
                    .encode(signature.as_ref())
                    .into_response()
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(crate) fn settings(server: &Server) -> Settings {
    Settings {
        api_base: server.base.clone(),
        ..Settings::default()
    }
}

pub(crate) fn scratch(name: &str) -> PathBuf {
    let unique = format!("z-update-{name}-{}-{}", std::process::id(), z_db::now());
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn a_newer_release_is_found_and_an_equal_older_or_trial_one_is_not() {
    let (pair, _) = keypair();
    for (tag, available) in [
        ("v9.9.9", true),
        ("v0.0.1", false),
        (&format!("v{}", z_common::VERSION)[..], false),
    ] {
        let mut release = Release::new("9.9.9", keypair().0);
        release.tag = tag.to_string();
        let server = serve(Some(release)).await;
        let info = check(&settings(&server)).await.unwrap();
        assert_eq!(info.available, available, "{tag}");
        assert_eq!(info.version, tag.trim_start_matches('v'));
        assert_eq!(info.current, z_common::VERSION);
        assert_eq!(info.notes, "What is new");
    }
    let mut trial = Release::new("9.9.9", pair);
    trial.prerelease = true;
    let server = serve(Some(trial)).await;
    assert!(
        !check(&settings(&server)).await.unwrap().available,
        "a trial release is not offered"
    );

    let none = serve(None).await;
    assert!(check(&settings(&none))
        .await
        .unwrap_err()
        .contains("no release"));
    let unreachable = Settings {
        api_base: "http://127.0.0.1:1".into(),
        ..Settings::default()
    };
    assert!(check(&unreachable)
        .await
        .unwrap_err()
        .contains("could not be reached"));
}

#[tokio::test]
async fn a_signed_release_replaces_the_program_keeps_the_old_one_and_can_be_rolled_back() {
    let (pair, public) = keypair();
    let server = serve(Some(Release::new("9.9.9", pair))).await;
    let info = check(&settings(&server)).await.unwrap();
    let program = download_verified(&settings(&server), &info, &public)
        .await
        .unwrap();
    assert_eq!(program, script("9.9.9"));

    let dir = scratch("replace");
    let exe = dir.join("z-ui");
    std::fs::write(&exe, script("0.1.0")).unwrap();
    install(&program, &exe, "9.9.9").await.unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), script("9.9.9"));
    assert_eq!(
        std::fs::read(dir.join("z-ui.prev")).unwrap(),
        script("0.1.0"),
        "the old one is kept"
    );
    assert!(!dir.join(".z-ui.new").exists(), "nothing staged is left");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    rollback(&exe).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), script("0.1.0"));
    assert!(rollback(&exe).unwrap_err().contains("no earlier program"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn a_release_signed_by_anyone_else_is_refused() {
    let (stranger, _) = keypair();
    let (_, ours) = keypair();
    let server = serve(Some(Release::new("9.9.9", stranger))).await;
    let info = check(&settings(&server)).await.unwrap();
    let error = download_verified(&settings(&server), &info, &ours)
        .await
        .unwrap_err();
    assert!(error.contains("not the maintainers'"), "{error}");
}

#[tokio::test]
async fn an_archive_other_than_the_one_signed_for_is_refused() {
    let (pair, public) = keypair();
    let mut release = Release::new("9.9.9", pair);
    // The checksums are signed and honest about a different archive.
    release.listed_hash = Some(hex::encode(Sha256::digest(b"the real archive")));
    let server = serve(Some(release)).await;
    let info = check(&settings(&server)).await.unwrap();
    let error = download_verified(&settings(&server), &info, &public)
        .await
        .unwrap_err();
    assert!(error.contains("not the file that was signed"), "{error}");
}

#[tokio::test]
async fn a_release_without_its_signature_or_a_signature_of_the_wrong_shape_is_refused() {
    let (pair, public) = keypair();
    let mut unsigned = Release::new("9.9.9", pair);
    unsigned.with_signature = false;
    let server = serve(Some(unsigned)).await;
    let info = check(&settings(&server)).await.unwrap();
    let error = download_verified(&settings(&server), &info, &public)
        .await
        .unwrap_err();
    assert!(error.contains("SHA256SUMS.sig"), "{error}");

    // 64 raw bytes are as good as their base64.
    let (pair, public) = keypair();
    let mut raw = Release::new("9.9.9", pair);
    raw.raw_signature = true;
    let server = serve(Some(raw)).await;
    let info = check(&settings(&server)).await.unwrap();
    assert!(download_verified(&settings(&server), &info, &public)
        .await
        .is_ok());

    assert!(verify(&public, b"message", b"not a signature at all !!").is_err());
    assert!(verify("not base64!", b"message", &[0; 64]).is_err());
}

#[tokio::test]
async fn a_program_that_will_not_say_it_is_the_version_or_will_not_start_is_never_put_in_place() {
    let dir = scratch("refuse");
    let exe = dir.join("z-ui");
    std::fs::write(&exe, script("0.1.0")).unwrap();
    for (program, why) in [
        (script("1.0.0"), "the wrong version"),
        (b"not a program".to_vec(), "garbage"),
        (b"#!/bin/sh\nexit 3\n".to_vec(), "a failure"),
    ] {
        let error = install(&program, &exe, "9.9.9").await.unwrap_err();
        assert!(!error.is_empty(), "{why}");
        assert_eq!(
            std::fs::read(&exe).unwrap(),
            script("0.1.0"),
            "{why}: the program is untouched"
        );
        assert!(
            !dir.join(".z-ui.new").exists(),
            "{why}: nothing staged is left"
        );
        assert!(!dir.join("z-ui.prev").exists(), "{why}: nothing was kept");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn checksums_are_read_the_way_sha256sum_writes_them() {
    let hash = "a".repeat(64);
    let sums = format!(
        "{hash}  z-ui-linux-amd64.tar.gz\n{}  *z-ui-linux-arm64.tar.gz\nshort  x\n",
        "B".repeat(64)
    );
    assert_eq!(hash_of(&sums, "z-ui-linux-amd64.tar.gz"), Some(hash));
    assert_eq!(
        hash_of(&sums, "z-ui-linux-arm64.tar.gz"),
        Some("b".repeat(64)),
        "binary mode and capitals"
    );
    assert_eq!(hash_of(&sums, "x"), None, "a hash is 64 hex digits");
    assert_eq!(hash_of(&sums, "missing"), None);
}

#[test]
fn the_program_is_the_file_named_z_ui_in_the_archive_and_nothing_else_will_do() {
    assert_eq!(program_in(&archive(b"binary")).unwrap(), b"binary");
    let mut tar = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(1);
    header.set_cksum();
    tar.append_data(&mut header, "z-ui-release/readme", &b"x"[..])
        .unwrap();
    let mut dir = tar::Header::new_gnu();
    dir.set_entry_type(tar::EntryType::Directory);
    dir.set_size(0);
    dir.set_cksum();
    tar.append_data(&mut dir, "z-ui-release/z-ui", &b""[..])
        .unwrap();
    let raw = tar.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut gz, &raw).unwrap();
    let error = program_in(&gz.finish().unwrap()).unwrap_err();
    assert!(
        error.contains("no z-ui program"),
        "a directory of that name is not it: {error}"
    );
    assert!(program_in(b"not an archive").is_err());
}

#[test]
fn the_built_in_key_is_a_valid_ed25519_public_key() {
    let key = base64::engine::general_purpose::STANDARD
        .decode(SIGNING_KEY)
        .unwrap();
    assert_eq!(key.len(), 32);
    // It verifies nothing by accident.
    assert!(verify(SIGNING_KEY, b"anything", &[0u8; 64]).is_err());
}

#[test]
fn what_the_release_workflow_signs_with_openssl_is_what_the_built_in_key_verifies() {
    // Made once with the maintainers' key, the way the workflow signs:
    //   openssl pkeyutl -sign -inkey signing-key.pem -rawin -in message -out sig
    const MESSAGE: &[u8] = b"z-ui release signing check\n";
    const SIGNATURE: &str =
        "eTn8V7W8IHbxROxFs/BQEN4w+klRE4zb9kVwR5tcEEatEN/qoZATQiVcSfl4WbQ5u+J2E7wHOY2MYoWUuUxCDQ==";
    assert!(
        verify(SIGNING_KEY, MESSAGE, SIGNATURE.as_bytes()).is_ok(),
        "as the base64 file"
    );
    let raw = base64::engine::general_purpose::STANDARD
        .decode(SIGNATURE)
        .unwrap();
    assert!(
        verify(SIGNING_KEY, MESSAGE, &raw).is_ok(),
        "as the raw file openssl writes"
    );
    assert!(
        verify(SIGNING_KEY, b"another message\n", &raw).is_err(),
        "and only for that message"
    );
}

#[test]
fn a_binary_replaced_while_running_is_still_found_at_its_path() {
    // The system shows a replaced program as "path (deleted)".
    let exe = own_executable();
    assert!(!exe.to_string_lossy().ends_with("(deleted)"));
}
