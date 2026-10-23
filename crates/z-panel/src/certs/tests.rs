//! Issuing and renewing, against an authority and a Cloudflare that are only
//! in this process ([`super::mock`]): everything the client does for real is
//! done, and what the authority is told is checked.

use std::sync::Arc;

use axum::http::StatusCode;
use rustls_pki_types::pem::PemObject;
use serde_json::{json, Value};

use super::acme::Method;
use super::settings::{self, Ca, Settings};
use super::{inspect, mock, renew_at, renew_due, IssueRequest};
use crate::server::testing::{free_port, panel, panel_with, Panel};
use crate::tls::testing::self_signed;
use crate::tls::Resolver;

struct World {
    panel: Panel,
    owner: String,
    ca: Arc<mock::Ca>,
    cf: Arc<mock::Cloudflare>,
    http_port: u16,
    tls_port: u16,
}

async fn world() -> World {
    let panel = panel("/").await;
    world_on(panel, false, None).await
}

/// The test authority and Cloudflare, with the panel set to use them.
/// `tls_port` is where TLS challenges are answered, a free one by default.
async fn world_on(panel: Panel, require_eab: bool, tls_port: Option<u16>) -> World {
    let owner = panel.set_up("").await;
    let cf = mock::Cloudflare::start("cf-token", &["example.com"]).await;
    let ca = mock::Ca::start(require_eab, cf.records.clone()).await;
    let (http_port, tls_port) = (free_port(), tls_port.unwrap_or_else(free_port));
    ca.listen_ports(http_port, tls_port);
    let settings = Settings {
        ca: Ca::Custom,
        directory_url: format!("{}/directory", ca.base),
        email: "admin@example.com".into(),
        http_port,
        tls_port,
        dns_wait: 0,
        cloudflare_token: "cf-token".into(),
        cloudflare_api: cf.base.clone(),
        ..Settings::default()
    };
    settings::save(panel.state.db(), &settings).await.unwrap();
    World {
        panel,
        owner,
        ca,
        cf,
        http_port,
        tls_port,
    }
}

fn request(names: &[&str], method: Method) -> IssueRequest {
    IssueRequest {
        name: String::new(),
        names: names.iter().map(|n| n.to_string()).collect(),
        method,
        auto_renew: true,
    }
}

async fn change(w: &World, edit: impl FnOnce(&mut Settings)) {
    let mut settings = settings::load(w.panel.state.db()).await;
    edit(&mut settings);
    settings::save(w.panel.state.db(), &settings).await.unwrap();
}

/// Make certificate `id` expire in `days_left` days, as if its life had
/// passed (it was made for 90 days).
async fn age(w: &World, id: i64, days_left: i64) {
    let now = z_db::now();
    sqlx::query("UPDATE certificates SET not_before = $1, not_after = $2 WHERE id = $3")
        .bind(now + days_left * 86_400 - 90 * 86_400)
        .bind(now + days_left * 86_400)
        .bind(id)
        .execute(w.panel.state.db().pool())
        .await
        .unwrap();
}

fn port_is_closed(port: u16) -> bool {
    std::net::TcpStream::connect(("127.0.0.1", port)).is_err()
}

fn leaf_of(pem: &str) -> Vec<u8> {
    rustls_pki_types::CertificateDer::pem_slice_iter(pem.as_bytes())
        .next()
        .unwrap()
        .unwrap()
        .as_ref()
        .to_vec()
}

#[tokio::test]
async fn a_certificate_is_got_over_http_and_stored_with_how_it_was_made() {
    let w = world().await;
    let certificate = super::issue(
        &w.panel.state,
        request(&["panel.example.com", "vpn.example.com"], Method::Http01),
    )
    .await
    .unwrap();
    assert_eq!(certificate.source, "acme");
    assert_eq!(certificate.method, "http-01");
    assert!(certificate.auto_renew);
    assert_eq!(certificate.ca, format!("{}/directory", w.ca.base));
    assert_eq!(certificate.name, "panel.example.com");
    assert_eq!(certificate.domains, "panel.example.com,vpn.example.com");
    assert_eq!(
        certificate.cert_pem.matches("BEGIN CERTIFICATE").count(),
        2,
        "leaf and the authority's"
    );
    inspect(&certificate.cert_pem, &certificate.key_pem).expect("the key is the certificate's");
    assert!(certificate.not_after > z_db::now());

    assert_eq!(w.ca.ordered(), [["panel.example.com", "vpn.example.com"]]);
    assert_eq!(w.ca.contacts(), [["mailto:admin@example.com"]]);
    assert_eq!(w.ca.count("POST /new-acct"), 1);
    assert!(
        port_is_closed(w.http_port),
        "the challenge port is closed again"
    );
    let kept = settings::load(w.panel.state.db()).await;
    assert!(
        kept.accounts.contains_key(&kept.directory()),
        "the account is kept"
    );

    // The next one uses that account.
    super::issue(
        &w.panel.state,
        request(&["other.example.com"], Method::Http01),
    )
    .await
    .unwrap();
    assert_eq!(w.ca.count("POST /new-acct"), 1);
    assert_eq!(w.ca.count("POST /new-order"), 2);
}

#[tokio::test]
async fn auto_takes_port_80_and_falls_back_to_tls_when_it_is_taken() {
    let w = world().await;
    let free = super::issue(&w.panel.state, request(&["a.example.com"], Method::Auto))
        .await
        .unwrap();
    assert_eq!(free.method, "http-01");

    let _held = tokio::net::TcpListener::bind(("0.0.0.0", w.http_port))
        .await
        .unwrap();
    let taken = super::issue(&w.panel.state, request(&["b.example.com"], Method::Auto))
        .await
        .unwrap();
    assert_eq!(taken.method, "tls-alpn-01");
    assert!(
        port_is_closed(w.tls_port),
        "its temporary port is closed again"
    );

    // An explicit HTTP challenge does not fall back: it says why.
    let error = super::issue(&w.panel.state, request(&["c.example.com"], Method::Http01))
        .await
        .unwrap_err();
    assert_eq!(error.code, "ca_failed");
    assert!(
        error.message.contains(&w.http_port.to_string()),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn tls_challenges_are_answered_on_the_panels_own_port_when_that_is_the_one() {
    let (cert, key) = self_signed(&["panel.test"], 30);
    let resolver = Resolver::from_pem(&cert, &key).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let acceptor = crate::tls::acceptor(resolver.clone()).unwrap();
    tokio::spawn(crate::tls::serve(
        listener,
        acceptor,
        axum::Router::new(),
        std::future::pending(),
    ));
    let panel = panel_with(
        &["--listen", &format!("127.0.0.1:{port}")],
        Some(resolver.clone()),
    )
    .await;
    let w = world_on(panel, false, Some(port)).await;
    let before = resolver.leaf();

    let issued = super::issue(
        &w.panel.state,
        request(&["panel.example.com"], Method::TlsAlpn01),
    )
    .await
    .unwrap();
    assert_eq!(issued.method, "tls-alpn-01");
    assert_eq!(
        resolver.leaf(),
        before,
        "the panel keeps its own certificate"
    );
    assert!(!port_is_closed(port), "and keeps serving");
    // The challenge is gone with the order.
    let again = super::issue(
        &w.panel.state,
        request(&["panel.example.com"], Method::TlsAlpn01),
    )
    .await;
    assert!(again.is_ok(), "{again:?}");
}

#[tokio::test]
async fn a_wildcard_is_proven_with_dns_and_the_records_are_taken_away() {
    let w = world().await;
    let issued = super::issue(
        &w.panel.state,
        request(&["*.example.com", "example.com"], Method::Auto),
    )
    .await
    .unwrap();
    assert_eq!(issued.method, "dns-01");
    assert!(
        issued.domains.contains("*.example.com"),
        "{}",
        issued.domains
    );
    assert!(
        w.cf.records.lock().unwrap().is_empty(),
        "no record is left behind"
    );
    let calls = w.cf.calls.lock().unwrap().clone();
    assert_eq!(
        calls.iter().filter(|c| c.starts_with("POST")).count(),
        2,
        "{calls:?}"
    );
    assert_eq!(
        calls.iter().filter(|c| c.starts_with("DELETE")).count(),
        2,
        "{calls:?}"
    );
    assert_eq!(w.ca.count("POST /chall/"), 2);
}

#[tokio::test]
async fn dns_needs_a_token_that_works_and_a_zone_the_account_has() {
    let w = world().await;
    change(&w, |s| s.cloudflare_token.clear()).await;
    let error = super::issue(&w.panel.state, request(&["*.example.com"], Method::Auto))
        .await
        .unwrap_err();
    assert_eq!(error.code, "cloudflare_needed");
    assert_eq!(w.ca.count("POST"), 0, "the authority was not bothered");

    change(&w, |s| s.cloudflare_token = "wrong".into()).await;
    let error = super::issue(&w.panel.state, request(&["*.example.com"], Method::Auto))
        .await
        .unwrap_err();
    assert!(
        error.message.contains("refused the token"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains("Zone"),
        "it says which permission: {}",
        error.message
    );

    change(&w, |s| s.cloudflare_token = "cf-token".into()).await;
    let error = super::issue(&w.panel.state, request(&["*.elsewhere.org"], Method::Auto))
        .await
        .unwrap_err();
    assert!(error.message.contains("no zone"), "{}", error.message);
    assert!(w.cf.records.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_address_asks_for_the_short_lived_profile_and_a_name_does_not() {
    let w = world().await;
    let issued = super::issue(&w.panel.state, request(&["203.0.113.7"], Method::Http01))
        .await
        .unwrap();
    assert!(issued.domains.contains("203.0.113.7"), "{}", issued.domains);
    super::issue(
        &w.panel.state,
        request(&["panel.example.com"], Method::Http01),
    )
    .await
    .unwrap();
    assert_eq!(w.ca.profiles(), [Some("shortlived".to_string()), None]);

    // And an address over TLS: asked for by its reverse name.
    let by_tls = super::issue(&w.panel.state, request(&["2001:db8::7"], Method::TlsAlpn01))
        .await
        .unwrap();
    assert_eq!(by_tls.method, "tls-alpn-01");
}

#[tokio::test]
async fn an_authority_that_will_not_validate_is_explained_and_nothing_is_kept() {
    let w = world().await;
    w.ca.refuse(Some("Connection refused from the validation server"));
    let error = super::issue(
        &w.panel.state,
        request(&["panel.example.com"], Method::Http01),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status, StatusCode::BAD_GATEWAY);
    assert_eq!(error.code, "ca_failed");
    assert!(
        error.message.contains("Connection refused"),
        "{}",
        error.message
    );
    assert!(w.panel.state.db().certificates().await.unwrap().is_empty());
    assert!(port_is_closed(w.http_port));
}

#[tokio::test]
async fn an_authority_that_wants_a_key_is_given_the_one_that_was_set() {
    let w = world_on(panel("/").await, true, None).await;
    let error = super::issue(
        &w.panel.state,
        request(&["panel.example.com"], Method::Http01),
    )
    .await
    .unwrap_err();
    assert!(
        error.message.contains("key") || error.message.contains("externalAccountRequired"),
        "{}",
        error.message
    );

    use base64::Engine;
    let hmac = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"a shared secret");
    change(&w, |s| {
        s.eab_kid = "kid-9".into();
        s.eab_hmac = hmac.clone();
    })
    .await;
    super::issue(
        &w.panel.state,
        request(&["panel.example.com"], Method::Http01),
    )
    .await
    .unwrap();
    assert_eq!(w.ca.eab_kids(), ["kid-9"]);

    change(&w, |s| {
        s.accounts.clear();
        s.eab_hmac = "***".into();
    })
    .await;
    let error = super::issue(&w.panel.state, request(&["x.example.com"], Method::Http01))
        .await
        .unwrap_err();
    assert!(error.message.contains("base64url"), "{}", error.message);
}

#[tokio::test]
async fn zerossl_gives_an_account_key_for_an_email() {
    let ok = crate::notify::http::testing::mock(|seen| {
        assert_eq!(seen.path, "/acme/eab-credentials-email");
        assert_eq!(
            String::from_utf8_lossy(&seen.body),
            "email=admin%40example%2Ecom"
        );
        (
            200,
            r#"{"success":true,"eab_kid":"kid-z","eab_hmac_key":"aG1hYw"}"#.into(),
        )
    })
    .await;
    let settings = Settings {
        email: "admin@example.com".into(),
        zerossl_api: ok.url(),
        ..Settings::default()
    };
    let (kid, hmac) = super::acme::zerossl_key(&settings, None).await.unwrap();
    assert_eq!((kid.as_str(), hmac.as_str()), ("kid-z", "aG1hYw"));

    let refused = crate::notify::http::testing::mock(|_| {
        (
            200,
            r#"{"success":false,"error":{"type":"invalid_email"}}"#.into(),
        )
    })
    .await;
    let settings = Settings {
        zerossl_api: refused.url(),
        ..settings
    };
    let error = super::acme::zerossl_key(&settings, None).await.unwrap_err();
    assert!(error.contains("invalid_email"), "{error}");
    let none = Settings::default();
    assert!(super::acme::zerossl_key(&none, None)
        .await
        .unwrap_err()
        .contains("email"));
}

#[test]
fn renewal_starts_a_month_before_or_a_third_of_the_life_before_for_a_short_one() {
    let day = 86_400;
    assert_eq!(renew_at(0, 90 * day, 30), 60 * day);
    assert_eq!(renew_at(0, 90 * day, 10), 80 * day);
    assert_eq!(
        renew_at(0, 6 * day, 30),
        4 * day,
        "six days: two days before the end"
    );
    assert_eq!(renew_at(0, 6 * day, 1), 5 * day);
}

#[tokio::test]
async fn a_certificate_close_to_expiry_is_renewed_in_place_and_the_panel_serves_the_new_one() {
    let (cert, key) = self_signed(&["panel.example.com"], 30);
    let resolver = Resolver::from_pem(&cert, &key).unwrap();
    let panel = panel_with(&[], Some(resolver.clone())).await;
    let w = world_on(panel, false, None).await;
    let issued = super::issue(
        &w.panel.state,
        request(&["panel.example.com"], Method::Http01),
    )
    .await
    .unwrap();
    resolver.set(
        crate::tls::stored_certificate(w.panel.state.db(), issued.id)
            .await
            .unwrap(),
    );
    w.panel.state.set_panel_certificate(Some(issued.id));
    let before = resolver.leaf().unwrap();

    // Plenty of time left: nothing to do.
    age(&w, issued.id, 60).await;
    assert!(renew_due(&w.panel.state).await.is_empty());

    age(&w, issued.id, 20).await;
    let done = renew_due(&w.panel.state).await;
    assert_eq!(done.len(), 1);
    assert!(done[0].1.is_ok(), "{done:?}");
    let renewed = w
        .panel
        .state
        .db()
        .certificate(issued.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        renewed.id, issued.id,
        "the same entry, so everything that uses it follows"
    );
    assert!(
        renewed.not_after > z_db::now() + 60 * 86_400,
        "a fresh one: {}",
        renewed.not_after
    );
    assert!(renewed.last_error.is_empty());
    assert_ne!(renewed.cert_pem, issued.cert_pem);
    assert_eq!(
        resolver.leaf().unwrap(),
        leaf_of(&renewed.cert_pem),
        "the panel took it at once"
    );
    assert_ne!(resolver.leaf().unwrap(), before);
    assert_eq!(w.ca.count("POST /new-order"), 2);
}

#[tokio::test]
async fn a_failed_renewal_is_remembered_not_repeated_at_once_and_told_once_a_day() {
    let w = world().await;
    let issued = super::issue(
        &w.panel.state,
        request(&["panel.example.com"], Method::Http01),
    )
    .await
    .unwrap();
    age(&w, issued.id, 20).await;
    w.ca.refuse(Some("no way"));

    let first = renew_due(&w.panel.state).await;
    assert!(
        first[0].1.as_ref().unwrap_err().contains("no way"),
        "{first:?}"
    );
    let kept = w
        .panel
        .state
        .db()
        .certificate(issued.id)
        .await
        .unwrap()
        .unwrap();
    assert!(kept.last_error.contains("no way") && kept.last_attempt_at > 0);
    assert_eq!(kept.cert_pem, issued.cert_pem, "the old certificate stays");
    let day = z_db::now() / 86_400;
    assert!(w
        .panel
        .state
        .db()
        .was_notified(&format!("cert_failed:{}:{day}", issued.id))
        .await
        .unwrap());

    let orders = w.ca.count("POST /new-order");
    assert!(
        renew_due(&w.panel.state).await.is_empty(),
        "not again at once"
    );
    assert_eq!(w.ca.count("POST /new-order"), orders);

    // Hours later, when the authority has come round, it works and the error is gone.
    sqlx::query("UPDATE certificates SET last_attempt_at = $1 WHERE id = $2")
        .bind(z_db::now() - 7 * 3600)
        .bind(issued.id)
        .execute(w.panel.state.db().pool())
        .await
        .unwrap();
    w.ca.refuse(None);
    let later = renew_due(&w.panel.state).await;
    assert!(later[0].1.is_ok(), "{later:?}");
    assert!(w
        .panel
        .state
        .db()
        .certificate(issued.id)
        .await
        .unwrap()
        .unwrap()
        .last_error
        .is_empty());
}

#[tokio::test]
async fn a_certificate_that_will_not_renew_itself_is_warned_about_once_near_the_end() {
    let w = world().await;
    let (cert, key) = self_signed(&["panel.test"], 5);
    let parsed = inspect(&cert, &key).ok().unwrap();
    let uploaded = w
        .panel
        .state
        .db()
        .create_certificate(
            "uploaded",
            "panel.test",
            &cert,
            &key,
            "upload",
            parsed.not_before,
            parsed.not_after,
        )
        .await
        .unwrap();
    assert!(
        renew_due(&w.panel.state).await.is_empty(),
        "nothing to renew: it is not the panel's to renew"
    );
    let marker = format!("cert_expiring:{}:{}", uploaded.id, uploaded.not_after);
    assert!(w.panel.state.db().was_notified(&marker).await.unwrap());

    // One with a long way to go is left alone.
    let (far, far_key) = self_signed(&["far.test"], 200);
    let parsed = inspect(&far, &far_key).ok().unwrap();
    let kept = w
        .panel
        .state
        .db()
        .create_certificate(
            "far",
            "far.test",
            &far,
            &far_key,
            "upload",
            parsed.not_before,
            parsed.not_after,
        )
        .await
        .unwrap();
    renew_due(&w.panel.state).await;
    assert!(!w
        .panel
        .state
        .db()
        .was_notified(&format!("cert_expiring:{}:{}", kept.id, kept.not_after))
        .await
        .unwrap());
}

// ------------------------------------------------------------------------ API

async fn api(
    w: &World,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> crate::server::testing::Reply {
    w.panel.send(method, path, Some(&w.owner), body).await
}

#[tokio::test]
async fn certificates_are_got_renewed_changed_and_made_self_signed_through_the_api() {
    let w = world().await;
    let issued = api(
        &w,
        "POST",
        "/api/certificates/issue",
        Some(json!({"names": ["panel.example.com"], "method": "http-01"})),
    )
    .await;
    assert_eq!(issued.status, StatusCode::CREATED, "{}", issued.body);
    let view = issued.json();
    assert_eq!(view["source"], "acme");
    assert_eq!(view["method"], "http-01");
    assert_eq!(view["auto_renew"], true);
    assert!(view["renews_at"].as_i64().unwrap() < view["not_after"].as_i64().unwrap());
    assert!(view.get("key_pem").is_none() && !issued.body.contains("PRIVATE KEY"));
    let id = view["id"].as_i64().unwrap();

    let changed = api(
        &w,
        "PUT",
        &format!("/api/certificates/{id}"),
        Some(json!({"name": "main", "auto_renew": false})),
    )
    .await
    .json();
    assert_eq!(
        (changed["name"].as_str(), changed["auto_renew"].as_bool()),
        (Some("main"), Some(false))
    );
    assert_eq!(
        changed["renews_at"],
        Value::Null,
        "not renewing, so no date"
    );

    let renewed = api(&w, "POST", &format!("/api/certificates/{id}/renew"), None).await;
    assert_eq!(renewed.status, StatusCode::OK, "{}", renewed.body);

    let own = api(
        &w,
        "POST",
        "/api/certificates/self-signed",
        Some(json!({"names": ["203.0.113.7", "panel.test"], "name": "mine"})),
    )
    .await;
    assert_eq!(own.status, StatusCode::CREATED, "{}", own.body);
    let own = own.json();
    assert_eq!(own["source"], "self-signed");
    let domains: Vec<&str> = own["domains"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        domains.contains(&"203.0.113.7") && domains.contains(&"panel.test"),
        "{domains:?}"
    );
    let own_id = own["id"].as_i64().unwrap();
    for refused in [
        api(
            &w,
            "PUT",
            &format!("/api/certificates/{own_id}"),
            Some(json!({"auto_renew": true})),
        )
        .await,
        api(
            &w,
            "POST",
            &format!("/api/certificates/{own_id}/renew"),
            None,
        )
        .await,
    ] {
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
        assert_eq!(refused.json()["error"], "not_renewable");
    }
    assert_eq!(
        api(&w, "POST", "/api/certificates/9999/renew", None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn what_cannot_be_issued_is_refused_with_a_code_and_the_authoritys_words() {
    let w = world().await;
    let invalid = api(
        &w,
        "POST",
        "/api/certificates/issue",
        Some(json!({"names": ["not a domain"]})),
    )
    .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST, "{}", invalid.body);
    assert_eq!(invalid.json()["error"], "invalid_names");
    let none = api(
        &w,
        "POST",
        "/api/certificates/issue",
        Some(json!({"names": []})),
    )
    .await;
    assert_eq!(none.json()["error"], "invalid_names");

    w.ca.refuse(Some("Timeout during connect"));
    let failed = api(
        &w,
        "POST",
        "/api/certificates/issue",
        Some(json!({"names": ["panel.example.com"], "method": "http-01"})),
    )
    .await;
    assert_eq!(failed.status, StatusCode::BAD_GATEWAY, "{}", failed.body);
    assert_eq!(failed.json()["error"], "ca_failed");
    assert!(
        failed.body.contains("Timeout during connect"),
        "{}",
        failed.body
    );
}

#[tokio::test]
async fn the_settings_for_certificates_are_kept_checked_and_never_show_their_secrets() {
    let w = world().await;
    let got = api(&w, "GET", "/api/certificates/acme", None).await.json();
    assert_eq!(got["ca"], "custom");
    assert_eq!(got["cloudflare_token_set"], true);
    assert!(!got.to_string().contains("cf-token"));

    let saved = api(
        &w,
        "PUT",
        "/api/certificates/acme",
        Some(json!({"ca": "letsencrypt_staging", "email": "me@example.org", "http_port": 8080, "tls_port": 8443, "renew_days": 20, "dns_wait": 5, "proxy": "socks5://u:hunter2@127.0.0.1:1080", "cloudflare_token": "new-token"})),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    for secret in ["new-token", "hunter2", "cf-token"] {
        assert!(
            !saved.body.contains(secret),
            "{secret} leaked: {}",
            saved.body
        );
    }
    let stored = settings::load(w.panel.state.db()).await;
    assert_eq!(stored.cloudflare_token, "new-token");
    assert_eq!(stored.directory(), settings::LETSENCRYPT_STAGING);
    assert!(
        stored.accounts.is_empty(),
        "the accounts are not the page's to set"
    );

    let bad = api(
        &w,
        "PUT",
        "/api/certificates/acme",
        Some(json!({"email": "nobody"})),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.body);
    assert_eq!(bad.json()["error"], "invalid_acme");
}

#[tokio::test]
async fn the_certificate_the_panel_serves_with_cannot_be_deleted() {
    let w = world().await;
    let own = api(
        &w,
        "POST",
        "/api/certificates/self-signed",
        Some(json!({"names": ["panel.test"]})),
    )
    .await
    .json();
    let id = own["id"].as_i64().unwrap();
    let saved = api(
        &w,
        "PUT",
        "/api/panel",
        Some(json!({"listen": "0.0.0.0:2053", "base_path": "/", "certificate_id": id})),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    let refused = api(&w, "DELETE", &format!("/api/certificates/{id}"), None).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert_eq!(refused.json()["error"], "certificate_in_use");
    api(
        &w,
        "PUT",
        "/api/panel",
        Some(json!({"listen": "0.0.0.0:2053", "base_path": "/", "certificate_id": null})),
    )
    .await;
    assert_eq!(
        api(&w, "DELETE", &format!("/api/certificates/{id}"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
}
