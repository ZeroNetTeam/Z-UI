//! The page a browser gets at a subscription's address: the user's usage and
//! time, the subscription link with a QR code and buttons that open it in
//! the common apps, and every config with its own QR code. In Persian when
//! the browser prefers it, English otherwise. An admin can replace it with
//! their own HTML ([`super::Settings::template`]).
//!
//! Everything shown is escaped; the page carries no script of its own beyond
//! copying text, and loads nothing from elsewhere.

use serde_json::json;

use super::endpoint::Endpoint;
use super::{date, formats, Settings};

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A QR code of `text`, as SVG.
pub fn qr(text: &str) -> String {
    qrcode::QrCode::new(text.as_bytes())
        .map(|code| {
            code.render::<qrcode::render::svg::Color>()
                .min_dimensions(200, 200)
                .quiet_zone(true)
                .build()
        })
        .unwrap_or_default()
}

fn bytes(n: i64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n.max(0) as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", units[unit])
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

struct Words {
    dir: &'static str,
    lang: &'static str,
    used: &'static str,
    unlimited: &'static str,
    expires: &'static str,
    never: &'static str,
    days_left: &'static str,
    subscription: &'static str,
    copy: &'static str,
    copied: &'static str,
    open_in: &'static str,
    configs: &'static str,
    none: &'static str,
    qr: &'static str,
    support: &'static str,
    status: fn(&str) -> &'static str,
}

const FA: Words = Words {
    dir: "rtl",
    lang: "fa",
    used: "مصرف",
    unlimited: "نامحدود",
    expires: "انقضا",
    never: "بدون انقضا",
    days_left: "روز مانده",
    subscription: "لینک اشتراک",
    copy: "کپی",
    copied: "کپی شد",
    open_in: "باز کردن در",
    configs: "کانفیگ‌ها",
    none: "کانفیگی برای این اشتراک نیست.",
    qr: "QR",
    support: "پشتیبانی",
    status: |status| match status {
        "active" => "فعال",
        "on_hold" => "در انتظار اولین اتصال",
        "disabled" => "غیرفعال",
        "limited" => "حجم تمام شده",
        "expired" => "منقضی شده",
        _ => "",
    },
};

const EN: Words = Words {
    dir: "ltr",
    lang: "en",
    used: "Used",
    unlimited: "unlimited",
    expires: "Expires",
    never: "Never expires",
    days_left: "days left",
    subscription: "Subscription link",
    copy: "Copy",
    copied: "Copied",
    open_in: "Open in",
    configs: "Configs",
    none: "This subscription has no configs.",
    qr: "QR",
    support: "Support",
    status: |status| match status {
        "active" => "Active",
        "on_hold" => "Waiting for the first connection",
        "disabled" => "Disabled",
        "limited" => "Out of data",
        "expired" => "Expired",
        _ => "",
    },
};

fn percent(text: &str) -> String {
    percent_encoding::utf8_percent_encode(text, percent_encoding::NON_ALPHANUMERIC).to_string()
}

pub fn render(
    settings: &Settings,
    user: &z_db::User,
    endpoints: &[Endpoint],
    sub_url: &str,
    accept_language: &str,
) -> String {
    let links: Vec<(String, String)> = endpoints
        .iter()
        .map(|e| (e.remark.clone(), formats::link(e)))
        .collect();
    if !settings.template.trim().is_empty() {
        let data = json!({
            "title": settings.title,
            "username": user.username,
            "status": user.status,
            "used_traffic": user.used_traffic,
            "data_limit": user.data_limit,
            "expire_at": user.expire_at,
            "subscription_url": sub_url,
            "support_url": settings.support_url,
            "links": links.iter().map(|(remark, link)| json!({"remark": remark, "link": link})).collect::<Vec<_>>(),
        });
        // Safe inside a <script> element: nothing in it can close one.
        let data = data.to_string().replace('<', "\\u003c");
        return settings.template.replace("{{data}}", &data);
    }

    let w = if accept_language
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("fa")
    {
        &FA
    } else {
        &EN
    };
    let now = z_db::now();
    let percent_used = if user.data_limit > 0 {
        (user.used_traffic as f64 / user.data_limit as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let usage = if user.data_limit > 0 {
        format!("{} / {}", bytes(user.used_traffic), bytes(user.data_limit))
    } else {
        format!("{} / {}", bytes(user.used_traffic), w.unlimited)
    };
    let expiry = match user.expire_at {
        Some(at) => format!(
            "{} {} · {} {}",
            w.expires,
            date(at),
            ((at - now).max(0) / 86_400),
            w.days_left
        ),
        None => w.never.to_string(),
    };
    let title = escape(&settings.title);
    let url = escape(sub_url);
    let encoded = percent(sub_url);
    let apps = [
        (
            "Hiddify",
            format!("hiddify://import/{sub_url}#{}", percent(&settings.title)),
        ),
        (
            "sing-box",
            format!(
                "sing-box://import-remote-profile?url={encoded}#{}",
                percent(&settings.title)
            ),
        ),
        (
            "Clash",
            format!(
                "clash://install-config?url={encoded}&name={}",
                percent(&settings.title)
            ),
        ),
        ("Streisand", format!("streisand://import/{sub_url}")),
        ("Happ", format!("happ://add/{sub_url}")),
    ];
    let buttons: String = apps
        .iter()
        .map(|(name, href)| format!("<a class=\"app\" href=\"{}\">{name}</a>", escape(href)))
        .collect();
    let configs: String = if links.is_empty() {
        format!("<p class=\"muted\">{}</p>", w.none)
    } else {
        links
            .iter()
            .map(|(remark, link)| {
                format!(
                    "<li><div class=\"row\"><span class=\"name\">{}</span><button data-copy=\"{}\">{}</button></div>\
                     <details><summary>{}</summary><div class=\"qr\">{}</div></details></li>",
                    escape(remark),
                    escape(link),
                    w.copy,
                    w.qr,
                    qr(link)
                )
            })
            .collect()
    };
    let support = if settings.support_url.is_empty() {
        String::new()
    } else {
        format!(
            "<a class=\"support\" href=\"{}\">{}</a>",
            escape(&settings.support_url),
            w.support
        )
    };
    format!(
        r#"<!doctype html>
<html lang="{lang}" dir="{dir}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<meta name="robots" content="noindex">
<title>{title}</title>
<style>
:root {{ color-scheme: dark; --bg: #0f0e0c; --card: #191816; --line: #2b2925; --text: #f2efe8; --muted: #a39d92; --gold: #e2b54d; --ok: #4fc483; --bad: #ef6b5b; }}
* {{ box-sizing: border-box; }}
body {{ margin: 0; background: var(--bg); color: var(--text); font: 15px/1.6 system-ui, "Vazirmatn", "Segoe UI", sans-serif; }}
main {{ max-width: 640px; margin: 0 auto; padding: 24px 16px 48px; }}
h1 {{ font-size: 20px; margin: 0 0 4px; color: var(--gold); }}
.card {{ background: var(--card); border: 1px solid var(--line); border-radius: 14px; padding: 18px; margin-top: 16px; }}
.muted {{ color: var(--muted); }}
.status {{ display: inline-block; padding: 2px 10px; border-radius: 99px; font-size: 13px; background: #2a2620; color: var(--gold); }}
.status.active {{ color: var(--ok); background: #18291f; }}
.status.limited, .status.expired, .status.disabled {{ color: var(--bad); background: #2c1a17; }}
.bar {{ height: 8px; border-radius: 99px; background: var(--line); overflow: hidden; margin: 10px 0 6px; }}
.bar span {{ display: block; height: 100%; background: var(--gold); }}
.url {{ direction: ltr; text-align: left; word-break: break-all; font: 12px ui-monospace, monospace; background: #0b0a09; padding: 10px; border-radius: 8px; border: 1px solid var(--line); }}
button, .app {{ background: var(--gold); color: #1a1508; border: 0; border-radius: 8px; padding: 6px 14px; font: inherit; font-size: 13px; font-weight: 600; cursor: pointer; text-decoration: none; display: inline-block; }}
.apps {{ display: flex; flex-wrap: wrap; gap: 8px; margin-top: 10px; }}
.app {{ background: #2a2620; color: var(--text); font-weight: 500; }}
ul {{ list-style: none; margin: 0; padding: 0; }}
li {{ border-top: 1px solid var(--line); padding: 10px 0; }}
li:first-child {{ border-top: 0; }}
.row {{ display: flex; align-items: center; justify-content: space-between; gap: 12px; }}
.name {{ overflow-wrap: anywhere; }}
summary {{ cursor: pointer; color: var(--muted); font-size: 13px; margin-top: 4px; }}
.qr {{ background: #fff; display: inline-block; padding: 8px; border-radius: 10px; margin-top: 8px; }}
.qr svg {{ display: block; width: 200px; height: 200px; }}
.support {{ color: var(--gold); }}
</style>
</head>
<body>
<main>
<h1>{title}</h1>
<div class="muted" dir="ltr" style="text-align:inherit">{username}</div>
<section class="card">
<span class="status {status_class}">{status}</span>
<div class="bar"><span style="width:{percent_used:.1}%"></span></div>
<div>{used}: <span dir="ltr">{usage}</span></div>
<div class="muted">{expiry}</div>
</section>
<section class="card">
<div class="row"><strong>{subscription}</strong><button data-copy="{url}">{copy}</button></div>
<div class="url" style="margin-top:10px">{url}</div>
<div class="qr">{sub_qr}</div>
<div class="muted" style="margin-top:12px">{open_in}</div>
<div class="apps">{buttons}</div>
</section>
<section class="card">
<strong>{configs_title}</strong>
<ul style="margin-top:8px">{configs}</ul>
</section>
<p>{support}</p>
</main>
<script>
document.addEventListener("click", function (event) {{
  var button = event.target.closest("[data-copy]");
  if (!button || !navigator.clipboard) return;
  navigator.clipboard.writeText(button.getAttribute("data-copy")).then(function () {{
    var text = button.textContent;
    button.textContent = "{copied}";
    setTimeout(function () {{ button.textContent = text; }}, 1500);
  }});
}});
</script>
</body>
</html>"#,
        lang = w.lang,
        dir = w.dir,
        title = title,
        username = escape(&user.username),
        status_class = escape(&user.status),
        status = (w.status)(&user.status),
        percent_used = percent_used,
        used = w.used,
        usage = usage,
        expiry = escape(&expiry),
        subscription = w.subscription,
        url = url,
        copy = w.copy,
        copied = w.copied,
        sub_qr = qr(sub_url),
        open_in = w.open_in,
        buttons = buttons,
        configs_title = w.configs,
        configs = configs,
        support = support,
    )
}
