//! The page a browser gets at a subscription's address: the user's status,
//! usage, data limit and time (with the Persian calendar in Persian), the
//! admin's announcement, the subscription link and every config with copy
//! buttons and QR codes, and a list of apps for the visitor's device with a link that adds the
//! subscription to each. Dark or light, Persian or English (the browser's language, or
//! `?lang=`). An admin can replace it with their own HTML
//! ([`super::Settings::template`]).
//!
//! The features follow the page PasarGuard shows its users; the code is
//! written here. Everything shown is escaped; the script on the page only
//! switches the theme, copies text and opens the QR popup, and nothing is
//! loaded from elsewhere (the QR codes are drawn by the panel).

use serde_json::json;

use super::apps::{Platform, APPS};
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
    /// The other language's code and how it is written there, for the switch.
    other: &'static str,
    other_name: &'static str,
    theme: &'static str,
    status_label: &'static str,
    data_limit: &'static str,
    data_used: &'static str,
    unlimited: &'static str,
    expires: &'static str,
    never: &'static str,
    days_left: &'static str,
    days: &'static str,
    on_hold_for: &'static str,
    on_hold_until: &'static str,
    subscription: &'static str,
    copy: &'static str,
    copied: &'static str,
    copy_all: &'static str,
    copied_all: &'static str,
    apps_title: &'static str,
    recommended: &'static str,
    add_sub: &'static str,
    get: &'static str,
    configs: &'static str,
    none: &'static str,
    qr: &'static str,
    close: &'static str,
    support: &'static str,
    more: &'static str,
    status: fn(&str) -> &'static str,
    /// How a usage that starts over is said, empty for one that does not.
    reset: fn(&str) -> &'static str,
}

const FA: Words = Words {
    dir: "rtl",
    lang: "fa",
    other: "en",
    other_name: "English",
    theme: "تغییر تم",
    status_label: "وضعیت",
    data_limit: "حجم کل",
    data_used: "مصرف شده",
    unlimited: "نامحدود",
    expires: "انقضا",
    never: "بدون انقضا",
    days_left: "روز مانده",
    days: "روز",
    on_hold_for: "مدت پس از اولین اتصال",
    on_hold_until: "منتظر اتصال تا",
    subscription: "لینک اشتراک",
    copy: "کپی",
    copied: "کپی شد",
    copy_all: "کپی همه‌ی کانفیگ‌ها",
    copied_all: "همه کپی شد",
    apps_title: "برنامه‌ها",
    recommended: "پیشنهادی",
    add_sub: "افزودن اشتراک",
    get: "دانلود",
    configs: "کانفیگ‌ها",
    none: "کانفیگی برای این اشتراک نیست.",
    qr: "QR",
    close: "بستن",
    support: "پشتیبانی",
    more: "بیشتر",
    status: |status| match status {
        "active" => "فعال",
        "on_hold" => "در انتظار اولین اتصال",
        "disabled" => "غیرفعال",
        "limited" => "حجم تمام شده",
        "expired" => "منقضی شده",
        _ => "",
    },
    reset: |strategy| match strategy {
        "daily" => "هر روز از نو",
        "weekly" => "هر هفته از نو",
        "monthly" => "هر ماه از نو",
        "yearly" => "هر سال از نو",
        _ => "",
    },
};

const EN: Words = Words {
    dir: "ltr",
    lang: "en",
    other: "fa",
    other_name: "فارسی",
    theme: "Switch theme",
    status_label: "Status",
    data_limit: "Data limit",
    data_used: "Data used",
    unlimited: "Unlimited",
    expires: "Expires",
    never: "Never expires",
    days_left: "days left",
    days: "days",
    on_hold_for: "Time after the first connection",
    on_hold_until: "Waiting for a connection until",
    subscription: "Subscription link",
    copy: "Copy",
    copied: "Copied",
    copy_all: "Copy all configs",
    copied_all: "All copied",
    apps_title: "Apps",
    recommended: "Recommended",
    add_sub: "Add subscription",
    get: "Download",
    configs: "Configs",
    none: "This subscription has no configs.",
    qr: "QR",
    close: "Close",
    support: "Support",
    more: "More",
    status: |status| match status {
        "active" => "Active",
        "on_hold" => "Waiting for the first connection",
        "disabled" => "Disabled",
        "limited" => "Out of data",
        "expired" => "Expired",
        _ => "",
    },
    reset: |strategy| match strategy {
        "daily" => "starts over every day",
        "weekly" => "starts over every week",
        "monthly" => "starts over every month",
        "yearly" => "starts over every year",
        _ => "",
    },
};

fn percent(text: &str) -> String {
    percent_encoding::utf8_percent_encode(text, percent_encoding::NON_ALPHANUMERIC).to_string()
}

/// `1405/08/03`: the Persian calendar's date of a Unix time (UTC).
pub fn jalali(seconds: i64) -> String {
    // The Gregorian date first (the same arithmetic as [`date`]).
    let text = date(seconds);
    let mut parts = text.split('-').map(|part| part.parse::<i64>().unwrap_or(0));
    let (gy, gm, gd) = (
        parts.next().unwrap_or(1970),
        parts.next().unwrap_or(1),
        parts.next().unwrap_or(1),
    );
    const BEFORE: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let gy2 = if gm > 2 { gy + 1 } else { gy };
    let mut days = 355_666 + 365 * gy + (gy2 + 3) / 4 - (gy2 + 99) / 100
        + (gy2 + 399) / 400
        + gd
        + BEFORE[(gm - 1).clamp(0, 11) as usize];
    let mut year = -1595 + 33 * (days / 12_053);
    days %= 12_053;
    year += 4 * (days / 1461);
    days %= 1461;
    if days > 365 {
        year += (days - 1) / 365;
        days = (days - 1) % 365;
    }
    let (month, day) = if days < 186 {
        (1 + days / 31, 1 + days % 31)
    } else {
        (7 + (days - 186) / 30, 1 + (days - 186) % 30)
    };
    format!("{year:04}/{month:02}/{day:02}")
}

/// `text` with its digits in the Persian ones.
fn persian_digits(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '0'..='9' => char::from_u32('۰' as u32 + (c as u32 - '0' as u32)).unwrap_or(c),
            c => c,
        })
        .collect()
}

/// The scheme of a link, for its badge: `vless`, `ss`, …
fn scheme(link: &str) -> String {
    link.split("://")
        .next()
        .filter(|s| s.len() <= 12 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        .unwrap_or("")
        .to_string()
}

pub fn render(
    settings: &Settings,
    user: &z_db::User,
    endpoints: &[Endpoint],
    sub_url: &str,
    accept_language: &str,
    lang: Option<&str>,
    user_agent: &str,
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
            "announce": settings.announce,
            "announce_url": settings.announce_url,
            "links": links.iter().map(|(remark, link)| json!({"remark": remark, "link": link})).collect::<Vec<_>>(),
        });
        // Safe inside a <script> element: nothing in it can close one.
        let data = data.to_string().replace('<', "\\u003c");
        return settings.template.replace("{{data}}", &data);
    }

    let w = match lang {
        Some("fa") => &FA,
        Some("en") => &EN,
        _ if accept_language
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("fa") =>
        {
            &FA
        }
        _ => &EN,
    };
    let fa = w.lang == "fa";
    // Numbers and dates the way the language writes them.
    let num = |text: String| if fa { persian_digits(&text) } else { text };
    let day = |at: i64| if fa { jalali(at) } else { date(at) };
    let now = z_db::now();

    let percent_used = if user.data_limit > 0 {
        (user.used_traffic as f64 / user.data_limit as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let bar_class = match percent_used {
        p if p >= 100.0 => "full",
        p if p >= 85.0 => "warn",
        _ => "ok",
    };
    let usage = if user.data_limit > 0 {
        format!("{} / {}", bytes(user.used_traffic), bytes(user.data_limit))
    } else {
        format!("{} / {}", bytes(user.used_traffic), w.unlimited)
    };

    let mut facts: Vec<(&str, String)> = Vec::new();
    facts.push((
        w.data_limit,
        if user.data_limit > 0 {
            bytes(user.data_limit)
        } else {
            w.unlimited.to_string()
        },
    ));
    let mut used = bytes(user.used_traffic);
    let reset = (w.reset)(&user.reset_strategy);
    if !reset.is_empty() {
        used.push_str(&format!(" · {reset}"));
    }
    facts.push((w.data_used, used));
    if user.status == "on_hold" {
        facts.push((
            w.on_hold_for,
            match user.on_hold_duration {
                Some(seconds) if seconds > 0 => format!("{} {}", seconds / 86_400, w.days),
                _ => "∞".to_string(),
            },
        ));
        facts.push((
            w.on_hold_until,
            user.on_hold_timeout.map_or_else(|| "∞".to_string(), day),
        ));
    } else {
        facts.push((
            w.expires,
            match user.expire_at {
                Some(at) => format!(
                    "{} · {} {}",
                    day(at),
                    (at - now).max(0) / 86_400,
                    w.days_left
                ),
                None => w.never.to_string(),
            },
        ));
    }
    let facts: String = facts
        .iter()
        .map(|(label, value)| {
            format!(
                "<div><dt>{label}</dt><dd dir=\"auto\">{}</dd></div>",
                escape(&num(value.clone()))
            )
        })
        .collect();

    let title = escape(&settings.title);
    let url = escape(sub_url);
    let encoded = percent(sub_url);
    let title_encoded = percent(&settings.title);
    let detected = Platform::of(user_agent);
    let tabs: String = Platform::ALL
        .iter()
        .map(|platform| {
            format!(
                "<button class=\"tab\" type=\"button\" data-tab=\"{}\">{}</button>",
                platform.key(),
                platform.name()
            )
        })
        .collect();
    let app_cards: String = APPS
        .iter()
        .map(|app| {
            let platforms = app
                .platforms
                .iter()
                .map(|p| p.key())
                .collect::<Vec<_>>()
                .join(" ");
            let badge = if app.recommended {
                format!("<small class=\"badge rec\">{}</small>", w.recommended)
            } else {
                String::new()
            };
            let add = app.import.map_or_else(String::new, |import| {
                format!(
                    "<a class=\"app primary\" href=\"{}\">{}</a>",
                    escape(&import(sub_url, &encoded, &title_encoded)),
                    w.add_sub
                )
            });
            let get: String = app
                .get
                .iter()
                .map(|(label, target)| {
                    format!(
                        "<a class=\"app\" href=\"{}\" rel=\"noopener noreferrer\">{} · {}</a>",
                        escape(target),
                        w.get,
                        escape(label)
                    )
                })
                .collect();
            format!(
                "<div class=\"app-card\" data-platforms=\"{platforms}\"><div class=\"app-head\"><strong>{}</strong>{badge}</div>\
                 <p class=\"muted\">{}</p><div class=\"apps\">{add}{get}</div></div>",
                app.name,
                if fa { app.about_fa } else { app.about_en }
            )
        })
        .collect();

    let mut templates = String::new();
    let configs: String = if links.is_empty() {
        format!("<p class=\"muted\">{}</p>", w.none)
    } else {
        links
            .iter()
            .enumerate()
            .map(|(n, (remark, link))| {
                templates.push_str(&format!("<template id=\"qr-{n}\">{}</template>", qr(link)));
                format!(
                    "<li><div class=\"row\"><span class=\"name\" dir=\"auto\">{}<small class=\"badge\">{}</small></span>\
                     <span class=\"acts\"><button class=\"ghost\" data-qr=\"qr-{n}\" data-name=\"{}\">{}</button>\
                     <button data-copy=\"{}\" data-link>{}</button></span></div></li>",
                    escape(remark),
                    escape(&scheme(link)),
                    escape(remark),
                    w.qr,
                    escape(link),
                    w.copy
                )
            })
            .collect()
    };
    let copy_all = if links.is_empty() {
        String::new()
    } else {
        format!(
            "<button class=\"wide\" id=\"copy-all\" data-done=\"{}\">{}</button>",
            w.copied_all, w.copy_all
        )
    };
    let announcement = if settings.announce.trim().is_empty() {
        String::new()
    } else {
        let more = if settings.announce_url.starts_with("http") {
            format!(
                " <a href=\"{}\" rel=\"noopener noreferrer\">{}</a>",
                escape(&settings.announce_url),
                w.more
            )
        } else {
            String::new()
        };
        format!(
            "<section class=\"note\" role=\"note\">{}{}</section>",
            escape(settings.announce.trim()).replace('\n', "<br>"),
            more
        )
    };
    let support = if settings.support_url.is_empty() {
        String::new()
    } else {
        format!(
            "<p class=\"foot\"><a class=\"support\" href=\"{}\" rel=\"noopener noreferrer\">{}</a></p>",
            escape(&settings.support_url),
            w.support
        )
    };
    let initial: String = user
        .username
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();

    let values: Vec<(&str, String)> = vec![
        ("lang", w.lang.to_string()),
        ("dir", w.dir.to_string()),
        (
            "page_title",
            format!("{} · {title}", escape(&user.username)),
        ),
        ("title", title),
        ("other", w.other.to_string()),
        ("other_name", w.other_name.to_string()),
        ("theme", w.theme.to_string()),
        ("announcement", announcement),
        ("initial", escape(&initial)),
        ("username", escape(&user.username)),
        ("status_label", w.status_label.to_string()),
        ("status_class", escape(&user.status)),
        ("status", (w.status)(&user.status).to_string()),
        ("bar_class", bar_class.to_string()),
        ("percent_used", format!("{percent_used:.1}")),
        ("percent_text", num(format!("{percent_used:.0}%"))),
        ("usage", escape(&num(usage))),
        ("facts", facts),
        ("subscription", w.subscription.to_string()),
        ("url", url),
        ("copy", w.copy.to_string()),
        ("copied", w.copied.to_string()),
        ("qr_word", w.qr.to_string()),
        (
            "sub_qr_template",
            format!("<template id=\"qr-sub\">{}</template>", qr(sub_url)),
        ),
        ("apps_title", w.apps_title.to_string()),
        ("detected", detected.map_or("", |p| p.key()).to_string()),
        ("tabs", tabs),
        ("app_cards", app_cards),
        ("configs_title", w.configs.to_string()),
        ("configs", configs),
        ("copy_all", copy_all),
        ("templates", templates),
        ("close", w.close.to_string()),
        ("support", support),
    ];
    fill(PAGE, &values)
}

/// `template` with each `{name}` that `values` has a value for replaced by it,
/// in one pass: what is put in is never read again for more names, so a user
/// name or a remark that looks like one stays what it is.
fn fill(template: &str, values: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len() + 4096);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let named = after
            .find('}')
            .filter(|close| *close <= 24)
            .map(|close| &after[..close])
            .and_then(|name| values.iter().find(|(known, _)| *known == name));
        match named {
            Some((name, value)) => {
                out.push_str(value);
                rest = &after[name.len() + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

const PAGE: &str = r##"<!doctype html>
<html lang="{lang}" dir="{dir}" data-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<meta name="robots" content="noindex">
<meta name="color-scheme" content="dark light">
<title>{page_title}</title>
<script>(function(){try{var t=localStorage.getItem("zui-theme");if(!t&&window.matchMedia&&matchMedia("(prefers-color-scheme: light)").matches)t="light";document.documentElement.setAttribute("data-theme",t==="light"?"light":"dark")}catch(e){}})()</script>
<style>
:root{--bg:#0f0e0c;--card:#191816;--line:#2b2925;--text:#f2efe8;--muted:#a39d92;--gold:#e2b54d;--on-gold:#1a1508;--soft:#25221d;--ok:#4fc483;--ok-bg:#18291f;--bad:#ef6b5b;--bad-bg:#2c1a17;--warn:#e8a23c;--code:#0b0a09}
:root[data-theme=light]{--bg:#f6f3ec;--card:#fffdf8;--line:#e3dccd;--text:#231f17;--muted:#6f685b;--gold:#b88a1c;--on-gold:#fffdf8;--soft:#f0eadc;--ok:#1e8a52;--ok-bg:#e1f3e8;--bad:#c0392b;--bad-bg:#fbe7e3;--warn:#b86e00;--code:#f0eadc}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font:15px/1.6 system-ui,"Vazirmatn","Segoe UI",Roboto,sans-serif;transition:background .2s,color .2s}
main{max-width:680px;margin:0 auto;padding:16px 16px 56px}
.top{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:6px 0 10px}
.brand{display:flex;align-items:center;gap:10px;font-weight:700;color:var(--gold)}
.logo{display:grid;place-items:center;width:30px;height:30px;border-radius:9px;background:var(--gold);color:var(--on-gold);font-weight:800}
.tools{display:flex;gap:8px}
.chip{background:var(--soft);color:var(--text);border:1px solid var(--line);border-radius:99px;padding:4px 12px;font:inherit;font-size:13px;cursor:pointer;text-decoration:none}
.card{background:var(--card);border:1px solid var(--line);border-radius:16px;padding:18px;margin-top:14px}
.note{margin-top:8px;padding:12px 14px;border-radius:12px;border:1px solid var(--gold);background:var(--soft);font-size:14px}
.note a{color:var(--gold);font-weight:600}
.who{display:flex;align-items:center;gap:14px}
.avatar{display:grid;place-items:center;flex:none;width:48px;height:48px;border-radius:50%;background:var(--gold);color:var(--on-gold);font-size:20px;font-weight:800}
h1{margin:0;font-size:19px;overflow-wrap:anywhere}
.status{display:inline-block;margin-top:2px;padding:1px 10px;border-radius:99px;font-size:12.5px;background:var(--soft);color:var(--gold)}
.status.active{color:var(--ok);background:var(--ok-bg)}
.status.limited,.status.expired,.status.disabled{color:var(--bad);background:var(--bad-bg)}
.meter{margin-top:16px}
.bar{height:9px;border-radius:99px;background:var(--line);overflow:hidden}
.bar span{display:block;height:100%;background:var(--gold);border-radius:99px;transition:width .4s}
.bar.warn span{background:var(--warn)}
.bar.full span{background:var(--bad)}
.meter-text{display:flex;justify-content:space-between;gap:12px;margin-top:6px;font-size:13px;color:var(--muted)}
.facts{display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:10px 16px;margin:16px 0 0}
.facts div{padding:10px 12px;border:1px solid var(--line);border-radius:12px;background:var(--soft)}
.facts dt{font-size:12px;color:var(--muted)}
.facts dd{margin:0;font-weight:600;overflow-wrap:anywhere}
.row{display:flex;align-items:center;justify-content:space-between;gap:12px}
.row strong{font-size:15px}
.url{direction:ltr;text-align:left;word-break:break-all;font:12px ui-monospace,monospace;background:var(--code);padding:10px;border-radius:10px;border:1px solid var(--line);margin-top:10px}
button,.app{background:var(--gold);color:var(--on-gold);border:0;border-radius:10px;padding:7px 14px;font:inherit;font-size:13px;font-weight:600;cursor:pointer;text-decoration:none;display:inline-block}
button.ghost,.app{background:var(--soft);color:var(--text);border:1px solid var(--line);font-weight:500}
button.wide{width:100%;margin-top:12px;padding:10px}
.apps{display:flex;flex-wrap:wrap;gap:8px;margin-top:10px}
.tabs{display:flex;flex-wrap:wrap;gap:6px;margin-top:10px}
.tab{background:var(--soft);color:var(--text);border:1px solid var(--line);font-weight:500;border-radius:99px;padding:5px 14px}
.tab.on{background:var(--gold);color:var(--on-gold);border-color:var(--gold);font-weight:700}
.app-card{border:1px solid var(--line);border-radius:14px;padding:12px 14px;margin-top:10px;background:var(--soft)}
.app-card[hidden]{display:none}
.app-card p{margin:2px 0 0;font-size:13px}
.app-head{display:flex;align-items:center;gap:8px}
.badge.rec{color:var(--gold);border-color:var(--gold);text-transform:none}
.app.primary{background:var(--gold);color:var(--on-gold);border-color:var(--gold);font-weight:700}
.muted{color:var(--muted)}
ul{list-style:none;margin:10px 0 0;padding:0}
li{border-top:1px solid var(--line);padding:10px 0}
li:first-child{border-top:0}
.name{overflow-wrap:anywhere;display:flex;align-items:center;gap:8px;min-width:0}
.badge{flex:none;font-size:11px;padding:0 8px;border-radius:99px;background:var(--soft);color:var(--muted);border:1px solid var(--line);text-transform:uppercase}
.acts{display:flex;gap:6px;flex:none}
.foot{text-align:center;margin-top:20px}
.support{color:var(--gold)}
.popup{position:fixed;inset:0;display:none;place-items:center;background:rgba(0,0,0,.65);padding:16px;z-index:10}
.popup.open{display:grid}
.popup-card{background:var(--card);border:1px solid var(--line);border-radius:18px;padding:18px;text-align:center;max-width:320px;width:100%}
.popup-card .qr{background:#fff;border-radius:12px;padding:10px;display:inline-block}
.popup-card svg{display:block;width:240px;height:240px;max-width:100%}
.popup-card .who-name{margin:10px 0;font-size:14px;overflow-wrap:anywhere}
</style>
</head>
<body>
<main>
<header class="top">
<div class="brand"><span class="logo">Z</span><span>{title}</span></div>
<div class="tools"><a class="chip" href="?lang={other}" hreflang="{other}">{other_name}</a><button class="chip" id="theme" type="button" aria-label="{theme}" title="{theme}"><svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true" style="display:block"><circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" stroke-width="2"/><path d="M12 3a9 9 0 0 1 0 18z" fill="currentColor"/></svg></button></div>
</header>
{announcement}
<section class="card">
<div class="who"><span class="avatar">{initial}</span><div><h1 dir="auto">{username}</h1><span class="status {status_class}" title="{status_label}">{status}</span></div></div>
<div class="meter"><div class="bar {bar_class}"><span style="width:{percent_used}%"></span></div>
<div class="meter-text"><span dir="ltr">{usage}</span><span>{percent_text}</span></div></div>
<dl class="facts">{facts}</dl>
</section>
<section class="card">
<div class="row"><strong>{subscription}</strong><span class="acts"><button class="ghost" data-qr="qr-sub" data-name="{subscription}">{qr_word}</button><button data-copy="{url}">{copy}</button></span></div>
<div class="url">{url}</div>
</section>
<section class="card" id="apps" data-detected="{detected}">
<strong>{apps_title}</strong>
<div class="tabs" role="tablist">{tabs}</div>
<div>{app_cards}</div>
</section>
<section class="card">
<strong>{configs_title}</strong>
<ul>{configs}</ul>
{copy_all}
</section>
{support}
</main>
<div class="popup" id="popup" role="dialog" aria-modal="true"><div class="popup-card"><div class="qr" id="popup-qr"></div><div class="who-name" id="popup-name"></div><button type="button" id="popup-close">{close}</button></div></div>
{sub_qr_template}{templates}
<script>
(function () {
  var COPIED = "{copied}";
  function copy(text) {
    if (navigator.clipboard && window.isSecureContext) return navigator.clipboard.writeText(text);
    return new Promise(function (resolve, reject) {
      var area = document.createElement("textarea");
      area.value = text; area.setAttribute("readonly", ""); area.style.position = "fixed"; area.style.opacity = "0";
      document.body.appendChild(area); area.select();
      var ok = false;
      try { ok = document.execCommand("copy"); } catch (e) {}
      document.body.removeChild(area);
      ok ? resolve() : reject();
    });
  }
  function flash(button, text) {
    var was = button.getAttribute("data-label") || button.textContent;
    button.setAttribute("data-label", was);
    button.textContent = text;
    setTimeout(function () { button.textContent = was; }, 1500);
  }
  var popup = document.getElementById("popup");
  function closePopup() { popup.classList.remove("open"); }
  document.addEventListener("click", function (event) {
    var target = event.target;
    var copyButton = target.closest("[data-copy]");
    if (copyButton) { copy(copyButton.getAttribute("data-copy")).then(function () { flash(copyButton, COPIED); }, function () {}); return; }
    var qrButton = target.closest("[data-qr]");
    if (qrButton) {
      var source = document.getElementById(qrButton.getAttribute("data-qr"));
      if (!source) return;
      var box = document.getElementById("popup-qr");
      box.innerHTML = "";
      box.appendChild(source.content.cloneNode(true));
      document.getElementById("popup-name").textContent = qrButton.getAttribute("data-name") || "";
      popup.classList.add("open");
      return;
    }
    if (target.closest("#copy-all")) {
      var all = document.getElementById("copy-all");
      var links = Array.prototype.map.call(document.querySelectorAll("[data-link]"), function (b) { return b.getAttribute("data-copy"); });
      copy(links.join("\n")).then(function () { flash(all, all.getAttribute("data-done")); }, function () {});
      return;
    }
    if (target === popup || target.closest("#popup-close")) closePopup();
  });
  document.addEventListener("keydown", function (event) { if (event.key === "Escape") closePopup(); });
  var apps = document.getElementById("apps");
  function show(platform) {
    Array.prototype.forEach.call(apps.querySelectorAll(".tab"), function (tab) {
      tab.classList.toggle("on", tab.getAttribute("data-tab") === platform);
    });
    Array.prototype.forEach.call(apps.querySelectorAll(".app-card"), function (card) {
      card.hidden = (" " + card.getAttribute("data-platforms") + " ").indexOf(" " + platform + " ") < 0;
    });
  }
  apps.addEventListener("click", function (event) {
    var tab = event.target.closest(".tab");
    if (tab) show(tab.getAttribute("data-tab"));
  });
  show(apps.getAttribute("data-detected") || "android");
  document.getElementById("theme").addEventListener("click", function () {
    var next = document.documentElement.getAttribute("data-theme") === "light" ? "dark" : "light";
    document.documentElement.setAttribute("data-theme", next);
    try { localStorage.setItem("zui-theme", next); } catch (e) {}
  });
})();
</script>
</body>
</html>"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::endpoint::{Protocol, Security, Transport};

    fn user(name: &str, status: &str) -> z_db::User {
        z_db::User {
            id: 1,
            username: name.into(),
            status: status.into(),
            uuid: String::new(),
            password: String::new(),
            sub_token: "tok".into(),
            data_limit: 10 * 1024 * 1024 * 1024,
            used_traffic: 9 * 1024 * 1024 * 1024,
            lifetime_traffic: 0,
            expire_at: Some(z_db::now() + 5 * 86_400 + 100),
            on_hold_duration: None,
            on_hold_timeout: None,
            reset_strategy: "monthly".into(),
            last_reset_at: None,
            ip_limit: 0,
            speed_up: 0,
            speed_down: 0,
            note: String::new(),
            online_at: None,
            admin_id: None,
            created_at: 0,
            updated_at: 0,
            group_ids: Vec::new(),
            next_plan: None,
            hwid_limit: 0,
            sub_updated_at: None,
            sub_user_agent: String::new(),
        }
    }

    fn endpoint(remark: &str) -> Endpoint {
        Endpoint {
            remark: remark.into(),
            address: "203.0.113.5".into(),
            port: 443,
            protocol: Protocol::Vless {
                id: "9a07bd14-8711-443d-8248-cba7b191509f".into(),
                flow: String::new(),
            },
            transport: Transport::Raw,
            security: Security::None,
        }
    }

    #[test]
    fn the_persian_calendar_is_right_on_known_days() {
        // Nowruz, and a day in between, checked against the calendar.
        assert_eq!(jalali(1_774_051_200), "1405/01/01"); // 2026-03-21
        assert_eq!(jalali(1_742_515_200), "1404/01/01"); // 2025-03-21
        assert_eq!(jalali(1_792_886_400), "1405/08/03"); // 2026-10-25
        assert_eq!(jalali(1_773_964_800), "1404/12/29"); // 2026-03-20, the last day of 1404
        assert_eq!(persian_digits("12.5 GB · 3"), "۱۲.۵ GB · ۳");
    }

    #[test]
    fn a_name_that_looks_like_a_placeholder_stays_what_it_is() {
        let filled = fill(
            "<a>{one}</a><b>{two}</b>{three} {not a name} {",
            &[("one", "{two}".into()), ("two", "2".into())],
        );
        assert_eq!(filled, "<a>{two}</a><b>2</b>{three} {not a name} {");
    }

    #[test]
    fn the_page_is_escaped_and_carries_what_it_should() {
        let settings = Settings {
            announce: "Maintenance <b>tonight</b>\nback by 9".into(),
            announce_url: "https://example.org/status?a=1&b=2".into(),
            ..Settings::default()
        };
        let user = user("<img src=x onerror=1>{configs}", "active");
        let page = render(
            &settings,
            &user,
            &[endpoint("one <script>alert(1)</script>"), endpoint("two")],
            "https://sub.example.org/sub/tok",
            "en-US",
            None,
            "Mozilla/5.0 (Linux; Android 14; Pixel 8) Chrome/126 Mobile",
        );
        assert!(!page.contains("<img src=x"), "a user name is text");
        assert!(!page.contains("<script>alert"), "a remark is text");
        assert!(page.contains("&lt;img src=x onerror=1&gt;{configs}"));
        assert!(page.contains("Maintenance &lt;b&gt;tonight&lt;/b&gt;<br>back by 9"));
        assert!(page.contains("href=\"https://example.org/status?a=1&amp;b=2\""));
        // Both themes, the switch, the copy-all button, the QR popup's source.
        assert!(page.contains("data-theme=\"dark\"") && page.contains("[data-theme=light]"));
        assert!(page.contains("id=\"copy-all\"") && page.contains("id=\"qr-sub\""));
        assert_eq!(
            page.matches("data-link>").count(),
            2,
            "one copy button for each config"
        );
        assert_eq!(
            page.matches("<template id=\"qr-").count(),
            3,
            "the link's and each config's"
        );
        assert!(page.contains("class=\"bar warn\""), "90% used is a warning");
        assert!(page.contains("starts over every month") && page.contains("5 days left"));
        assert!(page.contains("<small class=\"badge\">vless</small>"));
        assert!(page.contains("lang=\"en\" dir=\"ltr\""));
        // The apps: the visitor's device is the tab to start on, and every app
        // that can add a subscription has a link that does.
        assert!(page.contains("id=\"apps\" data-detected=\"android\""));
        assert_eq!(page.matches("class=\"tab\"").count(), 5);
        assert!(page.contains("href=\"hiddify://import/https://sub.example.org/sub/tok#"));
        assert!(page.contains(
            "href=\"v2rayng://install-sub?url=https%3A%2F%2Fsub%2Eexample%2Eorg%2Fsub%2Ftok#"
        ));
        assert!(
            page.contains("data-platforms=\"windows\""),
            "v2rayN is Windows only"
        );
        assert!(
            page.contains("Download · GitHub"),
            "an app with a release page links to it"
        );
    }

    #[test]
    fn the_apps_follow_the_language_and_an_unknown_device_starts_on_android() {
        let settings = Settings::default();
        let render_in = |accept: &str, agent: &str| {
            render(
                &settings,
                &user("sara", "active"),
                &[],
                "https://s/sub/t",
                accept,
                None,
                agent,
            )
        };
        let fa = render_in(
            "fa",
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X)",
        );
        assert!(fa.contains("data-detected=\"ios\""));
        assert!(fa.contains("افزودن اشتراک") && fa.contains("پیشنهادی"));
        assert!(render_in("en", "curl/8").contains("data-detected=\"\""));
    }

    #[test]
    fn the_language_is_asked_for_then_the_browsers_then_english() {
        let settings = Settings::default();
        let user = user("sara", "on_hold");
        let page = |accept: &str, lang: Option<&str>| {
            render(&settings, &user, &[], "https://s/sub/t", accept, lang, "")
        };
        let fa = page("fa-IR,fa;q=0.9", None);
        assert!(fa.contains("lang=\"fa\" dir=\"rtl\""));
        assert!(
            fa.contains("href=\"?lang=en\""),
            "the switch offers the other one"
        );
        // ?lang= beats the browser, and an unknown one is ignored.
        assert!(page("fa-IR", Some("en")).contains("lang=\"en\" dir=\"ltr\""));
        assert!(page("en-US", Some("fa")).contains("lang=\"fa\" dir=\"rtl\""));
        assert!(page("de", Some("xx")).contains("lang=\"en\""));
        // Persian shows the Persian calendar's date and Persian digits; an
        // on-hold user shows what it waits for.
        let mut dated = user.clone();
        dated.status = "active".into();
        dated.expire_at = Some(1_792_886_400);
        let fa = render(&settings, &dated, &[], "https://s/sub/t", "fa", None, "");
        assert!(fa.contains("۱۴۰۵/۰۸/۰۳"), "{fa}");
        assert!(page("en", None).contains("Waiting for the first connection"));
    }

    #[test]
    fn a_users_own_template_still_replaces_the_page_and_gets_the_announcement() {
        let settings = Settings {
            template: "<script>var d = {{data}};</script>".into(),
            announce: "hello </script>".into(),
            ..Settings::default()
        };
        let page = render(
            &settings,
            &user("a", "active"),
            &[],
            "https://s/sub/t",
            "",
            None,
            "",
        );
        assert!(
            page.contains("\"announce\":\"hello \\u003c/script>\""),
            "{page}"
        );
        assert_eq!(
            page.matches("</script>").count(),
            1,
            "nothing closes the script early"
        );
    }
}
