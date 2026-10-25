//! The program's commands for use on the server: admins, where the panel
//! listens, certificates, updates, backups, and a health check. They work on
//! the database directly, so they need no sign-in and no running panel (what
//! a running panel holds in memory is read again when it restarts).

use std::sync::Arc;

use anyhow::Context;

use crate::auth;
use crate::auth::session::SessionKeys;
use crate::certs::{self, acme};
use crate::config::{
    AdminCommand, CertCommand, Command, MigrateArgs, PanelConfig, SettingCommand, TakeoverCommand,
    UpdateCommand,
};
use crate::panel_settings::{self, Stored};
use crate::server::AppState;
use crate::{backup, monitor, notify, tls, update};

pub async fn run(
    config: &PanelConfig,
    db: &z_db::Database,
    stored: &Stored,
    command: &Command,
) -> anyhow::Result<()> {
    match command {
        Command::Admin(command) => admin(config, db, stored, command).await,
        Command::Setting(command) => setting(db, config, stored, command).await,
        Command::Cert(command) => cert(config, db, command).await,
        Command::Update(command) => update_command(db, command).await,
        Command::Backup { out } => backup_now(config, db, out.as_deref()).await,
        Command::Restore { file, passphrase } => {
            restore(config, db, file, passphrase.as_deref()).await
        }
        Command::Migrate(args) => migrate(config, db, args).await,
        Command::Takeover(command) => takeover(config, db, command).await,
        Command::Health => health(config, stored).await,
    }
}

/// A panel's state without a running panel: what the work of the certificate
/// store and the backups needs, and no listener, core or notifier worker.
async fn offline(config: &PanelConfig, db: &z_db::Database) -> AppState {
    let state = AppState::new(
        config.clone(),
        db.clone(),
        Arc::new(z_node::LocalNode::new()),
        SessionKeys::new(b"offline"),
        None,
        monitor::Monitor::new(Default::default(), config.data_dir.clone()),
    );
    state.set_notify_settings(notify::load(db).await);
    state
}

async fn admin(
    config: &PanelConfig,
    db: &z_db::Database,
    stored: &Stored,
    command: &AdminCommand,
) -> anyhow::Result<()> {
    match command {
        AdminCommand::Create {
            username,
            password,
            not_sudo,
        } => {
            auth::password::check_username(username).map_err(|e| anyhow::anyhow!(e.message))?;
            let (password, generated) = match password {
                Some(password) => (password.clone(), false),
                None => (hex::encode(rand::random::<[u8; 9]>()), true),
            };
            auth::password::check_strength(&password).map_err(|e| anyhow::anyhow!(e.message))?;
            let hash = auth::password::hash(&password)
                .await
                .map_err(|e| anyhow::anyhow!(e.message))?;
            db.create_admin(username, &hash, !not_sudo)
                .await
                .with_context(|| {
                    format!("making the admin {username:?} (is there one already?)")
                })?;
            println!(
                "made the admin {username:?}{}",
                if *not_sudo { "" } else { " (sudo)" }
            );
            if generated {
                println!("password: {password}");
            }
            Ok(())
        }
        AdminCommand::Invite {
            username,
            pending,
            host,
        } => {
            let admins = match (username, pending) {
                (Some(name), _) => vec![db
                    .admin_by_username(name)
                    .await?
                    .with_context(|| format!("there is no admin {name:?}"))?],
                (None, true) => db.pending_admins().await?,
                (None, false) => anyhow::bail!("give an admin's name, or --pending"),
            };
            let scheme = if config.cert.is_some() || stored.certificate_id.is_some() {
                "https"
            } else {
                "http"
            };
            for admin in admins {
                let token = auth::invite::issue(db, admin.id)
                    .await
                    .map_err(|e| anyhow::anyhow!(e.message))?;
                println!(
                    "{}  {scheme}://{host}:{}{}",
                    admin.username,
                    config.listen.port(),
                    config.under_base(&format!("/invite/{token}"))
                );
            }
            Ok(())
        }
        AdminCommand::List => {
            for admin in db.admins().await? {
                println!(
                    "#{:<3} {:<24} {}{}{}{}",
                    admin.id,
                    admin.username,
                    if admin.is_sudo { "sudo" } else { "role" },
                    if admin.enabled { "" } else { "  (disabled)" },
                    if admin.pending() {
                        "  (no password yet)"
                    } else {
                        ""
                    },
                    if admin.totp_secret.is_some() {
                        "  2FA"
                    } else {
                        ""
                    }
                );
            }
            Ok(())
        }
        AdminCommand::ResetPassword {
            username,
            password,
            disable_2fa,
        } => {
            let admin = db
                .admin_by_username(username)
                .await?
                .with_context(|| format!("there is no admin {username:?}"))?;
            let (password, generated) = match password {
                Some(password) => (password.clone(), false),
                None => (hex::encode(rand::random::<[u8; 9]>()), true),
            };
            auth::password::check_strength(&password).map_err(|e| anyhow::anyhow!(e.message))?;
            let hash = auth::password::hash(&password)
                .await
                .map_err(|e| anyhow::anyhow!(e.message))?;
            db.set_admin_password(admin.id, &hash).await?;
            if *disable_2fa {
                db.set_admin_totp(admin.id, None).await?;
            }
            println!("the password of {username:?} was reset; their sessions have ended");
            if generated {
                println!("new password: {password}");
            }
            if *disable_2fa {
                println!("two-factor sign-in is off");
            }
            Ok(())
        }
    }
}

async fn setting(
    db: &z_db::Database,
    config: &PanelConfig,
    stored: &panel_settings::Stored,
    command: &SettingCommand,
) -> anyhow::Result<()> {
    match command {
        SettingCommand::Show => {
            let scheme = if config.locked.certificate || stored.certificate_id.is_some() {
                "https"
            } else {
                "http"
            };
            let from = |locked: bool, stored: bool| match (locked, stored) {
                (true, _) => "a flag or ZUI_* variable",
                (false, true) => "stored",
                (false, false) => "default",
            };
            println!(
                "listen       {}  ({})",
                config.listen,
                from(config.locked.listen, stored.listen.is_some())
            );
            println!(
                "path         {}  ({})",
                config.base_path,
                from(config.locked.base_path, stored.base_path.is_some())
            );
            let certificate = match (&config.cert, stored.certificate_id) {
                (Some(cert), _) => format!("files {}  (a flag or ZUI_* variable)", cert.display()),
                (None, Some(id)) => match db.certificate(id).await? {
                    Some(c) => format!("#{id} {}  (stored)", c.name),
                    None => format!("#{id}, which no longer exists  (stored)"),
                },
                (None, None) => "none: plain HTTP  (default)".into(),
            };
            println!("certificate  {certificate}");
            println!(
                "address      {scheme}://{}{}",
                config.listen,
                config.under_base("/")
            );
            Ok(())
        }
        SettingCommand::Set {
            listen,
            port,
            path,
            certificate,
        } => {
            let mut next = stored.clone();
            if let Some(listen) = listen {
                next.listen = Some(*listen);
            }
            if let Some(port) = port {
                anyhow::ensure!(*port > 0, "port 0 is not a port to listen on");
                let mut address = config.listen;
                address.set_port(*port);
                next.listen = Some(address);
            }
            if let Some(path) = path {
                let path = panel_settings::normalize_base_path(path).map_err(anyhow::Error::msg)?;
                next.base_path = Some(path);
            }
            if let Some(certificate) = certificate {
                next.certificate_id = match certificate.trim() {
                    "none" | "" => None,
                    id => {
                        let id: i64 = id
                            .trim_start_matches('#')
                            .parse()
                            .with_context(|| format!("{id:?} is not a certificate id or `none`"))?;
                        let found = db
                            .certificate(id)
                            .await?
                            .with_context(|| format!("there is no certificate {id}"))?;
                        tls::certified_key(found.cert_pem.as_bytes(), found.key_pem.as_bytes())
                            .with_context(|| format!("certificate {id} cannot be served"))?;
                        Some(id)
                    }
                };
            }
            panel_settings::save(db, &next).await?;
            println!("stored; they apply when the panel next starts");
            Ok(())
        }
        SettingCommand::Reset => {
            panel_settings::save(db, &panel_settings::Stored::default()).await?;
            println!("the stored settings are gone; the flags and defaults apply again");
            Ok(())
        }
    }
}

fn days_left(not_after: i64) -> i64 {
    (not_after - z_db::now()).div_euclid(86_400)
}

async fn cert(
    config: &PanelConfig,
    db: &z_db::Database,
    command: &CertCommand,
) -> anyhow::Result<()> {
    match command {
        CertCommand::List => {
            let all = db.certificates().await?;
            if all.is_empty() {
                println!("no certificates");
            }
            for c in all {
                println!(
                    "#{:<3} {:<28} {:<10} {:>5} days  {}{}",
                    c.id,
                    c.name,
                    c.source,
                    days_left(c.not_after),
                    c.domains,
                    if c.auto_renew {
                        "  (renews itself)"
                    } else {
                        ""
                    }
                );
            }
            Ok(())
        }
        CertCommand::Issue {
            names,
            method,
            name,
            no_auto_renew,
            ca,
            email,
            cloudflare_token,
            porcelain,
        } => {
            let mut settings = certs::settings::load(db).await;
            if let Some(ca) = ca {
                settings.ca = match ca.as_str() {
                    "letsencrypt" => certs::settings::Ca::Letsencrypt,
                    "letsencrypt_staging" | "staging" => certs::settings::Ca::LetsencryptStaging,
                    "zerossl" => certs::settings::Ca::Zerossl,
                    other => anyhow::bail!("{other:?} is not an authority: letsencrypt, letsencrypt_staging or zerossl"),
                };
            }
            if let Some(email) = email {
                settings.email = email.clone();
            }
            if let Some(token) = cloudflare_token {
                settings.cloudflare_token = token.clone();
            }
            certs::settings::save(db, &settings).await?;
            let state = offline(config, db).await;
            let issued = certs::issue(
                &state,
                certs::IssueRequest {
                    name: name.clone().unwrap_or_default(),
                    names: names.clone(),
                    method: acme::Method::from_label(method),
                    auto_renew: !no_auto_renew,
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
            if *porcelain {
                println!("{}", issued.id);
            } else {
                println!(
                    "got certificate #{} {:?} for {} ({} days; {})",
                    issued.id,
                    issued.name,
                    issued.domains,
                    days_left(issued.not_after),
                    issued.method
                );
            }
            Ok(())
        }
        CertCommand::Renew { id } => {
            let state = offline(config, db).await;
            let renewed = certs::renew(&state, *id)
                .await
                .map_err(|e| anyhow::anyhow!(e.message))?;
            println!(
                "renewed #{} {:?}: {} days left",
                renewed.id,
                renewed.name,
                days_left(renewed.not_after)
            );
            println!("a running panel takes it when it restarts");
            Ok(())
        }
        CertCommand::SelfSigned {
            names,
            name,
            porcelain,
        } => {
            let parsed = acme::parse_names(names).map_err(|e| anyhow::anyhow!(e.message))?;
            let (cert_pem, key_pem) =
                certs::self_signed(&parsed).map_err(|e| anyhow::anyhow!(e.message))?;
            let seen =
                certs::inspect(&cert_pem, &key_pem).map_err(|e| anyhow::anyhow!(e.message))?;
            let made = db
                .add_certificate(z_db::NewCertificate {
                    name: name.clone().unwrap_or_else(|| parsed[0].text()),
                    domains: seen.domains.join(","),
                    cert_pem: cert_pem.trim().into(),
                    key_pem: key_pem.trim().into(),
                    source: "self-signed".into(),
                    not_before: seen.not_before,
                    not_after: seen.not_after,
                    ..Default::default()
                })
                .await?;
            if *porcelain {
                println!("{}", made.id);
            } else {
                println!(
                    "made certificate #{} {:?} for {}",
                    made.id, made.name, made.domains
                );
            }
            Ok(())
        }
    }
}

async fn update_command(db: &z_db::Database, command: &UpdateCommand) -> anyhow::Result<()> {
    let settings = update::load(db).await;
    let exe = update::own_executable();
    match command {
        UpdateCommand::Check => {
            let info = update::check(&settings).await.map_err(anyhow::Error::msg)?;
            if info.available {
                println!(
                    "version {} is out (this is {}): {}",
                    info.version, info.current, info.url
                );
            } else {
                println!("{} is the newest version", info.current);
            }
            Ok(())
        }
        UpdateCommand::Apply { yes } => {
            anyhow::ensure!(
                !update::in_container(),
                "this runs in a container: pull the new image instead"
            );
            let info = update::check(&settings).await.map_err(anyhow::Error::msg)?;
            if !info.available {
                println!("{} is the newest version", info.current);
                return Ok(());
            }
            println!(
                "version {} is out (this is {}): {}",
                info.version, info.current, info.url
            );
            if !yes {
                println!("run it again with --yes to put it in place");
                return Ok(());
            }
            let program = update::download_verified(&settings, &info, update::SIGNING_KEY)
                .await
                .map_err(anyhow::Error::msg)?;
            update::install(&program, &exe, &info.version)
                .await
                .map_err(anyhow::Error::msg)?;
            println!(
                "version {} is in place at {}; restart the panel to run it",
                info.version,
                exe.display()
            );
            Ok(())
        }
        UpdateCommand::Rollback => {
            update::rollback(&exe).map_err(anyhow::Error::msg)?;
            println!(
                "the earlier program is back at {}; restart the panel to run it",
                exe.display()
            );
            Ok(())
        }
    }
}

async fn backup_now(
    config: &PanelConfig,
    db: &z_db::Database,
    out: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let state = offline(config, db).await;
    let done = backup::run(&state, None)
        .await
        .map_err(anyhow::Error::msg)?;
    let path = backup::dir(&state).join(&done.file);
    println!("backup {} ({} bytes)", path.display(), done.bytes);
    if let Some(out) = out {
        std::fs::copy(&path, out).with_context(|| format!("copying it to {}", out.display()))?;
        println!("copied to {}", out.display());
    }
    Ok(())
}

async fn restore(
    config: &PanelConfig,
    db: &z_db::Database,
    file: &std::path::Path,
    passphrase: Option<&str>,
) -> anyhow::Result<()> {
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let state = offline(config, db).await;
    let done = backup::restore(&state, &bytes, passphrase.unwrap_or(""))
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    println!("restored; what was there before is kept as {}", done.safety);
    println!("start the panel again to use it (everyone has to sign in again)");
    Ok(())
}

/// Show what another panel's database would bring, and bring it with `--apply`.
async fn migrate(
    config: &PanelConfig,
    db: &z_db::Database,
    args: &MigrateArgs,
) -> anyhow::Result<()> {
    use crate::migrate::apply::{Conflict, Options};
    use crate::migrate::model::{Kind, LinkSupport};

    let kind = match args.kind.as_deref() {
        Some(text) => Some(Kind::parse(text).context("--kind is 3x-ui, pasarguard or marzban")?),
        None => None,
    };
    let xray_config: Option<serde_json::Value> = match &args.xray_config {
        Some(path) => {
            let bytes =
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            Some(
                serde_json::from_slice(&bytes)
                    .with_context(|| format!("{} is not JSON", path.display()))?,
            )
        }
        None => None,
    };
    let plan = crate::migrate::read(&args.from, kind, xray_config.as_ref())
        .await
        .map_err(|error| match error {
            crate::migrate::ReadError::Open(e) => anyhow::anyhow!("cannot open {}: {e}", args.from),
            crate::migrate::ReadError::Unknown => anyhow::anyhow!(
                "{} does not look like the database of 3X-UI, PasarGuard or Marzban (say which with --kind)",
                args.from
            ),
            crate::migrate::ReadError::Read(e) => anyhow::anyhow!("reading {}: {e}", args.from),
        })?;
    let summary = plan.summary();
    println!("a {} database", plan.kind.label());
    println!(
        "{} inbounds, {} users, {} groups, {} templates, {} hosts, {} hours of usage",
        summary.inbounds,
        summary.users,
        summary.groups,
        summary.templates,
        summary.hosts,
        summary.usage_hours
    );
    println!(
        "old subscription links: {}",
        match summary.links {
            LinkSupport::Tokens => "each user keeps its token, so the same address works",
            LinkSupport::Signed => "signed links keep working for the users that come over",
        }
    );
    for warning in &plan.warnings {
        println!("  [{}] {}: {}", warning.scope, warning.name, warning.detail);
    }
    if !args.apply {
        println!("nothing was changed; run again with --apply to bring it over");
        return Ok(());
    }
    let options = Options {
        inbounds: !args.no_inbounds,
        users: !args.no_users,
        hosts: !args.no_hosts,
        templates: !args.no_templates,
        admins: !args.no_admins,
        usage: !args.no_usage,
        settings: !args.no_settings,
        enable_inbounds: args.enable_inbounds,
        on_conflict: if args.rename {
            Conflict::Rename
        } else {
            Conflict::Skip
        },
        sync_core: false,
        refresh_usernames: Default::default(),
    };
    let state = offline(config, db).await;
    let report = crate::migrate::apply::apply(&state, plan, &options)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    let n = &report.imported;
    println!(
        "brought over {} inbounds, {} users, {} groups, {} templates, {} hosts, {} certificates",
        n.inbounds, n.users, n.groups, n.templates, n.hosts, n.certificates
    );
    println!(
        "what was here before is kept as {} (restoring it undoes this)",
        report.backup
    );
    println!("start (or restart) the panel to use it");
    Ok(())
}

fn confirm(question: &str, yes: bool) -> anyhow::Result<()> {
    use std::io::{BufRead, IsTerminal, Write};
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("this asks a question, and nobody is there to answer: give --yes");
    }
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        anyhow::bail!("cancelled; nothing was changed by this step")
    }
}

fn show_warnings(warnings: &[crate::migrate::model::Warning]) {
    for warning in warnings {
        println!("  [{}] {}: {}", warning.scope, warning.name, warning.detail);
    }
}

/// The panel to take over: the only one found, or the one named.
async fn chosen_panel(
    root: &std::path::Path,
    wanted: Option<&str>,
) -> anyhow::Result<crate::takeover::detect::Detected> {
    use crate::migrate::model::Kind;
    let mut found = crate::takeover::detect::detect(root);
    if let Some(text) = wanted {
        let kind = Kind::parse(text).context("--panel is 3x-ui, pasarguard or marzban")?;
        found.retain(|d| d.kind == kind);
    }
    let mut detected = match found.len() {
        0 => anyhow::bail!(
            "no 3X-UI, PasarGuard or Marzban installed by their own installers was found here"
        ),
        1 => found.remove(0),
        _ => anyhow::bail!(
            "more than one panel is here ({}): say which with --panel",
            found
                .iter()
                .map(|d| d.kind.label())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    crate::takeover::detect::inspect(root, &mut detected).await;
    Ok(detected)
}

async fn takeover(
    config: &PanelConfig,
    db: &z_db::Database,
    command: &TakeoverCommand,
) -> anyhow::Result<()> {
    use crate::takeover::run::{self, Context};
    use crate::takeover::state::{self, Phase};

    let root = crate::takeover::root();
    let host = crate::takeover::host::from_env();
    let app = offline(config, db).await;
    let say = |text: &str| println!("{text}");
    let cx = Context {
        root: &root,
        host: &*host,
        state: &app,
        say: &say,
    };
    let failed = |error: run::Failure| anyhow::anyhow!("{} ({})", error.message, error.code);

    // The part that changes nothing for anyone.
    async fn prepare(
        cx: &Context<'_>,
        root: &std::path::Path,
        panel: Option<&str>,
    ) -> anyhow::Result<()> {
        let detected = chosen_panel(root, panel).await?;
        println!("found {}", detected.kind.label());
        let prepared = run::prepare(cx, detected)
            .await
            .map_err(|e| anyhow::anyhow!("{} ({})", e.message, e.code))?;
        let n = &prepared.counts;
        println!(
            "brought over {} inbounds ({} to be switched on), {} users, {} groups, {} templates, {} hosts",
            n.inbounds,
            prepared.enable.len(),
            n.users,
            n.groups,
            n.templates,
            n.hosts
        );
        show_warnings(&prepared.warnings);
        Ok(())
    }

    let cutover = |yes: bool| {
        let (cx, failed) = (&cx, &failed);
        async move {
            confirm(
                "The old panel is stopped now and this one takes over: its users are cut off for a few seconds, and their apps reconnect by themselves. Go on?",
                yes,
            )?;
            let switched = run::cutover(cx).await.map_err(failed)?;
            show_warnings(&switched.warnings);
            println!(
                "done: {} users brought up to date, {} new, {} dropped",
                switched.refreshed, switched.added_users, switched.dropped_users
            );
            println!(
                "the old panel's files stay where they are, stopped, until {} (a week);",
                crate::notify::events::date(switched.expires_at)
            );
            println!("`z-ui takeover rollback` puts the old panel back (with what its users used meanwhile),");
            println!("`z-ui takeover delete-old` deletes it now");
            anyhow::Ok(())
        }
    };

    match command {
        TakeoverCommand::Detect => {
            let found = crate::takeover::detect::detect(&root);
            if found.is_empty() {
                println!("no panel that can be replaced was found");
            }
            for mut detected in found {
                crate::takeover::detect::inspect(&root, &mut detected).await;
                println!("{}", detected.kind.label());
                if let Some(database) = &detected.database {
                    println!("  database  {}", database.display());
                }
                if let Some(port) = detected.panel_port {
                    println!("  panel     port {port}");
                }
                if let Some(sub) = &detected.sub {
                    println!("  subscriptions  {}", sub.text());
                }
                for path in &detected.remove {
                    println!("  files     {}", path.display());
                }
                if let Some(why) = &detected.unsupported {
                    println!("  cannot be replaced: {why}");
                }
            }
            Ok(())
        }
        TakeoverCommand::Prepare { panel } => {
            prepare(&cx, &root, panel.as_deref()).await?;
            println!("the old panel still serves; `z-ui takeover cutover` makes the switch");
            Ok(())
        }
        TakeoverCommand::Cutover { yes } => cutover(*yes).await,
        TakeoverCommand::Run { panel, yes } => {
            prepare(&cx, &root, panel.as_deref()).await?;
            cutover(*yes).await
        }
        TakeoverCommand::Status => {
            match state::load(&config.data_dir) {
                None => println!("no takeover"),
                Some(st) => {
                    println!("{} · {:?}", st.detected.kind.label(), st.phase);
                    if let (Phase::Switched, Some(at)) = (st.phase, st.expires_at) {
                        let left = (at - z_db::now()).max(0);
                        println!(
                            "the old panel's files are deleted in about {} hours; until then `z-ui takeover rollback` goes back",
                            left / 3600
                        );
                    }
                }
            }
            Ok(())
        }
        TakeoverCommand::Rollback { yes } => {
            confirm(
                "This panel is stopped and the old one started again, with what its users used here added to its counters. Go on?",
                *yes,
            )?;
            let back = run::rollback(&cx).await.map_err(failed)?;
            show_warnings(&back.warnings);
            println!(
                "the old panel is back; {} bytes of usage of {} users were given back to it; this panel is stopped and will not start by itself",
                back.given_back.0, back.given_back.1
            );
            Ok(())
        }
        TakeoverCommand::DeleteOld { yes } => {
            confirm(
                "The old panel's files are deleted, and there is no way back after this. Go on?",
                *yes,
            )?;
            let warnings = run::delete_old(&cx).await.map_err(failed)?;
            show_warnings(&warnings);
            println!("the old panel's files are gone");
            Ok(())
        }
    }
}

/// Ask the panel on this machine whether it answers: connect to where it
/// listens (over TLS if it serves HTTPS) and read the status of its open
/// `auth/state`.
async fn health(config: &PanelConfig, stored: &Stored) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut address = config.listen;
    if address.ip().is_unspecified() {
        address.set_ip(if address.is_ipv4() {
            std::net::Ipv4Addr::LOCALHOST.into()
        } else {
            std::net::Ipv6Addr::LOCALHOST.into()
        });
    }
    let tls =
        config.cert.is_some() || (!config.locked.certificate && stored.certificate_id.is_some());
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        config.under_base("/api/auth/state")
    );
    let tcp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::TcpStream::connect(address),
    )
    .await
    .context("connecting timed out")?
    .with_context(|| format!("connecting to {address}"))?;
    let mut head = [0u8; 12];
    if tls {
        let connector = tokio_rustls::TlsConnector::from(tls::insecure_client(&[b"http/1.1"]));
        let name = rustls_pki_types::ServerName::try_from("localhost")?;
        let mut stream = connector
            .connect(name, tcp)
            .await
            .context("the TLS handshake")?;
        stream.write_all(request.as_bytes()).await?;
        stream.read_exact(&mut head).await?;
    } else {
        let mut stream = tcp;
        stream.write_all(request.as_bytes()).await?;
        stream.read_exact(&mut head).await?;
    }
    let line = String::from_utf8_lossy(&head);
    anyhow::ensure!(line.ends_with(" 200"), "the panel answered {line:?}");
    println!("ok");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn world() -> (PanelConfig, z_db::Database, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "z-cli-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config = PanelConfig::from_args_iter(["z-ui", "--data-dir", dir.to_str().unwrap()]);
        let db = z_db::Database::open(&config.database_url()).await.unwrap();
        (config, db, dir)
    }

    async fn run_args(
        config: &PanelConfig,
        db: &z_db::Database,
        args: &[&str],
    ) -> anyhow::Result<()> {
        let mut argv = vec!["z-ui", "--data-dir", config.data_dir.to_str().unwrap()];
        argv.extend_from_slice(args);
        let parsed = PanelConfig::from_args_iter(argv);
        let stored = panel_settings::load(db).await;
        run(
            &parsed,
            db,
            &stored,
            parsed.command.as_ref().expect("a command"),
        )
        .await
    }

    #[tokio::test]
    async fn an_admin_is_made_from_the_server_and_a_duplicate_or_a_weak_password_is_refused() {
        let (config, db, dir) = world().await;
        run_args(
            &config,
            &db,
            &["admin", "create", "root", "--password", "a-fine-password-1"],
        )
        .await
        .unwrap();
        let admins = db.admins().await.unwrap();
        assert_eq!(admins.len(), 1);
        assert!(admins[0].is_sudo && admins[0].username == "root");
        assert!(auth::password::verify("a-fine-password-1", &admins[0].password_hash).await);

        assert!(
            run_args(&config, &db, &["admin", "create", "root"])
                .await
                .is_err(),
            "taken"
        );
        assert!(run_args(
            &config,
            &db,
            &["admin", "create", "other", "--password", "short"]
        )
        .await
        .is_err());
        assert!(run_args(&config, &db, &["admin", "create", "a b"])
            .await
            .is_err());
        run_args(&config, &db, &["admin", "create", "helper", "--not-sudo"])
            .await
            .unwrap();
        assert!(
            !db.admin_by_username("helper")
                .await
                .unwrap()
                .unwrap()
                .is_sudo
        );
        run_args(&config, &db, &["admin", "list"]).await.unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn the_stored_address_is_set_shown_and_forgotten_from_the_server() {
        let (config, db, dir) = world().await;
        run_args(&config, &db, &["cert", "self-signed", "panel.test"])
            .await
            .unwrap();
        run_args(
            &config,
            &db,
            &[
                "setting",
                "set",
                "--port",
                "24680",
                "--path",
                "secret/",
                "--certificate",
                "1",
            ],
        )
        .await
        .unwrap();
        let stored = panel_settings::load(&db).await;
        assert_eq!(stored.listen.unwrap().port(), 24680);
        assert_eq!(stored.base_path.as_deref(), Some("/secret"));
        assert_eq!(stored.certificate_id, Some(1));
        run_args(&config, &db, &["setting", "show"]).await.unwrap();

        assert!(
            run_args(&config, &db, &["setting", "set", "--certificate", "99"])
                .await
                .is_err(),
            "no such certificate"
        );
        assert!(run_args(&config, &db, &["setting", "set", "--path", "a b"])
            .await
            .is_err());
        assert!(run_args(&config, &db, &["setting", "set", "--port", "0"])
            .await
            .is_err());
        run_args(&config, &db, &["setting", "set", "--certificate", "none"])
            .await
            .unwrap();
        assert_eq!(panel_settings::load(&db).await.certificate_id, None);
        run_args(&config, &db, &["setting", "reset"]).await.unwrap();
        assert_eq!(panel_settings::load(&db).await, Stored::default());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_backup_is_made_and_put_back_from_the_server() {
        let (config, db, dir) = world().await;
        run_args(
            &config,
            &db,
            &["admin", "create", "root", "--password", "a-fine-password-1"],
        )
        .await
        .unwrap();
        let out = dir.join("copy.zbk");
        run_args(&config, &db, &["backup", "--out", out.to_str().unwrap()])
            .await
            .unwrap();
        assert!(out.metadata().unwrap().len() > 100);
        run_args(
            &config,
            &db,
            &[
                "admin",
                "create",
                "after",
                "--password",
                "a-fine-password-2",
            ],
        )
        .await
        .unwrap();
        run_args(&config, &db, &["restore", out.to_str().unwrap()])
            .await
            .unwrap();
        assert!(
            db.admin_by_username("after").await.unwrap().is_none(),
            "what came after is gone"
        );
        assert!(db.admin_by_username("root").await.unwrap().is_some());
        assert!(run_args(
            &config,
            &db,
            &["restore", dir.join("missing.zbk").to_str().unwrap()]
        )
        .await
        .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn another_panels_database_is_shown_first_and_brought_over_only_when_asked() {
        use crate::migrate::tests::{TempDb, CLASSIC};
        let (config, db, dir) = world().await;
        let source = TempDb::new(CLASSIC).await;
        run_args(&config, &db, &["migrate", source.path()])
            .await
            .unwrap();
        assert!(
            db.all_users().await.unwrap().is_empty(),
            "no --apply, no change"
        );

        run_args(
            &config,
            &db,
            &["migrate", source.path(), "--apply", "--no-usage"],
        )
        .await
        .unwrap();
        assert_eq!(db.all_users().await.unwrap().len(), 10);
        assert_eq!(db.inbounds().await.unwrap().len(), 4);
        assert!(
            db.user_by_token("ali0sub1id").await.unwrap().is_some(),
            "the old link's token is the user's"
        );
        assert!(std::fs::read_dir(dir.join("backups"))
            .unwrap()
            .flatten()
            .any(|f| f.file_name().to_string_lossy().contains("-before-import")));

        // A kind that does not exist, a file that is no database, a missing file.
        assert!(run_args(
            &config,
            &db,
            &["migrate", source.path(), "--kind", "wordpress"]
        )
        .await
        .is_err());
        let junk = dir.join("junk.db");
        std::fs::write(&junk, b"not a database at all, only some text").unwrap();
        assert!(run_args(&config, &db, &["migrate", junk.to_str().unwrap()])
            .await
            .is_err());
        assert!(run_args(
            &config,
            &db,
            &["migrate", dir.join("missing.db").to_str().unwrap()]
        )
        .await
        .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_link_for_an_admin_without_a_password_is_made_from_the_server() {
        let (config, db, dir) = world().await;
        db.create_admin("ali", z_db::NO_PASSWORD, true)
            .await
            .unwrap();
        db.create_admin("bob", "$argon2id$not-pending", true)
            .await
            .unwrap();
        assert!(
            run_args(&config, &db, &["admin", "invite"]).await.is_err(),
            "who for?"
        );
        assert!(run_args(&config, &db, &["admin", "invite", "nobody"])
            .await
            .is_err());
        run_args(
            &config,
            &db,
            &[
                "admin",
                "invite",
                "--pending",
                "--host",
                "panel.example.com",
            ],
        )
        .await
        .unwrap();
        let count = |name: &'static str| {
            let db = db.clone();
            async move {
                let admin = db.admin_by_username(name).await.unwrap().unwrap();
                sqlx::query("SELECT COUNT(*) FROM admin_invites WHERE admin_id = $1")
                    .bind(admin.id)
                    .fetch_one(db.pool())
                    .await
                    .map(|row| sqlx::Row::get::<i64, _>(&row, 0))
                    .unwrap()
            }
        };
        assert_eq!(
            (count("ali").await, count("bob").await),
            (1, 0),
            "only who has none yet"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_panel_on_this_machine_is_replaced_and_given_back_from_the_server() {
        use crate::migrate::tests::{TempDb, CLASSIC};
        let (config, db, dir) = world().await;
        let machine = dir.join("machine");
        for (path, bytes) in [
            ("usr/local/x-ui/x-ui", b"binary".to_vec()),
            ("etc/x-ui/x-ui.db", TempDb::new(CLASSIC).await.bytes()),
            ("etc/systemd/system/x-ui.service", b"[Service]\n".to_vec()),
        ] {
            let target = machine.join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, bytes).unwrap();
        }
        // The machine is a directory, and no service is touched.
        std::env::set_var("ZUI_PREFIX", &machine);
        std::env::set_var("ZUI_NO_SYSTEMD", "1");
        {
            run_args(&config, &db, &["takeover", "detect"])
                .await
                .unwrap();
            run_args(&config, &db, &["takeover", "prepare"])
                .await
                .unwrap();
            assert_eq!(db.all_users().await.unwrap().len(), 10);
            assert!(
                machine.join("usr/local/x-ui/x-ui").exists(),
                "prepared, not switched"
            );
            // Without --yes and without anybody to ask, nothing happens.
            assert!(run_args(&config, &db, &["takeover", "cutover"])
                .await
                .is_err());
            assert!(machine.join("usr/local/x-ui/x-ui").exists());
            run_args(&config, &db, &["takeover", "cutover", "--yes"])
                .await
                .unwrap();
            assert!(
                machine.join("usr/local/x-ui/x-ui").exists(),
                "the old files stay for a week"
            );
            assert!(db.inbounds().await.unwrap().iter().all(|i| i.enabled));
            run_args(&config, &db, &["takeover", "status"])
                .await
                .unwrap();
            run_args(&config, &db, &["takeover", "rollback", "--yes"])
                .await
                .unwrap();
            assert_eq!(
                std::fs::read(machine.join("usr/local/x-ui/x-ui")).unwrap(),
                b"binary"
            );
            // Back, then there is nothing to delete or to go back from.
            assert!(run_args(&config, &db, &["takeover", "delete-old", "--yes"])
                .await
                .is_err());
        }
        std::env::remove_var("ZUI_PREFIX");
        std::env::remove_var("ZUI_NO_SYSTEMD");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A server that answers every request with `status`.
    async fn answering(status: &'static str) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buffer = [0u8; 1024];
                    let _ = stream.read(&mut buffer).await;
                    let _ = stream
                        .write_all(
                            format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n").as_bytes(),
                        )
                        .await;
                });
            }
        });
        address
    }

    #[tokio::test]
    async fn health_is_ok_for_a_panel_that_answers_200_and_not_for_one_that_does_not() {
        let (_, _, dir) = world().await;
        let check = |address: std::net::SocketAddr, base: &str| {
            let listen = address.to_string();
            let config = PanelConfig::from_args_iter([
                "z-ui",
                "--data-dir",
                dir.to_str().unwrap(),
                "--listen",
                &listen,
                "--base-path",
                base,
            ])
            .resolved()
            .unwrap();
            async move { health(&config, &Stored::default()).await }
        };
        assert!(check(answering("200 OK").await, "/zpanel").await.is_ok());
        let error = check(answering("503 Service Unavailable").await, "/")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("503"), "{error}");
        // Nothing listening at all.
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        assert!(check(closed, "/").await.is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn health_speaks_tls_to_a_panel_that_serves_https_whatever_its_certificate() {
        let (_, _, dir) = world().await;
        let (cert, key) = tls::testing::self_signed(&["panel.test"], 30);
        let resolver = tls::Resolver::from_pem(&cert, &key).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app =
            axum::Router::new().route("/api/auth/state", axum::routing::get(|| async { "{}" }));
        tokio::spawn(tls::serve(
            listener,
            tls::acceptor(resolver).unwrap(),
            app,
            std::future::pending(),
        ));
        let listen = address.to_string();
        let config = PanelConfig::from_args_iter([
            "z-ui",
            "--data-dir",
            dir.to_str().unwrap(),
            "--listen",
            &listen,
        ])
        .resolved()
        .unwrap();
        let stored = Stored {
            certificate_id: Some(1),
            ..Stored::default()
        };
        assert!(health(&config, &stored).await.is_ok(), "https");
        assert!(
            health(&config, &Stored::default()).await.is_err(),
            "plain HTTP to an HTTPS port is not an answer"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
