//! The link that lets an admin brought over from another panel set a
//! password. The token is 256 random bits shown once; only its hash is kept.

use sha2::{Digest, Sha256};

use crate::error::ApiError;

/// How long a link is good for.
pub const VALID_FOR: i64 = 7 * 86_400;

pub fn new_token() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}

pub fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.trim().as_bytes()))
}

/// A new link for `admin_id`: the token, to be given to the admin; any older
/// link of theirs stops working.
pub async fn issue(db: &z_db::Database, admin_id: i64) -> Result<String, ApiError> {
    let token = new_token();
    db.create_invite(admin_id, &hash(&token), z_db::now() + VALID_FOR)
        .await?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_long_unique_and_known_only_by_its_hash() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_ne!(hash(&a), a);
        assert_eq!(
            hash(&a),
            hash(&format!(" {a}\n")),
            "stray spaces do not matter"
        );
        assert_ne!(hash(&a), hash(&b));
    }
}
