//! What the panel needs to get certificates from a certificate authority:
//! which one, an email for it, the key it may ask for, Cloudflare's token
//! for DNS challenges, and the ports the challenges are answered on.
//! Secrets are kept in the database and never sent back to a browser.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::notify::http::Proxy;

pub const KEY: &str = "acme.settings";

pub const LETSENCRYPT: &str = "https://acme-v02.api.letsencrypt.org/directory";
pub const LETSENCRYPT_STAGING: &str = "https://acme-staging-v02.api.letsencrypt.org/directory";
pub const ZEROSSL: &str = "https://acme.zerossl.com/v2/DV90";
const CLOUDFLARE_API: &str = "https://api.cloudflare.com/client/v4";
const ZEROSSL_API: &str = "https://api.zerossl.com";

/// Which certificate authority to ask.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Ca {
    #[default]
    Letsencrypt,
    /// Let's Encrypt's test server: certificates it issues are not trusted,
    /// and its limits are loose, for trying things out.
    LetsencryptStaging,
    Zerossl,
    /// Another one, at `directory_url`.
    Custom,
}

impl Ca {
    pub fn label(self) -> &'static str {
        match self {
            Ca::Letsencrypt => "letsencrypt",
            Ca::LetsencryptStaging => "letsencrypt_staging",
            Ca::Zerossl => "zerossl",
            Ca::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ca: Ca,
    /// The directory URL of a `Custom` authority.
    pub directory_url: String,
    /// Where the authority writes to about the account (expiry notices).
    pub email: String,
    /// The key an authority that wants one (ZeroSSL, most custom ones) gives
    /// out, as an id and the HMAC key in base64url.
    pub eab_kid: String,
    pub eab_hmac: String,
    /// A Cloudflare API token that may edit DNS, for DNS challenges.
    pub cloudflare_token: String,
    /// The port HTTP challenges are answered on. The authority always asks
    /// on 80: set another only if that is forwarded to it.
    pub http_port: u16,
    /// The same for TLS-ALPN challenges (443).
    pub tls_port: u16,
    /// Start renewing this many days before expiry (a short-lived
    /// certificate starts at a third of its life instead).
    pub renew_days: u32,
    /// An `http://` or `socks5://` proxy for reaching the authority and
    /// Cloudflare; empty for none.
    pub proxy: String,
    /// Seconds to wait after writing a DNS record before the authority is
    /// asked to look.
    pub dns_wait: u64,
    /// The account on each directory, as the ACME client keeps it. Secret.
    pub accounts: BTreeMap<String, serde_json::Value>,
    /// The API addresses; only changed to point at a test double.
    pub cloudflare_api: String,
    pub zerossl_api: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ca: Ca::Letsencrypt,
            directory_url: String::new(),
            email: String::new(),
            eab_kid: String::new(),
            eab_hmac: String::new(),
            cloudflare_token: String::new(),
            http_port: 80,
            tls_port: 443,
            renew_days: 30,
            proxy: String::new(),
            dns_wait: 20,
            accounts: BTreeMap::new(),
            cloudflare_api: CLOUDFLARE_API.into(),
            zerossl_api: ZEROSSL_API.into(),
        }
    }
}

impl Settings {
    /// The directory the authority is asked at.
    pub fn directory(&self) -> String {
        match self.ca {
            Ca::Letsencrypt => LETSENCRYPT.into(),
            Ca::LetsencryptStaging => LETSENCRYPT_STAGING.into(),
            Ca::Zerossl => ZEROSSL.into(),
            Ca::Custom => self.directory_url.clone(),
        }
    }

    /// What a certificate records of who issued it.
    pub fn ca_label(&self) -> String {
        match self.ca {
            Ca::Custom => self.directory_url.clone(),
            other => other.label().into(),
        }
    }

    pub fn proxy(&self) -> Result<Option<Proxy>, String> {
        Proxy::parse(&self.proxy)
    }

    /// Whether the authority gives accounts only to those with a key.
    pub fn wants_eab(&self) -> bool {
        self.ca == Ca::Zerossl
    }

    /// Take what an admin sent: empty secrets keep what is stored, and what
    /// is not theirs to set (the accounts, the API addresses) stays.
    pub fn accept(&self, incoming: Input) -> Result<Settings, String> {
        let mut next = self.clone();
        next.ca = incoming.ca;
        next.directory_url = incoming.directory_url.trim().to_string();
        if next.ca == Ca::Custom {
            let url = url::Url::parse(&next.directory_url)
                .map_err(|_| "the directory URL of the authority is not a URL".to_string())?;
            if !matches!(url.scheme(), "https" | "http") {
                return Err("the directory URL is http:// or https://".into());
            }
        }
        next.email = incoming.email.trim().to_string();
        if !next.email.is_empty() && !(next.email.contains('@') && next.email.contains('.')) {
            return Err("that is not an email address".into());
        }
        if !incoming.eab_kid.trim().is_empty() || !incoming.eab_hmac.trim().is_empty() {
            next.eab_kid = incoming.eab_kid.trim().to_string();
        }
        if !incoming.eab_hmac.trim().is_empty() {
            next.eab_hmac = incoming.eab_hmac.trim().to_string();
        }
        if incoming.clear_eab {
            next.eab_kid.clear();
            next.eab_hmac.clear();
        }
        if !incoming.cloudflare_token.trim().is_empty() {
            next.cloudflare_token = incoming.cloudflare_token.trim().to_string();
        }
        if incoming.clear_cloudflare {
            next.cloudflare_token.clear();
        }
        if incoming.http_port == 0 || incoming.tls_port == 0 {
            return Err("a port is 1 to 65535".into());
        }
        next.http_port = incoming.http_port;
        next.tls_port = incoming.tls_port;
        if !(1..=60).contains(&incoming.renew_days) {
            return Err("renewal starts 1 to 60 days before expiry".into());
        }
        next.renew_days = incoming.renew_days;
        if incoming.dns_wait > 300 {
            return Err("waiting for DNS more than 300 seconds is too long".into());
        }
        next.dns_wait = incoming.dns_wait;
        // A proxy shown with its password hidden is the one already stored.
        let shown = redact(&self.proxy);
        next.proxy = if incoming.proxy.trim() == shown && !self.proxy.is_empty() {
            self.proxy.clone()
        } else {
            incoming.proxy.trim().to_string()
        };
        next.proxy()?;
        Ok(next)
    }

    pub fn view(&self) -> View {
        View {
            ca: self.ca,
            directory_url: self.directory_url.clone(),
            email: self.email.clone(),
            eab_kid: self.eab_kid.clone(),
            eab_hmac_set: !self.eab_hmac.is_empty(),
            cloudflare_token_set: !self.cloudflare_token.is_empty(),
            http_port: self.http_port,
            tls_port: self.tls_port,
            renew_days: self.renew_days,
            proxy: redact(&self.proxy),
            dns_wait: self.dns_wait,
            account_ready: self.accounts.contains_key(&self.directory()),
        }
    }
}

/// A proxy address with the password hidden.
fn redact(proxy: &str) -> String {
    match url::Url::parse(proxy) {
        Ok(mut url) if url.password().is_some() => {
            let _ = url.set_password(Some("•••"));
            url.to_string().trim_end_matches('/').to_string()
        }
        _ => proxy.to_string(),
    }
}

/// What the settings page sends.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct Input {
    #[serde(default)]
    pub ca: Ca,
    #[serde(default)]
    pub directory_url: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub eab_kid: String,
    /// Empty keeps the stored key.
    #[serde(default)]
    pub eab_hmac: String,
    #[serde(default)]
    pub clear_eab: bool,
    /// Empty keeps the stored token.
    #[serde(default)]
    pub cloudflare_token: String,
    #[serde(default)]
    pub clear_cloudflare: bool,
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_tls_port")]
    pub tls_port: u16,
    #[serde(default = "default_renew_days")]
    pub renew_days: u32,
    #[serde(default)]
    pub proxy: String,
    #[serde(default = "default_dns_wait")]
    pub dns_wait: u64,
}

fn default_http_port() -> u16 {
    80
}
fn default_tls_port() -> u16 {
    443
}
fn default_renew_days() -> u32 {
    30
}
fn default_dns_wait() -> u64 {
    20
}

/// What the page is shown: no secret in it.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct View {
    pub ca: Ca,
    pub directory_url: String,
    pub email: String,
    pub eab_kid: String,
    pub eab_hmac_set: bool,
    pub cloudflare_token_set: bool,
    pub http_port: u16,
    pub tls_port: u16,
    pub renew_days: u32,
    /// With the password hidden.
    pub proxy: String,
    pub dns_wait: u64,
    /// Whether an account is already registered with the chosen authority.
    pub account_ready: bool,
}

pub async fn load(db: &z_db::Database) -> Settings {
    match db.setting(KEY).await {
        Ok(Some(text)) => serde_json::from_str(&text).unwrap_or_else(|error| {
            tracing::warn!(%error, "the stored certificate settings are unreadable and were ignored");
            Settings::default()
        }),
        _ => Settings::default(),
    }
}

pub async fn save(db: &z_db::Database, settings: &Settings) -> Result<(), z_db::DbError> {
    db.set_setting(KEY, &serde_json::to_string(settings).unwrap_or_default())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Input {
        serde_json::from_str("{}").unwrap()
    }

    #[test]
    fn the_defaults_ask_letsencrypt_on_the_usual_ports() {
        let settings = Settings::default();
        assert_eq!(settings.directory(), LETSENCRYPT);
        assert_eq!((settings.http_port, settings.tls_port), (80, 443));
        assert_eq!(settings.ca_label(), "letsencrypt");
        let from_nothing: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(from_nothing, settings);
    }

    #[test]
    fn secrets_stay_when_left_empty_and_never_show_in_the_view() {
        let mut stored = Settings {
            cloudflare_token: "cf-secret".into(),
            eab_kid: "kid-1".into(),
            eab_hmac: "hmac-secret".into(),
            proxy: "socks5://bot:hunter2@127.0.0.1:1080".into(),
            ..Settings::default()
        };
        stored
            .accounts
            .insert(LETSENCRYPT.into(), serde_json::json!({"id": "x"}));

        let view = serde_json::to_string(&stored.view()).unwrap();
        for secret in ["cf-secret", "hmac-secret", "hunter2"] {
            assert!(!view.contains(secret), "{secret} leaked: {view}");
        }
        assert!(stored.view().account_ready && stored.view().cloudflare_token_set);

        // The page sends back what it was shown: nothing changes.
        let mut back = input();
        back.eab_kid = "kid-1".into();
        back.proxy = stored.view().proxy;
        let kept = stored.accept(back).unwrap();
        assert_eq!(kept.cloudflare_token, "cf-secret");
        assert_eq!(kept.eab_hmac, "hmac-secret");
        assert_eq!(kept.proxy, "socks5://bot:hunter2@127.0.0.1:1080");
        assert_eq!(
            kept.accounts, stored.accounts,
            "the accounts are not the page's"
        );

        // Typing a new one, or clearing, does change them.
        let mut change = input();
        change.cloudflare_token = "new".into();
        change.clear_eab = true;
        let changed = stored.accept(change).unwrap();
        assert_eq!(changed.cloudflare_token, "new");
        assert!(changed.eab_kid.is_empty() && changed.eab_hmac.is_empty());
        let mut clear = input();
        clear.clear_cloudflare = true;
        assert!(stored.accept(clear).unwrap().cloudflare_token.is_empty());
    }

    #[test]
    fn what_cannot_work_is_refused() {
        let stored = Settings::default();
        let refuse = |edit: fn(&mut Input)| {
            let mut i = input();
            edit(&mut i);
            stored.accept(i).unwrap_err()
        };
        assert!(refuse(|i| i.email = "nobody".into()).contains("email"));
        assert!(refuse(|i| i.http_port = 0).contains("port"));
        assert!(refuse(|i| i.renew_days = 0).contains("renewal"));
        assert!(refuse(|i| i.renew_days = 90).contains("renewal"));
        assert!(refuse(|i| i.dns_wait = 301).contains("300"));
        assert!(refuse(|i| i.proxy = "ftp://x".into()).contains("proxy"));
        assert!(refuse(|i| i.ca = Ca::Custom).contains("directory"));
        assert!(refuse(|i| {
            i.ca = Ca::Custom;
            i.directory_url = "ftp://ca.example/dir".into();
        })
        .contains("http"));
        let mut custom = input();
        custom.ca = Ca::Custom;
        custom.directory_url = "https://ca.example/dir".into();
        let custom = stored.accept(custom).unwrap();
        assert_eq!(custom.directory(), "https://ca.example/dir");
        assert_eq!(custom.ca_label(), "https://ca.example/dir");
    }
}
