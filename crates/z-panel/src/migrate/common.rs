//! Small conversions every reader needs.

use std::collections::HashSet;

use serde_json::Value;

use super::model::PlannedCertificate;

/// A JSON number that an older panel may have written as text.
pub fn number(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().replace(' ', "").parse::<i64>().ok(),
        Value::Bool(b) => Some(i64::from(*b)),
        _ => None,
    }
}

/// JSON kept as text in a column, even when it was kept as a JSON string of
/// JSON (some versions did).
pub fn parse_json_text(text: &str) -> Option<Value> {
    let first: Value = serde_json::from_str(text.trim()).ok()?;
    match first {
        Value::String(inner) => serde_json::from_str(&inner)
            .ok()
            .or(Some(Value::String(inner))),
        other => Some(other),
    }
}

/// A username this panel accepts (3 to 64 letters, digits, `.`, `_`, `-`,
/// `@`) made from the one the old panel had, and whether it had to be
/// changed. Unique among `taken`, which it is added to.
pub fn username(name: &str, taken: &mut HashSet<String>) -> (String, bool) {
    let mut clean: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@') {
                c
            } else {
                '_'
            }
        })
        .collect();
    clean.truncate(64);
    while clean.len() < 3 {
        clean.push('_');
    }
    let base = clean.clone();
    let mut n = 2;
    while taken.contains(&clean) {
        let suffix = format!("-{n}");
        clean = format!("{}{suffix}", &base[..base.len().min(64 - suffix.len())]);
        n += 1;
    }
    taken.insert(clean.clone());
    let changed = clean != name;
    (clean, changed)
}

/// A tag made of letters, digits, `-`, `_` and `.` (1 to 64), made from the
/// one the old panel had.
pub fn tag(raw: &str, taken: &mut HashSet<String>) -> (String, bool) {
    let mut clean: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    clean.truncate(64);
    if clean.is_empty() {
        clean = "inbound".into();
    }
    let base = clean.clone();
    let mut n = 2;
    while taken.contains(&clean) {
        let suffix = format!("-{n}");
        clean = format!("{}{suffix}", &base[..base.len().min(64 - suffix.len())]);
        n += 1;
    }
    taken.insert(clean.clone());
    let changed = clean != raw;
    (clean, changed)
}

/// Whether `text` looks like a UUID (what the core takes as an account id).
pub fn is_uuid(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(p, n)| p.len() == n && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The reset strategy for a period of `days`, and whether it is only the
/// nearest this panel has (it resets by the day, week, month or year).
pub fn reset_from_days(days: i64) -> (&'static str, bool) {
    match days {
        d if d <= 0 => ("no_reset", false),
        1 => ("day", false),
        7 => ("week", false),
        30 => ("month", false),
        365 => ("year", false),
        d if d <= 3 => ("day", true),
        d if d <= 14 => ("week", true),
        d if d <= 100 => ("month", true),
        _ => ("year", true),
    }
}

/// Lift what 3X-UI keeps in `realitySettings.settings` (the client side:
/// public key, fingerprint, spider path) up to `realitySettings`, where this
/// panel keeps it.
pub fn lift_reality(stream: &mut Value) {
    let Some(reality) = stream
        .get_mut("realitySettings")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if let Some(Value::Object(inner)) = reality.remove("settings") {
        for key in ["publicKey", "fingerprint", "spiderX", "mldsa65Verify"] {
            if let Some(value) = inner
                .get(key)
                .filter(|v| v.as_str().is_none_or(|s| !s.is_empty()))
            {
                reality
                    .entry(key.to_string())
                    .or_insert_with(|| value.clone());
            }
        }
    }
    // Xray's own configuration (PasarGuard's and Marzban's) holds only the
    // private key; links need the public one, which this panel keeps beside it.
    let has_public = reality
        .get("publicKey")
        .and_then(Value::as_str)
        .is_some_and(|k| !k.is_empty());
    if !has_public {
        let derived = reality
            .get("privateKey")
            .and_then(Value::as_str)
            .and_then(|private| zero_runtime::keys::x25519_from_private(private).ok());
        if let Some(pair) = derived {
            reality.insert("publicKey".into(), pair.public_key.into());
        }
    }
}

/// The certificate an inbound's TLS settings name, if they name one.
pub fn certificate_of(stream: &Value) -> Option<PlannedCertificate> {
    let entry = stream["tlsSettings"]["certificates"].as_array()?.first()?;
    let lines = |value: &Value| -> Option<String> {
        match value {
            Value::Array(items) => Some(
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Value::String(text) => Some(text.clone()),
            _ => None,
        }
        .filter(|text| !text.trim().is_empty())
    };
    if let (Some(cert), Some(key)) = (lines(&entry["certificate"]), lines(&entry["key"])) {
        return Some(PlannedCertificate::Inline { cert, key });
    }
    let (cert, key) = (
        entry["certificateFile"].as_str()?,
        entry["keyFile"].as_str()?,
    );
    (!cert.is_empty() && !key.is_empty()).then(|| PlannedCertificate::Files {
        cert: cert.into(),
        key: key.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_and_json_are_read_however_an_older_panel_wrote_them() {
        assert_eq!(number(&json!(5)), Some(5));
        assert_eq!(number(&json!("1 024")), Some(1024));
        assert_eq!(number(&json!(2.0)), Some(2));
        assert_eq!(number(&json!("many")), None);
        assert_eq!(number(&json!(true)), Some(1));
        assert_eq!(parse_json_text(r#"{"a":1}"#).unwrap()["a"], 1);
        assert_eq!(
            parse_json_text(r#""{\"a\":2}""#).unwrap()["a"],
            2,
            "JSON inside a JSON string"
        );
        assert_eq!(parse_json_text("garbage"), None);
    }

    #[test]
    fn a_username_is_made_acceptable_and_unique_and_the_change_is_known() {
        let mut taken = HashSet::new();
        assert_eq!(username("ali", &mut taken), ("ali".into(), false));
        assert_eq!(username("ali", &mut taken), ("ali-2".into(), true));
        assert_eq!(username("ali", &mut taken), ("ali-3".into(), true));
        assert_eq!(
            username("علی رضا", &mut taken).0,
            "_______",
            "letters it cannot take become underscores"
        );
        assert_eq!(
            username("a", &mut taken),
            ("a__".into(), true),
            "too short gets padding"
        );
        assert_eq!(
            username("user one@x.com", &mut taken),
            ("user_one@x.com".into(), true)
        );
        let long = "x".repeat(80);
        let (clean, changed) = username(&long, &mut taken);
        assert!(clean.len() == 64 && changed);
        let (second, _) = username(&long, &mut taken);
        assert!(second.len() == 64 && second != clean && second.ends_with("-2"));
    }

    #[test]
    fn a_tag_is_made_acceptable_and_unique() {
        let mut taken = HashSet::new();
        assert_eq!(
            tag("inbound-443", &mut taken),
            ("inbound-443".into(), false)
        );
        assert_eq!(
            tag("inbound 443!", &mut taken),
            ("inbound-443-".into(), true)
        );
        assert_eq!(
            tag("inbound-443", &mut taken),
            ("inbound-443-2".into(), true)
        );
        assert_eq!(tag("  ", &mut taken).0, "inbound");
    }

    #[test]
    fn periods_of_days_are_the_nearest_reset_this_panel_has() {
        assert_eq!(reset_from_days(0), ("no_reset", false));
        assert_eq!(reset_from_days(30), ("month", false));
        assert_eq!(reset_from_days(7), ("week", false));
        assert_eq!(reset_from_days(10), ("week", true));
        assert_eq!(reset_from_days(45), ("month", true));
        assert_eq!(reset_from_days(200), ("year", true));
        assert!(
            is_uuid("5783a3e7-e373-51cd-8642-c83782b807c5")
                && !is_uuid("hello")
                && !is_uuid("5783a3e7-e373-51cd-8642")
        );
    }

    #[test]
    fn reality_client_settings_move_up_and_certificates_are_found_inline_or_by_file() {
        let mut stream = json!({"security": "reality", "realitySettings": {"privateKey": "k", "settings": {"publicKey": "pub", "fingerprint": "chrome", "serverName": "x", "spiderX": ""}}});
        lift_reality(&mut stream);
        assert_eq!(stream["realitySettings"]["publicKey"], "pub");
        assert_eq!(stream["realitySettings"]["fingerprint"], "chrome");
        assert!(stream["realitySettings"].get("settings").is_none());
        assert!(
            stream["realitySettings"].get("spiderX").is_none(),
            "an empty value is not lifted"
        );

        let inline = json!({"tlsSettings": {"certificates": [{"certificate": ["-----BEGIN CERTIFICATE-----", "AAA", "-----END CERTIFICATE-----"], "key": ["-----BEGIN PRIVATE KEY-----", "BBB", "-----END PRIVATE KEY-----"]}]}});
        assert!(
            matches!(certificate_of(&inline), Some(PlannedCertificate::Inline { cert, .. }) if cert.contains("AAA"))
        );
        let files = json!({"tlsSettings": {"certificates": [{"certificateFile": "/a/fullchain.pem", "keyFile": "/a/key.pem"}]}});
        assert_eq!(
            certificate_of(&files),
            Some(PlannedCertificate::Files {
                cert: "/a/fullchain.pem".into(),
                key: "/a/key.pem".into()
            })
        );
        assert_eq!(
            certificate_of(&json!({"tlsSettings": {"certificates": []}})),
            None
        );
        assert_eq!(certificate_of(&json!({})), None);
    }
}
