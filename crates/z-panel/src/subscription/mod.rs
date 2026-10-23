//! Subscriptions: what a user's client app fetches to get its configs.
//!
//! A user's subscription lives at `{path}/{token}` (`/sub/…` unless the admin
//! changes it), outside the panel's secret path, since the link is handed to
//! users; the token is the secret, and revoking the user's credentials
//! changes it. It can also be served on a port of its own (`--sub-listen`),
//! for a subscription domain apart from the panel's.
//!
//! What a fetch does ([`serve`]):
//!  1. finds the user by its token;
//!  2. holds it to its device limit, by the `x-hwid` header the client sends;
//!  3. builds an endpoint for every inbound its groups give and every host of
//!     that inbound ([`endpoint`]), with its remark filled in;
//!  4. renders them in the format asked for (`?format=` or a last path part),
//!     or the one the User-Agent calls for ([`detect`]), a browser getting a
//!     page;
//!  5. answers with the usage headers clients show (`subscription-userinfo`
//!     and the rest).
//!
//! A user that may not connect (disabled, limited, expired) still gets an
//! answer, with its usage and no configs, so its app shows why.

pub mod endpoint;
pub mod formats;
pub mod page;

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::server::AppState;
use endpoint::Endpoint;

/// The setting the subscription options are kept under, as JSON.
pub const SETTING: &str = "subscription";

/// What an admin decides about subscriptions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct Settings {
    /// The path subscriptions are served under: `/sub` gives `/sub/{token}`.
    pub path: String,
    /// The address subscription links are given out with
    /// (`https://sub.example.com`); empty for the address the panel is
    /// reached at.
    pub public_url: String,
    /// The address configs point at when an inbound has no host; empty for
    /// the name the client fetched the subscription by.
    pub server_address: String,
    /// How often clients should fetch it again, in hours.
    pub update_hours: u32,
    /// The profile's name in client apps.
    pub title: String,
    pub support_url: String,
    /// Each config's name: `{remark}` with any of `{username}`, `{protocol}`,
    /// `{transport}`, `{security}`, `{data_left}`, `{days_left}`,
    /// `{expire_date}` and `{status}`.
    pub remark_template: String,
    /// Refuse a user with a device limit to an app that does not say which
    /// device it is on.
    pub hwid_required: bool,
    /// The page browsers get, in place of the built-in one: HTML in which
    /// `{{data}}` becomes the subscription as JSON.
    pub template: String,
    /// Where 3X-UI gave out its JSON and Clash subscriptions (`/json`,
    /// `/clash`): `{path}/{token}` there is the Xray or the Clash format
    /// of the same subscription. Empty for none.
    pub legacy_json_path: String,
    pub legacy_clash_path: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            path: "/sub".into(),
            public_url: String::new(),
            server_address: String::new(),
            update_hours: 12,
            title: z_common::PRODUCT.into(),
            support_url: String::new(),
            remark_template: "{remark}".into(),
            hwid_required: false,
            template: String::new(),
            legacy_json_path: String::new(),
            legacy_clash_path: String::new(),
        }
    }
}

impl Settings {
    /// Normalise and check what an admin sent. `base` is the panel's own
    /// path, which the subscription path must stay out of.
    pub fn checked(mut self, base: &str) -> Result<Self, &'static str> {
        let trimmed = self.path.trim().trim_matches('/');
        if trimmed.is_empty()
            || !trimmed
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
        {
            return Err("the path is letters, digits, '-', '_', '.' and '/', and not empty");
        }
        self.path = format!("/{trimmed}");
        if base != "/" && (self.path == base || self.path.starts_with(&format!("{base}/"))) {
            return Err("the subscription path cannot be inside the panel's path");
        }
        if base != "/" && base.starts_with(&format!("{}/", self.path)) {
            return Err("the panel's path cannot be inside the subscription path");
        }
        self.public_url = self.public_url.trim().trim_end_matches('/').to_string();
        if !self.public_url.is_empty()
            && !(self.public_url.starts_with("http://") || self.public_url.starts_with("https://"))
        {
            return Err("the public address starts with http:// or https://");
        }
        self.server_address = self.server_address.trim().to_string();
        self.update_hours = self.update_hours.clamp(1, 24 * 30);
        self.title = self.title.trim().chars().take(64).collect();
        if self.title.is_empty() {
            self.title = z_common::PRODUCT.into();
        }
        if self.remark_template.trim().is_empty() {
            self.remark_template = "{remark}".into();
        }
        self.legacy_json_path = legacy_path(&self.legacy_json_path)?;
        self.legacy_clash_path = legacy_path(&self.legacy_clash_path)?;
        for other in [&self.legacy_json_path, &self.legacy_clash_path] {
            if other.is_empty() {
                continue;
            }
            if *other == self.path {
                return Err("the JSON and Clash paths differ from the subscription path");
            }
            if base != "/"
                && (*other == base
                    || other.starts_with(&format!("{base}/"))
                    || base.starts_with(&format!("{other}/")))
            {
                return Err("the JSON and Clash paths stay out of the panel's path");
            }
        }
        if !self.legacy_json_path.is_empty() && self.legacy_json_path == self.legacy_clash_path {
            return Err("the JSON and Clash paths differ from each other");
        }
        Ok(self)
    }
}

/// An extra subscription path, normalised: `json/` gives `/json`, nothing
/// gives nothing.
fn legacy_path(path: &str) -> Result<String, &'static str> {
    let trimmed = path.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
    {
        return Err("a path is letters, digits, '-', '_', '.' and '/'");
    }
    Ok(format!("/{trimmed}"))
}

/// The stored settings, or the defaults.
pub async fn load(db: &z_db::Database) -> Settings {
    match db.setting(SETTING).await {
        Ok(Some(text)) => serde_json::from_str(&text).unwrap_or_default(),
        _ => Settings::default(),
    }
}

/// Each format a subscription is given in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Base64 of the links, one per line: what most clients take.
    Base64,
    /// The links as plain text.
    Links,
    Xray,
    Zray,
    SingBox,
    Clash,
    Page,
}

impl Format {
    fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "base64" | "v2ray" => Format::Base64,
            "links" | "plain" => Format::Links,
            "xray" | "json" | "v2ray-json" => Format::Xray,
            "zray" => Format::Zray,
            "sing-box" | "singbox" => Format::SingBox,
            "clash" | "clash-meta" | "mihomo" => Format::Clash,
            "page" | "html" => Format::Page,
            _ => return None,
        })
    }
}

/// The format a client asks for by what it calls itself: Clash and sing-box
/// apps get their own configuration, a browser the page, and everything else
/// (v2rayN, v2rayNG, Streisand, Happ, Hiddify, NekoBox…) the links.
pub fn detect(user_agent: &str, accept: &str) -> Format {
    let ua = user_agent.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| ua.contains(w));
    if has(&["clash", "mihomo", "stash", "flclash", "koala"]) {
        Format::Clash
    } else if has(&["sing-box", "singbox"])
        || ["sfa/", "sfi/", "sfm/", "sft/"]
            .iter()
            .any(|p| ua.starts_with(p))
    {
        Format::SingBox
    } else if accept.contains("text/html") && ua.contains("mozilla") {
        Format::Page
    } else {
        Format::Base64
    }
}

/// The token (and a format) when `path` is a subscription's, under `base`.
fn subscription_path<'a>(path: &'a str, base: &str) -> Option<(&'a str, Option<&'a str>)> {
    let rest = path.strip_prefix(base)?.strip_prefix('/')?;
    let mut parts = rest.split('/');
    let token = parts.next().filter(|t| !t.is_empty())?;
    let format = parts.next().filter(|f| !f.is_empty());
    if parts.next().is_some() {
        return None;
    }
    Some((token, format))
}

/// In front of every route: a subscription request is answered here, and
/// anything else goes on.
pub async fn intercept(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if matches!(*request.method(), Method::GET | Method::HEAD) {
        let settings = state.subscription();
        // The subscription path, and 3X-UI's own for its JSON and Clash ones.
        let places = [
            (settings.path.as_str(), None),
            (settings.legacy_json_path.as_str(), Some("xray")),
            (settings.legacy_clash_path.as_str(), Some("clash")),
        ];
        for (base, forced) in places {
            if base.is_empty() {
                continue;
            }
            let Some((token, format)) = subscription_path(request.uri().path(), base) else {
                continue;
            };
            if forced.is_some() && format.is_some() {
                continue;
            }
            let query_format = request.uri().query().and_then(|query| {
                query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("format="))
                    .map(str::to_string)
            });
            let format = forced
                .map(str::to_string)
                .or_else(|| format.map(str::to_string))
                .or(query_format);
            return serve(&state, token, format.as_deref(), request.headers())
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "a subscription could not be served");
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                });
        }
    }
    next.run(request).await
}

/// The router of a subscription-only listener: subscriptions, and 404 for
/// anything else.
pub fn router(state: AppState) -> axum::Router {
    axum::Router::new()
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            intercept,
        ))
        .with_state(state)
}

fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
}

/// The name the client reached us by, without a port.
pub fn request_host(headers: &HeaderMap) -> String {
    let host = header_text(headers, "host");
    if let Some(v6) = host.strip_prefix('[') {
        return v6.split(']').next().unwrap_or_default().to_string();
    }
    host.rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
        .map_or(host, |(name, _)| name)
        .to_string()
}

/// The device a client says it is on, if it says.
fn device(headers: &HeaderMap) -> Option<z_db::DeviceFields> {
    let clip = |text: &str, max: usize| text.trim().chars().take(max).collect::<String>();
    let hwid = clip(header_text(headers, "x-hwid"), 128);
    (!hwid.is_empty()).then(|| z_db::DeviceFields {
        hwid,
        platform: clip(header_text(headers, "x-device-os"), 64),
        os_version: clip(header_text(headers, "x-ver-os"), 64),
        model: clip(header_text(headers, "x-device-model"), 128),
        user_agent: clip(header_text(headers, "user-agent"), 256),
    })
}

fn refused(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        message.to_string(),
    )
        .into_response()
}

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

fn data_left(user: &z_db::User) -> String {
    if user.data_limit <= 0 {
        return "∞".into();
    }
    let left = (user.data_limit - user.used_traffic).max(0) as f64 / GB;
    format!("{left:.2} GB")
}

fn days_left(user: &z_db::User, now: i64) -> String {
    match user.expire_at {
        None => "∞".into(),
        Some(at) => ((at - now).max(0) / 86_400).to_string(),
    }
}

/// A date for people, from Unix seconds, in UTC: `2026-11-12`.
pub fn date(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// `template` with `e`'s and `user`'s values in place of its variables.
fn remark(template: &str, e: &Endpoint, user: &z_db::User, now: i64) -> String {
    template
        .replace("{remark}", &e.remark)
        .replace("{username}", &user.username)
        .replace("{protocol}", e.protocol.name())
        .replace("{transport}", e.transport.name())
        .replace("{security}", e.security.name())
        .replace("{data_left}", &data_left(user))
        .replace("{days_left}", &days_left(user, now))
        .replace(
            "{expire_date}",
            &user.expire_at.map(date).unwrap_or_else(|| "∞".into()),
        )
        .replace("{status}", &user.status)
}

/// Every endpoint `user` has now, with its remark filled in.
pub async fn user_endpoints(
    state: &AppState,
    user: &z_db::User,
    server: &str,
) -> Result<Vec<Endpoint>, z_db::DbError> {
    if !crate::accounts::connectable(user) {
        return Ok(Vec::new());
    }
    let db = state.db();
    let inbounds = db.inbounds().await?;
    let groups = db.access_groups().await?;
    let hosts = db.hosts().await?;
    let host_refs: Vec<&z_db::Host> = hosts.iter().collect();
    let certificates: HashMap<i64, z_db::Certificate> = db
        .certificates()
        .await?
        .into_iter()
        .map(|cert| (cert.id, cert))
        .collect();
    let template = state.subscription().remark_template;
    let now = z_db::now();
    Ok(inbounds
        .iter()
        .filter(|inbound| inbound.enabled && crate::accounts::grants(&groups, user, inbound))
        .flat_map(|inbound| endpoint::endpoints(inbound, &host_refs, user, server, &certificates))
        .map(|mut e| {
            e.remark = remark(&template, &e, user, now);
            e
        })
        .collect())
}

/// The subscription's address as users are given it.
pub fn url(settings: &Settings, origin: &str, token: &str) -> String {
    let origin = if settings.public_url.is_empty() {
        origin
    } else {
        &settings.public_url
    };
    format!("{origin}{}/{token}", settings.path)
}

/// Answer one fetch of the subscription with `token` (see the module's
/// description).
pub async fn serve(
    state: &AppState,
    token: &str,
    format: Option<&str>,
    headers: &HeaderMap,
) -> Result<Response, z_db::DbError> {
    let db = state.db();
    let Some(user) = crate::migrate::legacy::find_user(db, token).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let settings = state.subscription();
    let user_agent = header_text(headers, "user-agent");
    let format = format
        .and_then(Format::parse)
        .unwrap_or_else(|| detect(user_agent, header_text(headers, "accept")));

    // A browser looking at the page is not a device using the configs.
    if format != Format::Page {
        let seen = device(headers);
        if user.hwid_limit > 0 {
            match &seen {
                None if settings.hwid_required => {
                    return Ok(refused(
                        "This subscription needs an app that says which device it is on (x-hwid).",
                    ))
                }
                None => {}
                Some(device) => {
                    let devices = db.user_devices(user.id).await?;
                    let known = devices.iter().any(|d| d.hwid == device.hwid);
                    if !known && devices.len() as i64 >= user.hwid_limit {
                        return Ok(refused(
                            "This subscription is already in use on as many devices as it allows.",
                        ));
                    }
                }
            }
        }
        if let Some(device) = &seen {
            db.save_device(user.id, device).await?;
        }
        db.touch_subscription(user.id, &user_agent.chars().take(256).collect::<String>())
            .await?;
    }

    let host = request_host(headers);
    let server = if settings.server_address.is_empty() {
        host.clone()
    } else {
        settings.server_address.clone()
    };
    let endpoints = user_endpoints(state, &user, &server).await?;
    let scheme = if state.tls() { "https" } else { "http" };
    let origin = format!("{scheme}://{}", header_text(headers, "host"));
    let sub_url = url(&settings, &origin, &user.sub_token);

    let (content_type, body) = match format {
        Format::Base64 => (
            "text/plain; charset=utf-8",
            formats::base64_links(&endpoints),
        ),
        Format::Links => (
            "text/plain; charset=utf-8",
            endpoints
                .iter()
                .map(formats::link)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Format::Xray | Format::Zray => (
            "application/json",
            serde_json::to_string_pretty(&formats::xray_configs(
                &endpoints,
                format == Format::Zray,
            ))
            .unwrap_or_default(),
        ),
        Format::SingBox => (
            "application/json",
            serde_json::to_string_pretty(&formats::singbox_config(&endpoints)).unwrap_or_default(),
        ),
        Format::Clash => (
            "text/yaml; charset=utf-8",
            formats::clash_config(&endpoints),
        ),
        Format::Page => (
            "text/html; charset=utf-8",
            page::render(
                &settings,
                &user,
                &endpoints,
                &sub_url,
                header_text(headers, "accept-language"),
            ),
        ),
    };

    let mut response = Response::new(Body::from(body));
    let out = response.headers_mut();
    let mut set = |name: &'static str, value: String| {
        if let Ok(value) = HeaderValue::from_str(&value) {
            out.insert(name, value);
        }
    };
    set("content-type", content_type.into());
    set(
        "subscription-userinfo",
        format!(
            "upload=0; download={}; total={}; expire={}",
            user.used_traffic.max(0),
            user.data_limit.max(0),
            user.expire_at.unwrap_or(0).max(0)
        ),
    );
    set("profile-update-interval", settings.update_hours.to_string());
    set(
        "profile-title",
        format!(
            "base64:{}",
            base64::engine::general_purpose::STANDARD.encode(&settings.title)
        ),
    );
    set("profile-web-page-url", sub_url.clone());
    if !settings.support_url.is_empty() {
        set("support-url", settings.support_url.clone());
    }
    if format != Format::Page {
        let filename: String = settings
            .title
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            .collect();
        let filename = if filename.is_empty() {
            "subscription".into()
        } else {
            filename
        };
        set(
            "content-disposition",
            format!("attachment; filename=\"{filename}\""),
        );
    }
    set("cache-control", "no-store".into());
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_decides_the_format() {
        assert_eq!(detect("ClashMetaForAndroid/2.10", "*/*"), Format::Clash);
        assert_eq!(detect("mihomo/1.19", ""), Format::Clash);
        assert_eq!(detect("SFA/1.11.0 (sing-box 1.11.0)", ""), Format::SingBox);
        assert_eq!(detect("v2rayNG/1.9.30", ""), Format::Base64);
        assert_eq!(detect("Happ/1.0", ""), Format::Base64);
        assert_eq!(
            detect(
                "Mozilla/5.0 (X11; Linux x86_64) Safari/537.36",
                "text/html,application/xhtml+xml"
            ),
            Format::Page
        );
        assert_eq!(Format::parse("sing-box"), Some(Format::SingBox));
        assert_eq!(Format::parse("nope"), None);
    }

    #[test]
    fn a_subscription_path_holds_a_token_and_maybe_a_format() {
        assert_eq!(subscription_path("/sub/abc", "/sub"), Some(("abc", None)));
        assert_eq!(
            subscription_path("/sub/abc/clash", "/sub"),
            Some(("abc", Some("clash")))
        );
        assert_eq!(subscription_path("/sub", "/sub"), None);
        assert_eq!(subscription_path("/subway/abc", "/sub"), None);
        assert_eq!(subscription_path("/sub/a/b/c", "/sub"), None);
    }

    #[test]
    fn settings_are_checked_against_the_panel_path() {
        let with = |path: &str| Settings {
            path: path.into(),
            ..Settings::default()
        };
        assert_eq!(with("sub/").checked("/panel").unwrap().path, "/sub");
        assert!(with("/panel/sub").checked("/panel").is_err());
        assert!(with("/").checked("/panel").is_err());
        assert!(with("/a b").checked("/").is_err());
        assert!(with("/p").checked("/p/x").is_err());
        let public = Settings {
            public_url: "sub.example.com".into(),
            ..Settings::default()
        };
        assert!(public.checked("/").is_err());
    }

    #[test]
    fn dates_and_hosts_read_plainly() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_798_675_200), "2026-12-31");
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("sub.example.com:2096"));
        assert_eq!(request_host(&headers), "sub.example.com");
        headers.insert("host", HeaderValue::from_static("[2001:db8::1]:443"));
        assert_eq!(request_host(&headers), "2001:db8::1");
    }
}
