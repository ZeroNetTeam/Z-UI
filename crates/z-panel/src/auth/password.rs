//! Passwords: Argon2id with its recommended parameters, and the rules a new
//! one must meet. Hashing is deliberately slow, so it runs off the async
//! workers.

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::Argon2;

use crate::error::ApiError;

pub const MIN_LENGTH: usize = 8;
const MAX_LENGTH: usize = 256;

/// Refuse a new password that is too short (or absurdly long).
pub fn check_strength(password: &str) -> Result<(), ApiError> {
    let length = password.chars().count();
    if length < MIN_LENGTH {
        return Err(ApiError::bad_request(
            "weak_password",
            format!("a password needs at least {MIN_LENGTH} characters"),
        ));
    }
    if length > MAX_LENGTH {
        return Err(ApiError::bad_request(
            "weak_password",
            format!("a password may have at most {MAX_LENGTH} characters"),
        ));
    }
    Ok(())
}

/// A username: 3 to 32 letters, digits, `.`, `_` or `-`.
pub fn check_username(username: &str) -> Result<(), ApiError> {
    let valid = (3..=32).contains(&username.len())
        && username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "invalid_username",
            "a username is 3 to 32 letters, digits, '.', '_' or '-'",
        ))
    }
}

pub async fn hash(password: &str) -> Result<String, ApiError> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(ApiError::internal)
    })
    .await
    .map_err(ApiError::internal)?
}

/// Whether `password` matches `hash`. A malformed hash matches nothing.
pub async fn verify(password: &str, hash: &str) -> bool {
    let (password, hash) = (password.to_owned(), hash.to_owned());
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).is_ok_and(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
    })
    .await
    .unwrap_or(false)
}

/// A hash to verify against when the username does not exist, so a wrong
/// username takes as long to refuse as a wrong password.
pub async fn decoy_hash() -> &'static str {
    static DECOY: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
    DECOY
        .get_or_init(|| async {
            hash("decoy-password-never-matches")
                .await
                .unwrap_or_default()
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_hash_verifies_its_password_only() {
        let hash = hash("correct horse").await.unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify("correct horse", &hash).await);
        assert!(!verify("correct horsf", &hash).await);
        assert!(!verify("correct horse", "not a hash").await);
    }

    #[test]
    fn rules_for_new_credentials() {
        assert!(check_strength("1234567").is_err());
        assert!(check_strength("12345678").is_ok());
        assert!(check_strength("گذرواژه‌ی بلند").is_ok(), "any script counts");
        assert!(check_username("ab").is_err());
        assert!(check_username("admin").is_ok());
        assert!(check_username("ad min").is_err());
        assert!(check_username("a.b_c-d").is_ok());
    }
}
