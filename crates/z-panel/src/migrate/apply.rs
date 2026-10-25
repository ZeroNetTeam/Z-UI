//! Writing a [`Plan`] into this panel. What is written is first made safe: a
//! backup of everything the panel holds is taken (and the import does not go
//! on without it), every inbound is run through the core's own compiler
//! before it is kept, and a name that is taken is skipped or renamed as the
//! admin chose. Nothing is all-or-nothing: what cannot be taken is left out and
//! said, and what can is taken.

use std::collections::{HashMap, HashSet};

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use z_db::{ImportedUser, InboundFields, NextPlan, UserFields, UserTemplate};

use super::model::{LinkSupport, Plan, PlannedCertificate, Warning};
use crate::error::ApiError;
use crate::server::AppState;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Conflict {
    /// A user or template whose name is taken is left out.
    #[default]
    Skip,
    /// It comes in under another name (`ali-2`).
    Rename,
}

fn yes() -> bool {
    true
}

/// What to bring over, and how.
#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct Options {
    #[serde(default = "yes")]
    pub inbounds: bool,
    #[serde(default = "yes")]
    pub users: bool,
    #[serde(default = "yes")]
    pub hosts: bool,
    #[serde(default = "yes")]
    pub templates: bool,
    /// The old panel's admins, without passwords: each is given a link that
    /// sets one. Their users stay theirs.
    #[serde(default = "yes")]
    pub admins: bool,
    #[serde(default = "yes")]
    pub usage: bool,
    /// The old panel's subscription path, title and so on.
    #[serde(default = "yes")]
    pub settings: bool,
    /// Turn the imported inbounds on. Off, they are kept turned off: the old
    /// panel still has their ports until it is stopped.
    #[serde(default)]
    pub enable_inbounds: bool,
    #[serde(default)]
    pub on_conflict: Conflict,
    /// Hand the result to the core now. Not for the command line, which has
    /// no core of its own: the panel takes it up when it starts.
    #[serde(skip, default = "yes")]
    pub sync_core: bool,
    /// Users (by the name they have here) that an earlier step of a takeover
    /// made: they are brought up to date with the old panel instead of left
    /// out, and their usage is what the old panel counted.
    #[serde(skip)]
    pub refresh_usernames: HashSet<String>,
}

impl Default for Options {
    fn default() -> Self {
        serde_json::from_str("{}").expect("every field has a default")
    }
}

#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct Counts {
    pub inbounds: usize,
    pub users: usize,
    pub groups: usize,
    pub templates: usize,
    pub hosts: usize,
    pub admins: usize,
    pub certificates: usize,
    pub usage_hours: usize,
    /// Users that were already here and were brought up to date.
    pub refreshed: usize,
}

/// What an import did.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Report {
    pub imported: Counts,
    /// What was left out, and what was only approximated: each with its word.
    pub warnings: Vec<Warning>,
    /// The backup taken first; restoring it undoes the import.
    pub backup: String,
    pub links: LinkSupport,
    /// Whether the core took the new configuration (it may not when its ports
    /// are still held by the old panel).
    pub core_applied: bool,
    pub core_error: Option<String>,
    /// Each inbound's tag in the old panel and the tag it has here (the same
    /// inbound when it was already here).
    #[serde(skip)]
    #[schema(ignore)]
    pub tag_map: Vec<(String, String)>,
}

fn skip(
    warnings: &mut Vec<Warning>,
    scope: &str,
    name: &str,
    code: &str,
    detail: impl Into<String>,
) {
    warnings.push(Warning::new(scope, name, code, detail));
}

/// An admin name this panel accepts (3 to 32 letters, digits, `.`, `_`, `-`).
fn admin_name(name: &str, taken: &HashMap<String, i64>) -> String {
    let mut clean: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(32)
        .collect();
    while clean.len() < 3 {
        clean.push('_');
    }
    if clean == name || !taken.contains_key(&clean) {
        return clean;
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            format!(
                "{}{suffix}",
                clean.chars().take(32 - suffix.len()).collect::<String>()
            )
        })
        .find(|candidate| !taken.contains_key(candidate))
        .expect("an unused name")
}

/// A name that is free: `name`, or with `-2`, `-3`… when asked to rename.
fn free_name(
    name: &str,
    taken: &HashSet<String>,
    conflict: Conflict,
    limit: usize,
) -> Option<String> {
    if !taken.contains(name) {
        return Some(name.to_string());
    }
    if conflict == Conflict::Skip {
        return None;
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            format!(
                "{}{suffix}",
                name.chars().take(limit - suffix.len()).collect::<String>()
            )
        })
        .find(|candidate| !taken.contains(candidate))
}

/// The certificate store's id for what an inbound presented, adding it to
/// the store unless the same one is there.
pub(crate) async fn certificate_id(
    state: &AppState,
    cert: &PlannedCertificate,
    label: &str,
    known: &mut Vec<z_db::Certificate>,
    added: &mut usize,
) -> Result<i64, String> {
    let (cert_pem, key_pem) = match cert {
        PlannedCertificate::Inline { cert, key } => (cert.clone(), key.clone()),
        PlannedCertificate::Files { cert, key } => {
            let read = |path: &str| {
                std::fs::read_to_string(path)
                    .map_err(|_| format!("the file {path} cannot be read here"))
            };
            (read(cert)?, read(key)?)
        }
    };
    let (cert_pem, key_pem) = (cert_pem.trim().to_string(), key_pem.trim().to_string());
    let parsed = crate::certs::inspect(&cert_pem, &key_pem).map_err(|e| e.message)?;
    if let Some(same) = known.iter().find(|c| c.cert_pem.trim() == cert_pem) {
        return Ok(same.id);
    }
    let made = state
        .db()
        .add_certificate(z_db::NewCertificate {
            name: parsed
                .domains
                .first()
                .cloned()
                .unwrap_or_else(|| label.to_string())
                .chars()
                .take(128)
                .collect(),
            domains: parsed.domains.join(","),
            cert_pem,
            key_pem,
            source: "upload".into(),
            not_before: parsed.not_before,
            not_after: parsed.not_after,
            ..Default::default()
        })
        .await
        .map_err(|e| e.to_string())?;
    *added += 1;
    let id = made.id;
    known.push(made);
    Ok(id)
}

pub async fn apply(state: &AppState, plan: Plan, options: &Options) -> Result<Report, ApiError> {
    let backup = crate::backup::save_before(state, "-before-import")
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "backup_failed",
                format!("the import did not start, because a backup of what is here could not be made first: {error}"),
            )
        })?;
    let mut warnings = plan.warnings.clone();
    let mut counts = Counts::default();
    let db = state.db();
    let guard = state.core_lock().lock().await;

    // ---- subscription options
    if options.settings {
        if let Some(wanted) = &plan.subscription {
            let mut settings = state.subscription();
            if let Some(path) = &wanted.path {
                settings.path = path.clone();
            }
            if let Some(path) = &wanted.json_path {
                settings.legacy_json_path = path.clone();
            }
            if let Some(path) = &wanted.clash_path {
                settings.legacy_clash_path = path.clone();
            }
            if let Some(title) = &wanted.title {
                settings.title = title.clone();
            }
            if let Some(hours) = wanted.update_hours {
                settings.update_hours = hours.clamp(1, 24 * 30) as u32;
            }
            if let Some(url) = &wanted.support_url {
                settings.support_url = url.clone();
            }
            match settings.clone().checked(&state.config().base_path) {
                Ok(checked) => {
                    let text = serde_json::to_string(&checked).map_err(ApiError::internal)?;
                    db.set_setting(crate::subscription::SETTING, &text).await?;
                    state.set_subscription(checked);
                }
                Err(why) => skip(
                    &mut warnings,
                    "settings",
                    "subscription",
                    "subscription_settings_refused",
                    format!("the old panel's subscription settings were not taken: {why}"),
                ),
            }
        }
    }

    // ---- inbounds
    let mut existing = db.inbounds().await?;
    let mut inbound_ids: HashMap<String, i64> = HashMap::new();
    for inbound in &existing {
        inbound_ids.insert(inbound.tag.clone(), inbound.id);
    }
    let mut taken_tags: HashSet<String> = existing.iter().map(|i| i.tag.clone()).collect();
    let mut tag_of: HashMap<String, String> = HashMap::new();
    let mut certificates = db.certificates().await?;
    if options.inbounds {
        let core_settings = crate::core_settings::load(db).await;
        for planned in &plan.inbounds {
            // The same inbound already here (an earlier import of this panel)
            // is the one the users go to, not a second one beside it.
            if let Some(same) = existing.iter().find(|i| {
                i.tag == planned.tag && i.protocol == planned.protocol && i.port == planned.port
            }) {
                tag_of.insert(planned.tag.clone(), same.tag.clone());
                continue;
            }
            let Some(tag) = free_name(&planned.tag, &taken_tags, Conflict::Rename, 64) else {
                continue;
            };
            let mut stream = planned.stream.clone();
            if let Some(certificate) = &planned.certificate {
                match certificate_id(state, certificate, &tag, &mut certificates, &mut counts.certificates).await {
                    Ok(id) => {
                        if let Some(tls) = stream.get_mut("tlsSettings").and_then(serde_json::Value::as_object_mut) {
                            tls.remove("certificates");
                            tls.insert("certificateId".into(), id.into());
                        }
                    }
                    Err(why) => skip(&mut warnings, "inbound", &planned.tag, "cert_not_taken", format!("its certificate was not brought into the store ({why}); it keeps naming the files")),
                }
            }
            if crate::core_config::accounts_key(&planned.protocol).is_none() {
                skip(
                    &mut warnings,
                    "inbound",
                    &planned.tag,
                    "skipped_protocol",
                    format!("the panel does not make {:?} inbounds", planned.protocol),
                );
                continue;
            }
            let remark: String = planned.remark.trim().chars().take(128).collect();
            let fields = InboundFields {
                tag: tag.clone(),
                remark: if remark.is_empty() {
                    tag.clone()
                } else {
                    remark
                },
                enabled: options.enable_inbounds && planned.enabled,
                listen: planned.listen.clone(),
                port: planned.port,
                protocol: planned.protocol.clone(),
                settings: planned.settings.to_string(),
                stream: stream.to_string(),
                sniffing: planned.sniffing.to_string(),
            };
            if !(1..=65535).contains(&fields.port) {
                skip(
                    &mut warnings,
                    "inbound",
                    &planned.tag,
                    "skipped_invalid",
                    "its port is not between 1 and 65535",
                );
                continue;
            }
            // The core's own compiler is the judge, with this one turned on.
            let mut trial = existing.clone();
            trial.push(z_db::Inbound {
                id: 0,
                tag: fields.tag.clone(),
                remark: fields.remark.clone(),
                enabled: true,
                listen: fields.listen.clone(),
                port: fields.port,
                protocol: fields.protocol.clone(),
                settings: fields.settings.clone(),
                stream: fields.stream.clone(),
                sniffing: fields.sniffing.clone(),
                created_at: 0,
                updated_at: 0,
            });
            if let Err(error) = crate::core_config::compile(state, &trial, &core_settings).await {
                skip(
                    &mut warnings,
                    "inbound",
                    &planned.tag,
                    "skipped_core",
                    error.message,
                );
                continue;
            }
            let made = db.create_inbound(&fields).await?;
            inbound_ids.insert(made.tag.clone(), made.id);
            taken_tags.insert(made.tag.clone());
            tag_of.insert(planned.tag.clone(), made.tag.clone());
            existing.push(made);
            counts.inbounds += 1;
        }
    } else {
        for planned in &plan.inbounds {
            if inbound_ids.contains_key(&planned.tag) {
                tag_of.insert(planned.tag.clone(), planned.tag.clone());
            }
        }
    }

    // ---- groups
    let mut group_ids: HashMap<String, i64> = HashMap::new();
    let mut known_groups = db.access_groups().await?;
    let mut taken_groups: HashSet<String> = known_groups.iter().map(|g| g.name.clone()).collect();
    for group in &plan.groups {
        let inbounds: Vec<i64> = if group.disabled {
            skip(
                &mut warnings,
                "group",
                &group.name,
                "group_disabled",
                "it was disabled there, so here it holds no inbounds",
            );
            Vec::new()
        } else {
            let mut ids: Vec<i64> = group
                .inbound_tags
                .iter()
                .filter_map(|t| tag_of.get(t))
                .filter_map(|t| inbound_ids.get(t).copied())
                .collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        // The same group already here (an earlier import) is used as it is.
        if let Some(same) = known_groups.iter().find(|g| {
            let mut have = g.inbound_ids.clone();
            have.sort_unstable();
            g.name == group.name && !g.all_inbounds && have == inbounds
        }) {
            group_ids.insert(group.name.clone(), same.id);
            continue;
        }
        let Some(name) = free_name(&group.name, &taken_groups, Conflict::Rename, 64) else {
            continue;
        };
        let id = db.save_access_group(None, &name, false, &inbounds).await?;
        taken_groups.insert(name.clone());
        known_groups.push(z_db::AccessGroup {
            id,
            name,
            all_inbounds: false,
            inbound_ids: inbounds,
            created_at: 0,
        });
        group_ids.insert(group.name.clone(), id);
        counts.groups += 1;
    }

    // ---- admins: without passwords, and each user stays its admin's
    let mut owner_ids: HashMap<i64, i64> = HashMap::new();
    if options.admins {
        let mut by_name: HashMap<String, i64> = db
            .admins()
            .await?
            .into_iter()
            .map(|a| (a.username, a.id))
            .collect();
        let reseller = db
            .roles()
            .await?
            .into_iter()
            .find(|r| r.name == "Reseller")
            .map(|r| r.id);
        for planned in &plan.admins {
            let name = admin_name(&planned.name, &by_name);
            if let Some(id) = by_name.get(&planned.name).or_else(|| by_name.get(&name)) {
                // Somebody of that name is here already: its users are theirs.
                owner_ids.insert(planned.source_id, *id);
                continue;
            }
            let made = db
                .create_admin_with(
                    &name,
                    z_db::NO_PASSWORD,
                    &z_db::AdminFields {
                        is_sudo: planned.sudo,
                        role_id: if planned.sudo { None } else { reseller },
                        enabled: planned.enabled,
                        note: format!("from {}", plan.kind.label()),
                        ..Default::default()
                    },
                )
                .await?;
            if name != planned.name {
                skip(
                    &mut warnings,
                    "settings",
                    &planned.name,
                    "renamed",
                    format!("the admin's name became {name}"),
                );
            }
            by_name.insert(name, made.id);
            owner_ids.insert(planned.source_id, made.id);
            counts.admins += 1;
        }
    }

    // ---- users
    let keep_old_links = match (plan.kind.legacy_family(), &plan.legacy_secret) {
        (Some(_), Some(secret)) => match db.legacy_secret(super::legacy::KIND).await? {
            Some(kept) if &kept != secret => {
                skip(&mut warnings, "settings", plan.kind.label(), "old_links_not_kept", "the signed links of another panel are already kept, and only one panel's can be");
                false
            }
            _ => true,
        },
        (Some(_), None) => {
            skip(&mut warnings, "settings", plan.kind.label(), "old_links_not_kept", "the old panel's signing key was not in its database, so its links cannot be checked");
            false
        }
        _ => false,
    };
    let mut taken_users: HashSet<String> = db
        .all_users()
        .await?
        .into_iter()
        .map(|u| u.username)
        .collect();
    let mut taken_tokens: HashSet<String> = db
        .all_users()
        .await?
        .into_iter()
        .map(|u| u.sub_token)
        .collect();
    let mut imported_users = 0usize;
    if options.users {
        let now = z_db::now();
        let fields_of = |planned: &super::model::PlannedUser, username: String| UserFields {
            username,
            status: planned.status.clone(),
            data_limit: planned.data_limit,
            expire_at: planned.expire_at,
            on_hold_duration: planned.on_hold_duration,
            on_hold_timeout: planned.on_hold_timeout,
            reset_strategy: planned.reset_strategy.clone(),
            ip_limit: planned.ip_limit,
            speed_up: 0,
            speed_down: 0,
            note: planned.note.clone(),
            group_ids: planned
                .group_names
                .iter()
                .filter_map(|g| group_ids.get(g).copied())
                .collect(),
            next_plan: planned.next_plan.as_ref().map(|p| NextPlan {
                data_limit: p.data_limit,
                expire_days: p.expire_days,
                carry_over: p.carry_over,
            }),
            hwid_limit: planned.hwid_limit,
        };
        let mut present: HashMap<String, i64> = HashMap::new();
        if !options.refresh_usernames.is_empty() {
            for user in db.all_users().await? {
                present.insert(user.username, user.id);
            }
        }
        for planned in &plan.users {
            if let (true, Some(id)) = (
                options.refresh_usernames.contains(&planned.username),
                present.get(&planned.username).copied(),
            ) {
                db.update_user(id, &fields_of(planned, planned.username.clone()))
                    .await?;
                db.set_user_imported(
                    id,
                    &ImportedUser {
                        used_traffic: planned.used_traffic,
                        lifetime_traffic: planned.lifetime_traffic,
                        created_at: planned.created_at.unwrap_or(now),
                        last_reset_at: planned.last_reset_at,
                        online_at: planned.online_at,
                    },
                )
                .await?;
                if options.usage {
                    if let Some(hours) = plan.usage.get(&planned.source_id) {
                        db.replace_usage_hours(id, hours).await?;
                        counts.usage_hours += hours.len();
                    }
                }
                counts.refreshed += 1;
                continue;
            }
            let Some(username) =
                free_name(&planned.username, &taken_users, options.on_conflict, 64)
            else {
                skip(
                    &mut warnings,
                    "user",
                    &planned.username,
                    "skipped_exists",
                    "a user with this name is already here",
                );
                continue;
            };
            let fields = fields_of(planned, username.clone());
            let uuid = planned
                .uuid
                .clone()
                .unwrap_or_else(zero_runtime::keys::uuid);
            let password = planned
                .password
                .clone()
                .unwrap_or_else(|| zero_runtime::keys::password(18));
            let token = planned
                .sub_token
                .clone()
                .filter(|t| !taken_tokens.contains(t))
                .unwrap_or_else(|| hex::encode(rand::random::<[u8; 16]>()));
            let owner = planned
                .owner
                .and_then(|source| owner_ids.get(&source))
                .copied();
            let made = match db
                .create_user(&fields, &uuid, &password, &token, owner)
                .await
            {
                Ok(user) => user,
                Err(error) => {
                    skip(
                        &mut warnings,
                        "user",
                        &planned.username,
                        "skipped_error",
                        error.to_string(),
                    );
                    continue;
                }
            };
            taken_users.insert(username);
            taken_tokens.insert(token);
            db.set_user_imported(
                made.id,
                &ImportedUser {
                    used_traffic: planned.used_traffic,
                    lifetime_traffic: planned.lifetime_traffic,
                    created_at: planned.created_at.unwrap_or(now),
                    last_reset_at: planned.last_reset_at,
                    online_at: planned.online_at,
                },
            )
            .await?;
            if keep_old_links {
                db.add_legacy_user(
                    super::legacy::KIND,
                    planned.source_id,
                    &planned.source_name,
                    made.id,
                    planned.valid_after,
                )
                .await?;
            }
            if options.usage {
                if let Some(hours) = plan.usage.get(&planned.source_id) {
                    db.add_usage_hours(made.id, hours).await?;
                    counts.usage_hours += hours.len();
                }
            }
            imported_users += 1;
        }
        counts.users = imported_users;
        if keep_old_links && imported_users > 0 {
            if let Some(secret) = &plan.legacy_secret {
                db.set_legacy_secret(super::legacy::KIND, secret).await?;
            }
        }
    }

    // ---- templates
    if options.templates {
        let mut taken: HashSet<String> = db
            .user_templates()
            .await?
            .into_iter()
            .map(|t| t.name)
            .collect();
        for planned in &plan.templates {
            let Some(name) = free_name(&planned.name, &taken, options.on_conflict, 64) else {
                skip(
                    &mut warnings,
                    "template",
                    &planned.name,
                    "skipped_exists",
                    "a template with this name is already here",
                );
                continue;
            };
            db.save_user_template(&UserTemplate {
                id: 0,
                name: name.clone(),
                data_limit: planned.data_limit,
                expire_days: planned.expire_days,
                on_hold: planned.on_hold,
                reset_strategy: planned.reset_strategy.clone(),
                hwid_limit: planned.hwid_limit,
                group_ids: planned
                    .group_names
                    .iter()
                    .filter_map(|g| group_ids.get(g).copied())
                    .collect(),
                ..Default::default()
            })
            .await?;
            taken.insert(name);
            counts.templates += 1;
        }
    }

    // ---- hosts
    if options.hosts && options.inbounds {
        let mut known_hosts = db.hosts().await?;
        for host in &plan.hosts {
            let Some(inbound) = tag_of
                .get(&host.inbound_tag)
                .and_then(|t| inbound_ids.get(t))
            else {
                continue;
            };
            let remark: String = host.remark.chars().take(128).collect();
            if known_hosts.iter().any(|h| {
                h.inbound_id == *inbound
                    && h.fields.remark == remark
                    && h.fields.address == host.address
                    && h.fields.port == host.port
            }) {
                continue;
            }
            let made = db
                .create_host(
                    *inbound,
                    &z_db::HostFields {
                        remark,
                        address: host.address.clone(),
                        port: host.port,
                        sni: host.sni.clone(),
                        host: host.host.clone(),
                        path: host.path.clone(),
                        alpn: host.alpn.clone(),
                        fingerprint: host.fingerprint.clone(),
                        allow_insecure: host.allow_insecure,
                        enabled: host.enabled,
                        priority: host.priority,
                    },
                )
                .await?;
            known_hosts.push(made);
            counts.hosts += 1;
        }
    }
    drop(guard);

    // ---- the core takes what is now in the database
    let (core_applied, core_error) =
        if options.sync_core && (counts.inbounds > 0 || imported_users > 0) {
            match crate::core_config::sync(state).await {
                Ok(_) => (true, None),
                Err(error) => (false, Some(error.message)),
            }
        } else {
            (true, None)
        };
    tracing::info!(
        kind = plan.kind.label(),
        inbounds = counts.inbounds,
        users = counts.users,
        "an import from another panel was made"
    );
    Ok(Report {
        links: if keep_old_links && imported_users > 0 {
            LinkSupport::Signed
        } else {
            LinkSupport::Tokens
        },
        imported: counts,
        warnings,
        backup,
        core_applied,
        core_error,
        tag_map: plan
            .inbounds
            .iter()
            .filter_map(|p| tag_of.get(&p.tag).map(|t| (p.tag.clone(), t.clone())))
            .collect(),
    })
}
