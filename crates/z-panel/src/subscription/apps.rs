//! The apps the page offers for opening a subscription in: for each, the
//! platforms it runs on, a link that adds the subscription to it, and where
//! to get it. Every link here was checked against the app's own source or
//! release page; an app with no link that adds a subscription says so.

/// What a visitor's device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Android,
    Ios,
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    pub const ALL: [Platform; 5] = [
        Platform::Android,
        Platform::Ios,
        Platform::Windows,
        Platform::MacOs,
        Platform::Linux,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Platform::Android => "android",
            Platform::Ios => "ios",
            Platform::Windows => "windows",
            Platform::MacOs => "macos",
            Platform::Linux => "linux",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Platform::Android => "Android",
            Platform::Ios => "iOS",
            Platform::Windows => "Windows",
            Platform::MacOs => "macOS",
            Platform::Linux => "Linux",
        }
    }

    /// The device a browser says it is, from its `User-Agent`.
    pub fn of(user_agent: &str) -> Option<Platform> {
        let agent = user_agent.to_ascii_lowercase();
        if agent.contains("android") {
            Some(Platform::Android)
        } else if agent.contains("iphone") || agent.contains("ipad") || agent.contains("ipod") {
            Some(Platform::Ios)
        } else if agent.contains("windows") {
            Some(Platform::Windows)
        } else if agent.contains("macintosh") || agent.contains("mac os x") {
            Some(Platform::MacOs)
        } else if agent.contains("linux") || agent.contains("x11") {
            Some(Platform::Linux)
        } else {
            None
        }
    }
}

pub struct App {
    pub name: &'static str,
    pub platforms: &'static [Platform],
    pub recommended: bool,
    /// The link that adds the subscription at `url` (with `encoded` the same
    /// percent-encoded, `title` its name, encoded), if the app has one.
    pub import: Option<fn(url: &str, encoded: &str, title: &str) -> String>,
    /// (what the link is called, where it goes)
    pub get: &'static [(&'static str, &'static str)],
    pub about_en: &'static str,
    pub about_fa: &'static str,
}

use Platform::{Android, Ios, Linux, MacOs, Windows};

pub const APPS: &[App] = &[
    App {
        name: "Hiddify",
        platforms: &[Android, Ios, Windows, MacOs, Linux],
        recommended: true,
        import: Some(|url, _, title| format!("hiddify://import/{url}#{title}")),
        get: &[(
            "GitHub",
            "https://github.com/hiddify/hiddify-app/releases/latest",
        )],
        about_en: "One app for every device; adds this subscription with one tap.",
        about_fa: "یک برنامه برای همه‌ی دستگاه‌ها؛ این اشتراک را با یک لمس اضافه می‌کند.",
    },
    App {
        name: "v2rayNG",
        platforms: &[Android],
        recommended: false,
        import: Some(|_, encoded, title| format!("v2rayng://install-sub?url={encoded}#{title}")),
        get: &[("GitHub", "https://github.com/2dust/v2rayNG/releases/latest")],
        about_en: "The classic Android client.",
        about_fa: "کلاینت قدیمی و آشنای اندروید.",
    },
    App {
        name: "Streisand",
        platforms: &[Ios, MacOs],
        recommended: true,
        import: Some(|url, _, _| format!("streisand://import/{url}")),
        get: &[],
        about_en: "For iPhone, iPad and Mac. Find it in the App Store.",
        about_fa: "برای آیفون، آیپد و مک. در اپ‌استور پیدایش کنید.",
    },
    App {
        name: "Happ",
        platforms: &[Android, Ios, Windows, MacOs],
        recommended: false,
        import: Some(|url, _, _| format!("happ://add/{url}")),
        get: &[("happ.su", "https://www.happ.su")],
        about_en: "A simple client for phones and computers.",
        about_fa: "کلاینت ساده برای گوشی و کامپیوتر.",
    },
    App {
        name: "sing-box",
        platforms: &[Android, Ios, MacOs],
        recommended: false,
        import: Some(|_, encoded, title| {
            format!("sing-box://import-remote-profile?url={encoded}#{title}")
        }),
        get: &[(
            "GitHub",
            "https://github.com/SagerNet/sing-box/releases/latest",
        )],
        about_en: "The sing-box apps (Android, and iOS and macOS from the stores).",
        about_fa: "برنامه‌های sing-box (اندروید، و آی‌اواس و مک از اپ‌استور).",
    },
    App {
        name: "Clash Verge",
        platforms: &[Windows, MacOs, Linux],
        recommended: false,
        import: Some(|_, encoded, title| {
            format!("clash://install-config?url={encoded}&name={title}")
        }),
        get: &[(
            "GitHub",
            "https://github.com/clash-verge-rev/clash-verge-rev/releases/latest",
        )],
        about_en: "A desktop client for Windows, macOS and Linux.",
        about_fa: "کلاینت دسکتاپ برای ویندوز، مک و لینوکس.",
    },
    App {
        name: "v2rayN",
        platforms: &[Windows],
        recommended: false,
        import: None,
        get: &[("GitHub", "https://github.com/2dust/v2rayN/releases/latest")],
        about_en: "Copy the subscription link, then Subscriptions › Add in the app.",
        about_fa: "لینک اشتراک را کپی کنید، سپس در برنامه Subscriptions › Add.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_is_told_from_its_browser() {
        let cases = [
            (
                "Mozilla/5.0 (Linux; Android 14; Pixel 8) Chrome/126 Mobile",
                Platform::Android,
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) Safari",
                Platform::Ios,
            ),
            (
                "Mozilla/5.0 (iPad; CPU OS 17_5 like Mac OS X)",
                Platform::Ios,
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Firefox/130.0",
                Platform::Windows,
            ),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari",
                Platform::MacOs,
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64) Firefox/130.0",
                Platform::Linux,
            ),
        ];
        for (agent, platform) in cases {
            assert_eq!(Platform::of(agent), Some(platform), "{agent}");
        }
        assert_eq!(Platform::of("curl/8.9"), None);
        assert_eq!(Platform::of(""), None);
    }

    #[test]
    fn every_platform_has_an_app_and_the_links_are_made_the_way_the_apps_read_them() {
        for platform in Platform::ALL {
            assert!(
                APPS.iter().any(|app| app.platforms.contains(&platform)),
                "{}",
                platform.name()
            );
        }
        let (url, encoded) = (
            "https://s.example/sub/a b",
            "https%3A%2F%2Fs%2Eexample%2Fsub%2Fa%20b",
        );
        let link = |name: &str| {
            let app = APPS.iter().find(|app| app.name == name).unwrap();
            (app.import.unwrap())(url, encoded, "My%20Panel")
        };
        assert_eq!(
            link("Hiddify"),
            "hiddify://import/https://s.example/sub/a b#My%20Panel"
        );
        assert_eq!(
            link("v2rayNG"),
            format!("v2rayng://install-sub?url={encoded}#My%20Panel")
        );
        assert_eq!(
            link("Streisand"),
            "streisand://import/https://s.example/sub/a b"
        );
        assert_eq!(link("Happ"), "happ://add/https://s.example/sub/a b");
        assert_eq!(
            link("sing-box"),
            format!("sing-box://import-remote-profile?url={encoded}#My%20Panel")
        );
        assert_eq!(
            link("Clash Verge"),
            format!("clash://install-config?url={encoded}&name=My%20Panel")
        );
        // Nothing links anywhere but https.
        for app in APPS {
            for (_, target) in app.get {
                assert!(target.starts_with("https://"), "{}", app.name);
            }
        }
    }
}
