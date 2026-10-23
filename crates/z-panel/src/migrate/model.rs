//! What an import is made of, whichever panel it comes from: the other
//! panel's database is read into a [`Plan`], the plan is shown to the admin
//! (with every word of caution), and only then written.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which panel a database is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
pub enum Kind {
    #[serde(rename = "3x-ui")]
    ThreeXUi,
    #[serde(rename = "pasarguard")]
    Pasarguard,
    #[serde(rename = "marzban")]
    Marzban,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::ThreeXUi => "3x-ui",
            Kind::Pasarguard => "pasarguard",
            Kind::Marzban => "marzban",
        }
    }

    pub fn parse(text: &str) -> Option<Kind> {
        match text.trim().to_ascii_lowercase().as_str() {
            "3x-ui" | "3xui" | "x-ui" | "xui" => Some(Kind::ThreeXUi),
            "pasarguard" => Some(Kind::Pasarguard),
            "marzban" => Some(Kind::Marzban),
            _ => None,
        }
    }

    /// What its subscription links are checked with: Marzban and PasarGuard
    /// sign them with the same key in the same ways; a 3X-UI link is only a
    /// token, which the user keeps.
    pub fn legacy_family(self) -> Option<&'static str> {
        match self {
            Kind::ThreeXUi => None,
            Kind::Pasarguard | Kind::Marzban => Some("marz"),
        }
    }
}

/// Something worth telling the admin about one thing that was read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Warning {
    /// `inbound`, `user`, `host`, `group`, `template`, `settings` or `source`.
    pub scope: String,
    /// What it is about: a tag, a username.
    pub name: String,
    /// A stable word for what is the matter, which the dashboard words.
    pub code: String,
    /// The same, in English, with the details.
    pub detail: String,
}

impl Warning {
    pub fn new(
        scope: &str,
        name: impl Into<String>,
        code: &str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            scope: scope.into(),
            name: name.into(),
            code: code.into(),
            detail: detail.into(),
        }
    }
}

/// A certificate an inbound presented, as the old panel kept it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedCertificate {
    /// The PEM text, certificate and key.
    Inline { cert: String, key: String },
    /// Files on the old panel's machine.
    Files { cert: String, key: String },
}

#[derive(Debug, Clone)]
pub struct PlannedInbound {
    /// The tag it had.
    pub tag: String,
    pub remark: String,
    pub enabled: bool,
    pub listen: String,
    pub port: i64,
    pub protocol: String,
    /// Without its accounts (those are the users').
    pub settings: Value,
    pub stream: Value,
    pub sniffing: Value,
    /// Taken out of `stream.tlsSettings`; the apply step puts the certificate
    /// store's id in its place.
    pub certificate: Option<PlannedCertificate>,
}

#[derive(Debug, Clone)]
pub struct PlannedGroup {
    pub name: String,
    pub inbound_tags: Vec<String>,
    pub disabled: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PlannedPlan {
    pub data_limit: i64,
    pub expire_days: i64,
    pub carry_over: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PlannedUser {
    /// The id it had in the old panel.
    pub source_id: i64,
    /// The name it had there, which an old link may name.
    pub source_name: String,
    pub username: String,
    /// `active`, `disabled`, `limited`, `expired` or `on_hold`.
    pub status: String,
    pub uuid: Option<String>,
    pub password: Option<String>,
    /// The token its subscription link had, where the old panel's links are
    /// plain tokens (3X-UI's `subId`).
    pub sub_token: Option<String>,
    pub data_limit: i64,
    pub used_traffic: i64,
    pub lifetime_traffic: i64,
    pub expire_at: Option<i64>,
    pub on_hold_duration: Option<i64>,
    pub on_hold_timeout: Option<i64>,
    pub reset_strategy: String,
    pub last_reset_at: Option<i64>,
    pub ip_limit: i64,
    pub hwid_limit: i64,
    pub note: String,
    pub online_at: Option<i64>,
    pub created_at: Option<i64>,
    /// Before this time a link of the old panel no longer counts for it.
    pub valid_after: i64,
    /// By name, as the plan's groups.
    pub group_names: Vec<String>,
    pub next_plan: Option<PlannedPlan>,
}

#[derive(Debug, Clone, Default)]
pub struct PlannedTemplate {
    pub name: String,
    pub data_limit: i64,
    pub expire_days: i64,
    pub on_hold: bool,
    pub reset_strategy: String,
    pub hwid_limit: i64,
    pub group_names: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PlannedHost {
    /// The tag of the inbound it is for.
    pub inbound_tag: String,
    pub remark: String,
    pub address: String,
    pub port: Option<i64>,
    pub sni: String,
    pub host: String,
    pub path: String,
    pub alpn: String,
    pub fingerprint: String,
    pub allow_insecure: bool,
    pub enabled: bool,
    pub priority: i64,
}

/// Subscription options the old panel had, and the paths its links used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlannedSubscription {
    /// `/sub`: where links were; ours is set to it when it can be.
    pub path: Option<String>,
    /// 3X-UI's own paths for its JSON and Clash subscriptions.
    pub json_path: Option<String>,
    pub clash_path: Option<String>,
    pub title: Option<String>,
    pub update_hours: Option<i64>,
    pub support_url: Option<String>,
}

/// Everything read from one old panel's database.
#[derive(Debug, Clone)]
pub struct Plan {
    pub kind: Kind,
    pub inbounds: Vec<PlannedInbound>,
    pub groups: Vec<PlannedGroup>,
    pub users: Vec<PlannedUser>,
    pub templates: Vec<PlannedTemplate>,
    pub hosts: Vec<PlannedHost>,
    /// Usage by the hour per old user id: `(hour, uplink, downlink)`.
    pub usage: BTreeMap<i64, Vec<(i64, i64, i64)>>,
    pub subscription: Option<PlannedSubscription>,
    /// The key the old panel signed its subscription links with.
    pub legacy_secret: Option<String>,
    pub warnings: Vec<Warning>,
}

impl Plan {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            inbounds: Vec::new(),
            groups: Vec::new(),
            users: Vec::new(),
            templates: Vec::new(),
            hosts: Vec::new(),
            usage: BTreeMap::new(),
            subscription: None,
            legacy_secret: None,
            warnings: Vec::new(),
        }
    }

    pub fn warn(
        &mut self,
        scope: &str,
        name: impl Into<String>,
        code: &str,
        detail: impl Into<String>,
    ) {
        self.warnings.push(Warning::new(scope, name, code, detail));
    }
}

/// What a plan holds, counted: what the admin is shown before choosing.
#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct Summary {
    pub inbounds: usize,
    pub users: usize,
    pub groups: usize,
    pub templates: usize,
    pub hosts: usize,
    /// How many hours of usage there are.
    pub usage_hours: usize,
    /// Whether the old panel's subscription links will keep working, and how.
    pub links: LinkSupport,
    pub inbound_names: Vec<String>,
    pub user_names: Vec<String>,
}

/// How an old panel's subscription links carry over.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkSupport {
    /// The users keep their tokens: the same address, the same link.
    #[default]
    Tokens,
    /// Links signed with the old panel's key still work.
    Signed,
}

impl Plan {
    pub fn summary(&self) -> Summary {
        let names = |items: Vec<String>| items.into_iter().take(12).collect::<Vec<_>>();
        Summary {
            inbounds: self.inbounds.len(),
            users: self.users.len(),
            groups: self.groups.len(),
            templates: self.templates.len(),
            hosts: self.hosts.len(),
            usage_hours: self.usage.values().map(Vec::len).sum(),
            links: if self.legacy_secret.is_some() {
                LinkSupport::Signed
            } else {
                LinkSupport::Tokens
            },
            inbound_names: names(self.inbounds.iter().map(|i| i.tag.clone()).collect()),
            user_names: names(self.users.iter().map(|u| u.username.clone()).collect()),
        }
    }
}
