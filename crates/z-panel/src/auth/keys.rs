//! API keys: how they are made and recognised, and which addresses may use
//! them. A key is 32 random bytes, so a plain SHA-256 is all that is needed
//! to keep it (there is nothing to guess, unlike a password).

use std::net::IpAddr;

use base64::Engine;
use sha2::{Digest, Sha256};

/// Every key starts with this, so a leaked one is easy to spot and report.
pub const PREFIX: &str = "zk_";
/// How many characters of a key are kept in the clear to tell keys apart.
const SHOWN: usize = PREFIX.len() + 8;

/// A fresh key and what is kept of it: `(key, prefix, hash)`.
pub fn generate() -> (String, String, String) {
    let bytes: [u8; 32] = rand::random();
    let key = format!(
        "{PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    );
    let prefix = key.chars().take(SHOWN).collect();
    let hash = hash(&key);
    (key, prefix, hash)
}

/// What the database keeps of a key.
pub fn hash(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

/// The key in an `Authorization: Bearer` or `X-API-Key` header.
pub fn from_headers(headers: &axum::http::HeaderMap) -> Option<&str> {
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            let (scheme, token) = value.split_once(' ')?;
            scheme
                .eq_ignore_ascii_case("bearer")
                .then_some(token.trim())
        });
    bearer
        .or_else(|| {
            headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
        })
        .filter(|key| !key.is_empty())
}

/// Parse a list of addresses and CIDR ranges (`203.0.113.7`,
/// `10.0.0.0/8`, `2001:db8::/32`), refusing what is not one.
pub fn parse_ranges(list: &[String]) -> Result<Vec<String>, String> {
    let mut ranges = Vec::new();
    for entry in list
        .iter()
        .map(|entry| entry.trim())
        .filter(|e| !e.is_empty())
    {
        Range::parse(entry).ok_or_else(|| format!("{entry:?} is not an address or range"))?;
        ranges.push(entry.to_owned());
    }
    ranges.dedup();
    Ok(ranges)
}

/// Whether `address` is in the comma-separated `list`; an empty list allows
/// every address.
pub fn allows(list: &str, address: IpAddr) -> bool {
    let entries: Vec<&str> = list
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    entries.is_empty()
        || entries
            .iter()
            .filter_map(|entry| Range::parse(entry))
            .any(|range| range.contains(address))
}

/// A network: an address and how many leading bits of it matter.
struct Range {
    network: u128,
    bits: u32,
    v6: bool,
}

impl Range {
    fn parse(text: &str) -> Option<Self> {
        let (address, length) = match text.split_once('/') {
            Some((address, length)) => (address, Some(length.parse::<u32>().ok()?)),
            None => (text, None),
        };
        let address: IpAddr = address.parse().ok()?;
        let (value, width, v6) = widen(address);
        let bits = length.unwrap_or(width);
        if bits > width {
            return None;
        }
        Some(Self {
            network: value & mask(bits, width),
            bits,
            v6,
        })
    }

    fn contains(&self, address: IpAddr) -> bool {
        let (value, width, v6) = widen(address);
        v6 == self.v6 && value & mask(self.bits, width) == self.network
    }
}

/// An address as a number, how many bits it has, and whether it is IPv6.
/// An IPv4 address that arrived inside IPv6 counts as IPv4.
fn widen(address: IpAddr) -> (u128, u32, bool) {
    match address.to_canonical() {
        IpAddr::V4(v4) => (u128::from(u32::from(v4)), 32, false),
        IpAddr::V6(v6) => (u128::from(v6), 128, true),
    }
}

fn mask(bits: u32, width: u32) -> u128 {
    if bits == 0 {
        0
    } else {
        (u128::MAX << (128 - bits)) >> (128 - width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn a_key_is_random_prefixed_and_only_its_hash_is_kept() {
        let (key, prefix, hashed) = generate();
        assert!(key.starts_with("zk_") && key.len() == 3 + 43);
        assert!(key.starts_with(&prefix) && prefix.len() == 11);
        assert_eq!(hashed, hash(&key));
        assert_eq!(hashed.len(), 64);
        assert!(!hashed.contains(&key[3..]));
        assert_ne!(generate().0, key);
        assert_ne!(hash("zk_a"), hash("zk_b"));
    }

    #[test]
    fn a_key_is_read_from_either_header() {
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(from_headers(&headers), None);
        headers.insert("authorization", "Bearer zk_abc".parse().unwrap());
        assert_eq!(from_headers(&headers), Some("zk_abc"));
        headers.insert("authorization", "bearer   zk_abc ".parse().unwrap());
        assert_eq!(from_headers(&headers), Some("zk_abc"));
        headers.insert("authorization", "Basic dXNlcjpwYXNz".parse().unwrap());
        assert_eq!(from_headers(&headers), None, "not a bearer token");
        headers.insert("x-api-key", "zk_def".parse().unwrap());
        assert_eq!(from_headers(&headers), Some("zk_def"));
        headers.insert("authorization", "Bearer ".parse().unwrap());
        headers.remove("x-api-key");
        assert_eq!(from_headers(&headers), None);
    }

    #[test]
    fn ranges_match_addresses_in_both_families() {
        assert!(allows("", ip("203.0.113.9")), "no list, no limit");
        assert!(allows("203.0.113.9", ip("203.0.113.9")));
        assert!(!allows("203.0.113.9", ip("203.0.113.10")));
        assert!(allows("10.0.0.0/8, 192.168.1.0/24", ip("10.200.3.4")));
        assert!(allows("10.0.0.0/8, 192.168.1.0/24", ip("192.168.1.77")));
        assert!(!allows("10.0.0.0/8, 192.168.1.0/24", ip("192.168.2.1")));
        assert!(allows("0.0.0.0/0", ip("8.8.8.8")));
        assert!(allows("2001:db8::/32", ip("2001:db8:1::5")));
        assert!(!allows("2001:db8::/32", ip("2001:db9::1")));
        assert!(!allows("10.0.0.0/8", ip("::1")), "families do not mix");
        assert!(
            allows("203.0.113.9", ip("::ffff:203.0.113.9")),
            "an IPv4 address inside IPv6 is still that address"
        );
        assert!(
            !allows("garbage", ip("1.2.3.4")),
            "an unreadable list allows nobody"
        );
    }

    #[test]
    fn a_list_from_a_client_is_checked_before_it_is_kept() {
        let ok = parse_ranges(&[" 10.0.0.0/8 ".into(), "".into(), "::1".into()]).unwrap();
        assert_eq!(ok, ["10.0.0.0/8", "::1"]);
        for bad in [
            "10.0.0.0/33",
            "300.1.1.1",
            "host.example",
            "10.0.0.0/x",
            "::1/129",
        ] {
            assert!(parse_ranges(&[bad.into()]).is_err(), "{bad}");
        }
    }
}
