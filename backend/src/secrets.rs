//! Encryption for the few stored secrets OIS must read back rather than verify (#605).
//!
//! Everything else credential-shaped in the schema is SHA-256 hashed (`0045_api_keys.sql`), because
//! OIS only ever needs to *check* it. The VATUSA webhook secret is different: HMAC-verifying a
//! delivery needs the key material itself, so it cannot be hashed — it is encrypted instead.
//!
//! XChaCha20-Poly1305 (RustCrypto, the family `sha2`/`hmac` already come from) with a fresh random
//! 24-byte nonce per encryption, so nonce reuse is not a practical concern. Stored as
//! `nonce ‖ ciphertext+tag` beside a `key_version`, so a future key can be introduced without guessing
//! which key wrote a row. There is deliberately no re-encryption path: the only secret stored this way
//! is disposable — if it can't be decrypted, the webhook is deleted and registered again.

use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};

/// The only key version so far. Bump it if `OIS_SECRET_KEY` is ever rotated in place.
pub const KEY_VERSION: i32 = 1;

const NONCE_LEN: usize = 24;

/// `nonce ‖ ciphertext+tag` for `plaintext` under `key`.
pub fn encrypt(key: &[u8; 32], plaintext: &str) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let sealed = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .expect("XChaCha20-Poly1305 encryption of an in-memory string cannot fail");
    let mut out = Vec::with_capacity(NONCE_LEN + sealed.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    out
}

/// The plaintext back, or `None` if `blob` is truncated, was tampered with, or was written under a
/// different key — all of which the caller handles the same way: discard and re-register.
pub fn decrypt(key: &[u8; 32], blob: &[u8]) -> Option<String> {
    if blob.len() < NONCE_LEN {
        return None;
    }
    let (nonce, sealed) = blob.split_at(NONCE_LEN);
    let opened = XChaCha20Poly1305::new(key.into())
        .decrypt(XNonce::from_slice(nonce), sealed)
        .ok()?;
    String::from_utf8(opened).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    /// The webhook receiver HMACs with `secret.as_bytes()`, so the decrypted string must be the
    /// stored one byte for byte — not merely equivalent.
    #[test]
    fn a_secret_round_trips_byte_identically() {
        let secret = "whsec_Zx9 ünïcode/+=\n tail ";
        let blob = encrypt(&KEY, secret);
        assert_eq!(decrypt(&KEY, &blob).unwrap().as_bytes(), secret.as_bytes());
    }

    #[test]
    fn the_stored_form_is_not_the_plaintext_and_is_never_reused() {
        let a = encrypt(&KEY, "same secret");
        let b = encrypt(&KEY, "same secret");
        assert_ne!(a, b, "a fresh nonce per encryption");
        assert!(!a.windows(11).any(|w| w == b"same secret"));
    }

    #[test]
    fn a_tampered_or_truncated_blob_is_refused() {
        let mut blob = encrypt(&KEY, "secret");
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert_eq!(decrypt(&KEY, &blob), None);
        assert_eq!(decrypt(&KEY, &[1, 2, 3]), None);
    }

    #[test]
    fn a_different_key_cannot_read_it() {
        let blob = encrypt(&KEY, "secret");
        assert_eq!(decrypt(&[8; 32], &blob), None);
    }
}
