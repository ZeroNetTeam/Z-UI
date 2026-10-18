//! Time-based one-time passwords (RFC 6238): the six digits an
//! authenticator app shows, with SHA-1 and 30-second steps, which every app
//! supports.

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha1::Sha1;

const STEP: u64 = 30;
const DIGITS: u32 = 6;
/// Codes one step either side are accepted: a phone a little out of time,
/// or a code typed as it rolled over.
const SKEW: i64 = 1;

/// A new secret: 20 random bytes, written in base32 as the apps take it.
pub fn new_secret() -> String {
    let mut bytes = [0u8; 20];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base32(&bytes)
}

/// The `otpauth://` link an authenticator app scans.
pub fn otpauth_url(secret: &str, issuer: &str, account: &str) -> String {
    let issuer = percent(issuer);
    format!(
        "otpauth://totp/{issuer}:{account}?secret={secret}&issuer={issuer}&algorithm=SHA1&digits={DIGITS}&period={STEP}",
        account = percent(account)
    )
}

/// Whether `code` is valid for `secret` at `unix_seconds`.
pub fn verify(secret: &str, code: &str, unix_seconds: u64) -> bool {
    let code = code.trim().replace(' ', "");
    let Some(key) = base32_decode(secret) else {
        return false;
    };
    if code.len() != DIGITS as usize || !code.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let counter = (unix_seconds / STEP) as i64;
    (-SKEW..=SKEW).any(|offset| {
        let at = counter + offset;
        at >= 0 && constant_time_eq(hotp(&key, at as u64).as_bytes(), code.as_bytes())
    })
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The code an authenticator app shows for `secret` now.
#[cfg(test)]
pub fn current_code(secret: &str) -> String {
    hotp(&base32_decode(secret).expect("a base32 secret"), now() / STEP)
}

/// RFC 4226's HOTP for one counter value.
fn hotp(key: &[u8], counter: u64) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let binary = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    format!(
        "{:0width$}",
        binary % 10u32.pow(DIGITS),
        width = DIGITS as usize
    )
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn base32_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in text
        .trim_end_matches('=')
        .chars()
        .filter(|c| !c.is_whitespace())
    {
        let value = ALPHABET
            .iter()
            .position(|&a| a == c.to_ascii_uppercase() as u8)? as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    (!out.is_empty()).then_some(out)
}

fn percent(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 appendix B, SHA-1, keeping the last six digits.
    #[test]
    fn rfc_6238_vectors() {
        let secret = base32(b"12345678901234567890");
        for (time, code) in [
            (59, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
        ] {
            assert!(verify(&secret, code, time), "{time}");
        }
    }

    #[test]
    fn a_code_from_another_window_or_secret_is_refused() {
        let secret = base32(b"12345678901234567890");
        assert!(
            verify(&secret, "287082", 59 + 30),
            "one step late is allowed"
        );
        assert!(
            !verify(&secret, "287082", 59 + 90),
            "three steps late is not"
        );
        assert!(!verify(&new_secret(), "287082", 59));
        assert!(!verify(&secret, "28708", 59));
        assert!(!verify(&secret, "abcdef", 59));
        assert!(verify(&secret, " 287 082 ", 59), "spaces are ignored");
    }

    #[test]
    fn secrets_round_trip_through_base32() {
        let secret = new_secret();
        assert_eq!(secret.len(), 32);
        assert_eq!(base32_decode(&secret).unwrap().len(), 20);
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
    }

    #[test]
    fn the_link_names_the_issuer_and_account() {
        let url = otpauth_url("ABC", "Z-UI", "my admin");
        assert_eq!(
            url,
            "otpauth://totp/Z-UI:my%20admin?secret=ABC&issuer=Z-UI&algorithm=SHA1&digits=6&period=30"
        );
    }
}
