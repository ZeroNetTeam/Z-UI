//! The subscription links of Marzban and PasarGuard are not tokens kept in a
//! table: a link is a claim ("this is user 17, made at that time") signed
//! with the panel's key. After an import from one of them the key is kept
//! here, and a link that is signed with it, and made after the user was made
//! and last revoked, shows the user it names. Three kinds of link were issued
//! over the years, and all three are read:
//!
//!  * a JWT (`eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.…`) naming the username;
//!  * the newer `<base64 of "v3,id,time">.<HMAC-SHA256 of it>`;
//!  * the older `<base64 of "username,time">` followed by ten characters of a
//!    SHA-256 of it and the key.
//!
//! This follows PasarGuard's own check (`get_subscription_payload`), whose
//! tokens these all are.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::{Digest, Sha256};
use z_db::{Database, DbError, LegacyRef, User};

/// What the old panels' links are kept under in the database.
pub const KIND: &str = "marz";

const JWT_HEADER: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.";

/// Who a link names, and when it was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub who: Who,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Who {
    Id(i64),
    Name(String),
}

type HmacSha256 = Hmac<Sha256>;

fn unb64(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).ok()
}

/// The data inside a link: `v2|v3,id,time` or `username,time`.
fn claim_of(data: &str) -> Option<Claim> {
    let parts: Vec<&str> = data.split(',').collect();
    match parts.as_slice() {
        [version @ ("v2" | "v3"), id, created] => {
            let _ = version;
            Some(Claim {
                who: Who::Id(id.parse().ok()?),
                created_at: created.parse().ok()?,
            })
        }
        [name, created] => Some(Claim {
            who: Who::Name((*name).to_string()),
            created_at: created.parse().ok()?,
        }),
        _ => None,
    }
}

/// Whether `token` is a link signed with `secret`, and what it claims.
pub fn parse(token: &str, secret: &str, now: i64) -> Option<Claim> {
    if token.len() < 15 {
        return None;
    }
    if token.starts_with(JWT_HEADER) {
        let (signed, signature) = token.rsplit_once('.')?;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).ok()?;
        mac.update(signed.as_bytes());
        mac.verify_slice(&unb64(signature)?).ok()?;
        let payload: Value = serde_json::from_slice(&unb64(signed.split('.').nth(1)?)?).ok()?;
        if payload["access"] != "subscription" {
            return None;
        }
        // A token that says it expires, does.
        if payload["exp"].as_f64().is_some_and(|exp| exp < now as f64) {
            return None;
        }
        return Some(Claim {
            who: Who::Name(
                payload["sub"]
                    .as_str()
                    .filter(|s| !s.is_empty())?
                    .to_string(),
            ),
            created_at: payload["iat"].as_f64()? as i64,
        });
    }
    if let Some((data, signature)) = token.rsplit_once('.') {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).ok()?;
        mac.update(data.as_bytes());
        mac.verify_slice(&unb64(signature)?).ok()?;
        return claim_of(&String::from_utf8(unb64(data)?).ok()?);
    }
    // The older kind: the last ten characters sign the rest.
    let split = token
        .len()
        .checked_sub(10)
        .filter(|n| token.is_char_boundary(*n))?;
    let (data, signature) = token.split_at(split);
    let decoded = String::from_utf8(unb64(data)?).ok()?;
    let digest = Sha256::digest(format!("{data}{secret}").as_bytes());
    let in_base64: String = base64::engine::general_purpose::URL_SAFE
        .encode(digest)
        .chars()
        .take(10)
        .collect();
    let in_hex: String = hex::encode(digest).chars().take(10).collect();
    let signed = [in_base64, in_hex].iter().any(|expected| {
        // Compared without stopping at the first difference.
        expected.len() == signature.len()
            && expected
                .bytes()
                .zip(signature.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    });
    if signed {
        claim_of(&decoded)
    } else {
        None
    }
}

/// The user a link of the old panel names, if the panel was imported and the
/// link is good.
pub async fn resolve(db: &Database, token: &str) -> Result<Option<User>, DbError> {
    let Some(secret) = db.legacy_secret(KIND).await? else {
        return Ok(None);
    };
    let Some(claim) = parse(token, &secret, z_db::now()) else {
        return Ok(None);
    };
    let found = match &claim.who {
        Who::Id(id) => db.user_by_legacy(KIND, LegacyRef::Id(*id)).await?,
        Who::Name(name) => db.user_by_legacy(KIND, LegacyRef::Username(name)).await?,
    };
    Ok(found
        .filter(|(_, valid_after)| claim.created_at >= *valid_after)
        .map(|(user, _)| user))
}

/// The user a link names: a token this panel gave, or one of an old panel's.
pub async fn find_user(db: &Database, token: &str) -> Result<Option<User>, DbError> {
    match db.user_by_token(token).await? {
        Some(user) => Ok(Some(user)),
        None => resolve(db, token).await,
    }
}
