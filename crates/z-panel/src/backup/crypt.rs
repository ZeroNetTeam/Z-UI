//! A passphrase over a backup: XChaCha20-Poly1305, its key made from the
//! passphrase with Argon2id (slow on purpose, so guessing is slow).
//!
//! An encrypted backup is `ZUIE1`, a 16-byte salt, a 24-byte nonce and then
//! the sealed bytes. The magic and the salt are authenticated too, so a
//! changed header is as much a failure as a changed body.

use argon2::Argon2;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

const MAGIC: &[u8; 5] = b"ZUIE1";
const SALT: usize = 16;
const NONCE: usize = 24;

#[derive(Debug, PartialEq, Eq)]
pub enum CryptError {
    /// The file is not an encrypted backup at all.
    NotEncrypted,
    /// The passphrase is wrong, or the file was changed or cut short.
    WrongOrDamaged,
}

/// Whether `bytes` is an encrypted backup (a plain one is a gzip file).
pub fn is_encrypted(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

fn key(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .expect("Argon2 takes any passphrase and a 16-byte salt");
    key
}

pub fn encrypt(passphrase: &str, data: &[u8]) -> Vec<u8> {
    let salt: [u8; SALT] = rand::random();
    let nonce: [u8; NONCE] = rand::random();
    let mut out = Vec::with_capacity(MAGIC.len() + SALT + NONCE + data.len() + 16);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    let aad = out.clone();
    out.extend_from_slice(&nonce);
    let cipher = XChaCha20Poly1305::new((&key(passphrase, &salt)).into());
    let sealed = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: data,
                aad: &aad,
            },
        )
        .expect("sealing in memory does not fail");
    out.extend_from_slice(&sealed);
    out
}

pub fn decrypt(passphrase: &str, bytes: &[u8]) -> Result<Vec<u8>, CryptError> {
    if !is_encrypted(bytes) {
        return Err(CryptError::NotEncrypted);
    }
    let header = MAGIC.len() + SALT;
    if bytes.len() < header + NONCE + 16 {
        return Err(CryptError::WrongOrDamaged);
    }
    let (aad, rest) = bytes.split_at(header);
    let (nonce, sealed) = rest.split_at(NONCE);
    let cipher = XChaCha20Poly1305::new((&key(passphrase, &aad[MAGIC.len()..])).into());
    cipher
        .decrypt(XNonce::from_slice(nonce), Payload { msg: sealed, aad })
        .map_err(|_| CryptError::WrongOrDamaged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_sealed_opens_with_the_passphrase_and_only_with_it() {
        let sealed = encrypt("correct horse", b"the whole panel");
        assert!(is_encrypted(&sealed) && !is_encrypted(b"\x1f\x8b gzip"));
        assert!(
            !sealed.windows(5).any(|w| w == b"panel"),
            "nothing readable is left"
        );
        assert_eq!(
            decrypt("correct horse", &sealed).unwrap(),
            b"the whole panel"
        );
        assert_eq!(
            decrypt("wrong horse", &sealed),
            Err(CryptError::WrongOrDamaged)
        );
        assert_eq!(
            decrypt("correct horse", b"plain"),
            Err(CryptError::NotEncrypted)
        );
        // Two seals of one thing differ: salt and nonce are fresh.
        assert_ne!(
            encrypt("correct horse", b"x"),
            encrypt("correct horse", b"x")
        );
    }

    #[test]
    fn any_change_to_the_file_is_noticed() {
        let sealed = encrypt("pass phrase", b"data to protect");
        for at in [0, 5, 10, 21, 30, sealed.len() - 1] {
            let mut changed = sealed.clone();
            changed[at] ^= 1;
            assert!(decrypt("pass phrase", &changed).is_err(), "byte {at}");
        }
        assert_eq!(
            decrypt("pass phrase", &sealed[..sealed.len() - 3]),
            Err(CryptError::WrongOrDamaged)
        );
        assert_eq!(
            decrypt("pass phrase", &sealed[..10]),
            Err(CryptError::WrongOrDamaged)
        );
        assert_eq!(
            decrypt("pass phrase", b"ZUIE1"),
            Err(CryptError::WrongOrDamaged)
        );
    }
}
