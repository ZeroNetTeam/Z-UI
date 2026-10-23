//! Getting a certificate from a certificate authority over ACME: an account,
//! an order for some names, a challenge for each, and the certificate.

use std::net::IpAddr;
use std::time::Duration;

use axum::http::StatusCode;
use base64::Engine;
use instant_acme::{
    Account, AuthorizationStatus, ChallengeType, ExternalAccountKey, Identifier, NewAccount,
    NewOrder, OrderStatus, RetryPolicy,
};
use serde::{Deserialize, Serialize};

use super::client::Client;
use super::cloudflare::{Cloudflare, Record};
use super::settings::{self, Ca, Settings};
use super::solve::{AlpnSolver, HttpSolver};
use crate::error::ApiError;
use crate::notify::http::Proxy;
use crate::outbound::{self, Request};
use crate::server::AppState;

/// How a name is proven to be the panel's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum Method {
    /// Port 80 if it can be had, else the TLS port; a wildcard needs DNS.
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "http-01")]
    Http01,
    #[serde(rename = "tls-alpn-01")]
    TlsAlpn01,
    #[serde(rename = "dns-01")]
    Dns01,
}

impl Method {
    /// What a certificate records of how it was issued.
    pub fn label(self) -> &'static str {
        match self {
            Method::Auto => "auto",
            Method::Http01 => "http-01",
            Method::TlsAlpn01 => "tls-alpn-01",
            Method::Dns01 => "dns-01",
        }
    }

    pub fn from_label(label: &str) -> Method {
        match label {
            "http-01" => Method::Http01,
            "tls-alpn-01" => Method::TlsAlpn01,
            "dns-01" => Method::Dns01,
            _ => Method::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Name {
    Dns(String),
    /// `*.example.com`, kept with its star.
    Wildcard(String),
    Ip(IpAddr),
}

impl Name {
    pub fn text(&self) -> String {
        match self {
            Name::Dns(name) | Name::Wildcard(name) => name.clone(),
            Name::Ip(ip) => ip.to_string(),
        }
    }

    fn identifier(&self) -> Identifier {
        match self {
            Name::Dns(name) | Name::Wildcard(name) => Identifier::Dns(name.clone()),
            Name::Ip(ip) => Identifier::Ip(*ip),
        }
    }
}

fn bad(code: &'static str, message: impl Into<String>) -> ApiError {
    ApiError::bad_request(code, message)
}

fn ca(message: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "ca_failed", message.to_string())
}

/// The most names an order may hold (a certificate authority's own limit).
const MAX_NAMES: usize = 100;

/// A domain name, checked: letters, digits and hyphens in labels, at least
/// two labels, ASCII only (an internationalised name is given in its
/// `xn--` form).
fn domain(text: &str) -> Result<String, String> {
    if !text.is_ascii() {
        return Err(format!(
            "{text}: give an internationalised name in its xn-- (punycode) form"
        ));
    }
    let name = text.trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = name.split('.').collect();
    if name.len() > 253 || labels.len() < 2 {
        return Err(format!("{text} is not a domain name with a dot in it"));
    }
    for label in &labels {
        let ok = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !ok {
            return Err(format!("{text} is not a valid domain name"));
        }
    }
    if labels
        .last()
        .is_some_and(|tld| tld.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!(
            "{text} looks like a broken address, not a domain name"
        ));
    }
    Ok(name)
}

/// What an admin typed: domains, `*.domains` and addresses.
pub fn parse_names(list: &[String]) -> Result<Vec<Name>, ApiError> {
    let mut names: Vec<Name> = Vec::new();
    for raw in list
        .iter()
        .flat_map(|line| line.split([',', ' ', '\n', '\t']))
    {
        let raw = raw.trim().trim_start_matches('[').trim_end_matches(']');
        if raw.is_empty() {
            continue;
        }
        let name = if let Ok(ip) = raw.parse::<IpAddr>() {
            Name::Ip(ip)
        } else if let Some(rest) = raw.strip_prefix("*.") {
            Name::Wildcard(format!(
                "*.{}",
                domain(rest).map_err(|m| bad("invalid_names", m))?
            ))
        } else {
            Name::Dns(domain(raw).map_err(|m| bad("invalid_names", m))?)
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() {
        return Err(bad(
            "invalid_names",
            "give at least one domain name or address",
        ));
    }
    if names.len() > MAX_NAMES {
        return Err(bad(
            "invalid_names",
            format!("{MAX_NAMES} names at most in one certificate"),
        ));
    }
    Ok(names)
}

/// Whether what is asked can work at all, before the authority is bothered.
fn check_plan(names: &[Name], method: Method, settings: &Settings) -> Result<(), ApiError> {
    let wildcard = names.iter().any(|n| matches!(n, Name::Wildcard(_)));
    let address = names.iter().any(|n| matches!(n, Name::Ip(_)));
    if wildcard && !matches!(method, Method::Auto | Method::Dns01) {
        return Err(bad(
            "invalid_method",
            "a wildcard name can only be proven with a DNS challenge",
        ));
    }
    if address && method == Method::Dns01 {
        return Err(bad(
            "invalid_method",
            "an address cannot be proven with a DNS challenge; use HTTP or TLS",
        ));
    }
    if (wildcard || method == Method::Dns01) && settings.cloudflare_token.is_empty() {
        return Err(bad(
            "cloudflare_needed",
            "a DNS challenge writes a record at Cloudflare: give a Cloudflare API token in the certificate settings",
        ));
    }
    if wildcard && address {
        return Err(bad(
            "invalid_method",
            "a wildcard and an address cannot share a certificate (one needs DNS, the other cannot use it)",
        ));
    }
    if settings.ca == Ca::Custom && settings.directory_url.is_empty() {
        return Err(bad(
            "invalid_ca",
            "the other authority's directory URL is not set",
        ));
    }
    Ok(())
}

/// A certificate and key the authority issued.
pub struct Obtained {
    pub chain_pem: String,
    pub key_pem: String,
    /// The challenge that was used.
    pub method: Method,
}

/// The ZeroSSL account key an email buys, over its API.
pub(super) async fn zerossl_key(
    settings: &Settings,
    proxy: Option<&Proxy>,
) -> Result<(String, String), String> {
    if settings.email.is_empty() {
        return Err("ZeroSSL wants an email address: set one in the certificate settings".into());
    }
    let body = format!(
        "email={}",
        percent_encoding::utf8_percent_encode(&settings.email, percent_encoding::NON_ALPHANUMERIC)
    );
    let url = format!(
        "{}/acme/eab-credentials-email",
        settings.zerossl_api.trim_end_matches('/')
    );
    let reply = outbound::send(Request {
        method: "POST",
        url: &url,
        headers: &[],
        body: Some(("application/x-www-form-urlencoded", body.as_bytes())),
        proxy,
        timeout: Duration::from_secs(30),
        max_bytes: outbound::DEFAULT_MAX,
    })
    .await
    .map_err(|e| format!("ZeroSSL could not be reached: {e}"))?;
    let json: serde_json::Value = serde_json::from_slice(&reply.body).unwrap_or_default();
    match (json["eab_kid"].as_str(), json["eab_hmac_key"].as_str()) {
        (Some(kid), Some(hmac)) if reply.is_success() => Ok((kid.into(), hmac.into())),
        _ => Err(format!(
            "ZeroSSL gave no account key: {}",
            json["error"]["type"]
                .as_str()
                .map_or_else(|| format!("HTTP {}", reply.status), str::to_string)
        )),
    }
}

/// The account on the chosen directory: the one kept from before, else a new
/// one whose credentials are kept.
async fn account(
    state: &AppState,
    settings: &Settings,
    proxy: Option<Proxy>,
) -> Result<Account, String> {
    let directory = settings.directory();
    if let Some(stored) = settings.accounts.get(&directory) {
        if let Ok(credentials) = serde_json::from_value(stored.clone()) {
            match Account::builder_with_http(Box::new(Client {
                proxy: proxy.clone(),
            }))
            .from_credentials(credentials)
            .await
            {
                Ok(account) => return Ok(account),
                Err(error) => {
                    tracing::warn!(%error, "the stored account was not accepted; a new one is made")
                }
            }
        }
    }
    let mut settings = settings.clone();
    if settings.wants_eab() && (settings.eab_kid.is_empty() || settings.eab_hmac.is_empty()) {
        let (kid, hmac) = zerossl_key(&settings, proxy.as_ref()).await?;
        settings.eab_kid = kid;
        settings.eab_hmac = hmac;
    }
    let external = if !settings.eab_kid.is_empty() && !settings.eab_hmac.is_empty() {
        let key = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(settings.eab_hmac.trim_end_matches('='))
            .map_err(|_| "the account key (HMAC) is not base64url".to_string())?;
        Some(ExternalAccountKey::new(settings.eab_kid.clone(), &key))
    } else {
        None
    };
    let contact = format!("mailto:{}", settings.email);
    let contacts: Vec<&str> = if settings.email.is_empty() {
        Vec::new()
    } else {
        vec![contact.as_str()]
    };
    let (account, credentials) = Account::builder_with_http(Box::new(Client { proxy }))
        .create(
            &NewAccount {
                contact: &contacts,
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            directory.clone(),
            external.as_ref(),
        )
        .await
        .map_err(|e| format!("the authority did not make an account: {e}"))?;
    // Keep it, with any key that was fetched: read the settings again, as
    // they may have been changed while the authority was answering.
    let mut latest = settings::load(state.db()).await;
    latest.accounts.insert(
        directory,
        serde_json::to_value(&credentials).map_err(|e| e.to_string())?,
    );
    if latest.eab_kid.is_empty() && !settings.eab_kid.is_empty() {
        latest.eab_kid = settings.eab_kid.clone();
        latest.eab_hmac = settings.eab_hmac.clone();
    }
    settings::save(state.db(), &latest)
        .await
        .map_err(|e| format!("the account could not be kept: {e}"))?;
    Ok(account)
}

/// What answers the challenges of one order, started as they are needed and
/// stopped (and the DNS records removed) when it is over.
struct Solvers<'a> {
    state: &'a AppState,
    settings: &'a Settings,
    proxy: Option<Proxy>,
    http: Option<HttpSolver>,
    alpn: Option<AlpnSolver>,
    records: Vec<Record>,
}

impl<'a> Solvers<'a> {
    fn cloudflare(&self) -> Cloudflare {
        Cloudflare {
            token: self.settings.cloudflare_token.clone(),
            base: self.settings.cloudflare_api.clone(),
            proxy: self.proxy.clone(),
        }
    }

    async fn http(&mut self) -> Result<&HttpSolver, String> {
        if self.http.is_none() {
            self.http = Some(HttpSolver::start(self.settings.http_port).await?);
        }
        Ok(self.http.as_ref().expect("just started"))
    }

    async fn alpn(&mut self) -> Result<&AlpnSolver, String> {
        if self.alpn.is_none() {
            let panel = self
                .state
                .tls_resolver()
                .map(|r| (r.clone(), self.state.config().listen.port()));
            self.alpn = Some(AlpnSolver::start(panel, self.settings.tls_port).await?);
        }
        Ok(self.alpn.as_ref().expect("just started"))
    }

    /// Which challenge to answer for a name, starting what it needs.
    async fn choose(&mut self, method: Method, dns_for_all: bool) -> Result<ChallengeType, String> {
        match method {
            Method::Http01 => self.http().await.map(|_| ChallengeType::Http01),
            Method::TlsAlpn01 => self.alpn().await.map(|_| ChallengeType::TlsAlpn01),
            Method::Dns01 => Ok(ChallengeType::Dns01),
            Method::Auto if dns_for_all => Ok(ChallengeType::Dns01),
            Method::Auto => match self.http().await {
                Ok(_) => Ok(ChallengeType::Http01),
                Err(http_error) => match self.alpn().await {
                    Ok(_) => Ok(ChallengeType::TlsAlpn01),
                    Err(alpn_error) => Err(format!(
                        "neither challenge port could be used: {http_error}; {alpn_error}. Free one of them, or prove the name with DNS."
                    )),
                },
            },
        }
    }

    async fn cleanup(&mut self) {
        let records = std::mem::take(&mut self.records);
        if !records.is_empty() {
            let cloudflare = self.cloudflare();
            for record in &records {
                cloudflare.remove(record).await;
            }
        }
        self.http = None;
        self.alpn = None;
    }
}

fn kind_method(kind: &ChallengeType) -> Method {
    match kind {
        ChallengeType::Http01 => Method::Http01,
        ChallengeType::TlsAlpn01 => Method::TlsAlpn01,
        _ => Method::Dns01,
    }
}

/// Get a certificate for `names` from the authority in `settings`.
pub async fn obtain(
    state: &AppState,
    settings: &Settings,
    names: &[Name],
    method: Method,
) -> Result<Obtained, ApiError> {
    check_plan(names, method, settings)?;
    let proxy = settings
        .proxy()
        .map_err(|m| bad("invalid_proxy", format!("the proxy for certificates: {m}")))?;
    let account = account(state, settings, proxy.clone()).await.map_err(ca)?;

    let identifiers: Vec<Identifier> = names.iter().map(Name::identifier).collect();
    // An address gets a short-lived certificate, which is a profile of its
    // own; an authority that has no profiles is asked without.
    let wants_profile = names.iter().any(|n| matches!(n, Name::Ip(_)))
        && matches!(
            settings.ca,
            Ca::Letsencrypt | Ca::LetsencryptStaging | Ca::Custom
        );
    let refused = |e: instant_acme::Error| ca(format!("the authority refused the order: {e}"));
    let mut order = if wants_profile {
        match account
            .new_order(&NewOrder::new(&identifiers).profile("shortlived"))
            .await
        {
            Err(instant_acme::Error::Unsupported(_)) => account
                .new_order(&NewOrder::new(&identifiers))
                .await
                .map_err(refused)?,
            other => other.map_err(refused)?,
        }
    } else {
        account
            .new_order(&NewOrder::new(&identifiers))
            .await
            .map_err(refused)?
    };

    let mut solvers = Solvers {
        state,
        settings,
        proxy,
        http: None,
        alpn: None,
        records: Vec::new(),
    };
    // With a wildcard in the order, DNS proves every name: one way, no port needed.
    let dns_for_all = names.iter().any(|n| matches!(n, Name::Wildcard(_)));
    let result = run(&mut order, &mut solvers, method, dns_for_all).await;
    solvers.cleanup().await;
    let (used, key_pem) = result.map_err(ca)?;
    let chain_pem = order
        .poll_certificate(&RetryPolicy::new().timeout(Duration::from_secs(120)))
        .await
        .map_err(|e| ca(format!("the certificate did not come: {e}")))?;
    Ok(Obtained {
        chain_pem,
        key_pem,
        method: used,
    })
}

/// Answer the order's challenges, wait for it to be ready, and finalize it.
async fn run(
    order: &mut instant_acme::Order,
    solvers: &mut Solvers<'_>,
    method: Method,
    dns_for_all: bool,
) -> Result<(Method, String), String> {
    let mut kinds: Vec<Method> = Vec::new();
    let mut authorizations = order.authorizations();
    while let Some(result) = authorizations.next().await {
        let mut authz = result.map_err(|e| format!("reading the authorization: {e}"))?;
        match authz.status {
            AuthorizationStatus::Pending => {}
            AuthorizationStatus::Valid => continue,
            other => {
                return Err(format!(
                    "the authority says the authorization for {} is {other:?}",
                    authz.identifier()
                ))
            }
        }
        let shown = authz.identifier().to_string();
        let base = match authz.identifier().identifier {
            Identifier::Dns(name) => name.clone(),
            other => match other {
                Identifier::Ip(ip) => ip.to_string(),
                _ => shown.clone(),
            },
        };
        let kind = solvers.choose(method, dns_for_all).await?;
        kinds.push(kind_method(&kind));
        let mut challenge = authz.challenge(kind.clone()).ok_or_else(|| {
            format!(
                "the authority offers no {} challenge for {shown}",
                kind_method(&kind).label()
            )
        })?;
        let key = challenge.key_authorization();
        match kind {
            ChallengeType::Http01 => solvers.http().await?.add(&challenge.token, key.as_str()),
            ChallengeType::TlsAlpn01 => solvers.alpn().await?.add(&base, key.digest().as_ref())?,
            _ => {
                let record = solvers
                    .cloudflare()
                    .add_txt(&format!("_acme-challenge.{base}"), &key.dns_value())
                    .await?;
                solvers.records.push(record);
                // Let the record reach the authority's resolvers.
                tokio::time::sleep(Duration::from_secs(solvers.settings.dns_wait)).await;
            }
        }
        challenge
            .set_ready()
            .await
            .map_err(|e| format!("telling the authority {shown} is ready: {e}"))?;
    }

    let policy = RetryPolicy::new()
        .initial_delay(Duration::from_millis(250))
        .backoff(1.5)
        .timeout(Duration::from_secs(120));
    match order.poll_ready(&policy).await {
        Ok(OrderStatus::Ready) => {}
        Ok(other) => {
            return Err(refusal(order)
                .await
                .unwrap_or_else(|| format!("the order ended as {other:?}")))
        }
        Err(error) => {
            return Err(refusal(order)
                .await
                .unwrap_or_else(|| format!("waiting for the authority: {error}")))
        }
    }
    let key_pem = order
        .finalize()
        .await
        .map_err(|e| format!("finalizing the order: {e}"))?;
    // What is recorded is what can prove every name again: DNS can, where
    // HTTP and TLS cannot prove a wildcard.
    let used = if kinds.contains(&Method::Dns01) {
        Method::Dns01
    } else {
        kinds.first().copied().unwrap_or(method)
    };
    Ok((used, key_pem))
}

/// Why the authority would not validate a name, in its own words.
async fn refusal(order: &mut instant_acme::Order) -> Option<String> {
    let mut reasons = Vec::new();
    let mut authorizations = order.authorizations();
    while let Some(Ok(authz)) = authorizations.next().await {
        if authz.status == AuthorizationStatus::Valid {
            continue;
        }
        let name = authz.identifier().to_string();
        let said = authz
            .challenges
            .iter()
            .find_map(|c| c.error.as_ref())
            .map(|problem| {
                problem
                    .detail
                    .clone()
                    .unwrap_or_else(|| problem.to_string())
            });
        reasons.push(match said {
            Some(said) => format!("{name}: {said}"),
            None => format!("{name}: {:?}", authz.status),
        });
    }
    (!reasons.is_empty())
        .then(|| format!("the authority could not validate: {}", reasons.join(" · ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Result<Vec<Name>, ApiError> {
        parse_names(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn names_are_read_in_any_separator_lowered_and_without_repeats() {
        let parsed = names(&[
            "Panel.Example.com, vpn.example.com",
            "panel.example.com\n*.Example.org",
            "203.0.113.7 [2001:db8::1]",
        ])
        .unwrap();
        let text: Vec<String> = parsed.iter().map(Name::text).collect();
        assert_eq!(
            text,
            [
                "panel.example.com",
                "vpn.example.com",
                "*.example.org",
                "203.0.113.7",
                "2001:db8::1"
            ]
        );
        assert!(matches!(parsed[2], Name::Wildcard(_)) && matches!(parsed[3], Name::Ip(_)));
    }

    #[test]
    fn names_that_cannot_be_certified_are_refused_with_a_reason() {
        for bad in [
            "",
            "localhost",
            "exa mple.com x",
            "-bad.example.com",
            "bad-.example.com",
            "under_score.example.com",
            "ünicode.example.com",
            "1.2.3",
            "*.com",
            "a..b.com",
            "*.*.example.com",
        ] {
            assert!(names(&[bad]).is_err(), "{bad:?}");
        }
        let long = format!("{}.example.com", "a".repeat(64));
        assert!(names(&[&long]).is_err());
        let many: Vec<String> = (0..101).map(|i| format!("h{i}.example.com")).collect();
        assert!(parse_names(&many).is_err());
    }

    #[test]
    fn a_plan_that_cannot_work_is_refused_before_the_authority_is_asked() {
        let settings = Settings::default();
        let with_token = Settings {
            cloudflare_token: "t".into(),
            ..Settings::default()
        };
        let wild = names(&["*.example.com"]).unwrap();
        let plain = names(&["example.com"]).unwrap();
        let address = names(&["203.0.113.7"]).unwrap();
        assert!(check_plan(&wild, Method::Http01, &with_token).is_err());
        assert!(check_plan(&address, Method::Dns01, &with_token).is_err());
        assert_eq!(
            check_plan(&wild, Method::Auto, &settings).unwrap_err().code,
            "cloudflare_needed"
        );
        assert_eq!(
            check_plan(&plain, Method::Dns01, &settings)
                .unwrap_err()
                .code,
            "cloudflare_needed"
        );
        assert!(check_plan(&wild, Method::Dns01, &with_token).is_ok());
        assert!(check_plan(&plain, Method::Auto, &settings).is_ok());
        assert!(check_plan(&address, Method::TlsAlpn01, &settings).is_ok());
        let mixed = names(&["*.example.com", "203.0.113.7"]).unwrap();
        assert!(check_plan(&mixed, Method::Auto, &with_token).is_err());
        let custom = Settings {
            ca: Ca::Custom,
            ..Settings::default()
        };
        assert_eq!(
            check_plan(&plain, Method::Auto, &custom).unwrap_err().code,
            "invalid_ca"
        );
    }

    #[test]
    fn methods_are_recorded_and_read_back() {
        for method in [Method::Http01, Method::TlsAlpn01, Method::Dns01] {
            assert_eq!(Method::from_label(method.label()), method);
        }
        assert_eq!(Method::from_label(""), Method::Auto);
        assert_eq!(Method::from_label("nonsense"), Method::Auto);
        assert_eq!(
            serde_json::to_string(&Method::TlsAlpn01).unwrap(),
            "\"tls-alpn-01\""
        );
    }
}
