//! Signed-in sessions: a short JWT in an `HttpOnly`, `SameSite=Strict`
//! cookie scoped to the panel's path. The token names the admin and the
//! `token_version` it was issued under, so raising that version (a new
//! password, 2FA turned off) ends every older session at once.

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

pub const COOKIE: &str = "z_ui_session";
/// How long a sign-in lasts.
pub const LIFETIME_SECONDS: u64 = 24 * 60 * 60;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// The admin's id.
    pub sub: i64,
    /// The admin's `token_version` when the session was issued.
    pub ver: i64,
    pub iat: u64,
    pub exp: u64,
}

/// The key sessions are signed with: random, kept in the database, so a
/// restart does not sign everyone out.
#[derive(Clone)]
pub struct SessionKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

impl SessionKeys {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
        }
    }

    pub fn issue(&self, admin_id: i64, token_version: i64, now: u64) -> String {
        let claims = Claims {
            sub: admin_id,
            ver: token_version,
            iat: now,
            exp: now + LIFETIME_SECONDS,
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .expect("HS256 signing does not fail")
    }

    pub fn verify(&self, token: &str) -> Option<Claims> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.leeway = 0;
        decode::<Claims>(token, &self.decoding, &validation)
            .ok()
            .map(|data| data.claims)
    }
}

/// The `Set-Cookie` value that signs in, for a panel under `path`.
pub fn cookie(token: &str, path: &str, secure: bool) -> String {
    format!(
        "{COOKIE}={token}; Path={path}; HttpOnly; SameSite=Strict; Max-Age={LIFETIME_SECONDS}{}",
        if secure { "; Secure" } else { "" }
    )
}

/// The `Set-Cookie` value that signs out.
pub fn clear_cookie(path: &str, secure: bool) -> String {
    format!(
        "{COOKIE}=; Path={path}; HttpOnly; SameSite=Strict; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

/// The session token in a request's `Cookie` header, if any.
pub fn from_cookie_header(header: &str) -> Option<&str> {
    header
        .split(';')
        .map(str::trim)
        .find_map(|pair| pair.strip_prefix(COOKIE)?.strip_prefix('='))
        .filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_verifies_with_its_key_only_and_expires() {
        let keys = SessionKeys::new(b"0123456789abcdef0123456789abcdef");
        let now = super::super::totp::now();
        let token = keys.issue(7, 3, now);
        let claims = keys.verify(&token).unwrap();
        assert_eq!((claims.sub, claims.ver), (7, 3));
        assert!(SessionKeys::new(b"another key, another signature..")
            .verify(&token)
            .is_none());
        let old = keys.issue(7, 3, now - LIFETIME_SECONDS - 10);
        assert!(keys.verify(&old).is_none(), "expired");
        assert!(keys.verify("garbage").is_none());
    }

    #[test]
    fn the_cookie_is_strict_and_found_again() {
        let cookie = cookie("abc", "/secret", true);
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Path=/secret") && cookie.ends_with("; Secure"));
        assert_eq!(
            from_cookie_header("theme=dark; z_ui_session=abc; x=1"),
            Some("abc")
        );
        assert_eq!(from_cookie_header("z_ui_session_x=abc"), None);
        assert_eq!(from_cookie_header("z_ui_session="), None);
    }
}
