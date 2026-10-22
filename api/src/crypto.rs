//! Encryption at rest for secrets stored in Postgres (the Google tokens).
//!
//! A sealed blob is `0x01 ‖ nonce(24) ‖ ciphertext+tag`, made with XChaCha20-Poly1305
//! under a 32-byte [`TokenKey`]. The leading version byte leaves room for key rotation.
//! Callers pass the column's identity as the AAD, so a blob can't be moved to another
//! column and still open.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

/// The blob format version this module writes and reads.
const VERSION: u8 = 1;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

/// The 32-byte key that seals stored tokens (`WAYPOINT_TOKEN_KEY`). Its `Debug` never
/// shows the key, its bytes are wiped on drop, and `==` compares in constant time.
#[derive(Clone)]
pub struct TokenKey([u8; 32]);

impl Drop for TokenKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl PartialEq for TokenKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.ct_eq(&other.0).into()
    }
}

impl Eq for TokenKey {}

impl TokenKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Decodes standard base64 (as `openssl rand -base64 32` prints it) into a key. The
    /// error never contains the value.
    pub fn from_base64(value: &str) -> Result<Self, String> {
        let bytes = STANDARD
            .decode(value.trim())
            .map_err(|_| "is not valid base64".to_string())?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|bytes: Vec<u8>| format!("must decode to 32 bytes, got {}", bytes.len()))?;
        Ok(Self(bytes))
    }
}

impl fmt::Debug for TokenKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenKey(<redacted>)")
    }
}

/// Why a blob didn't open. Neither variant says anything about the contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    /// Too short, or an unknown version byte.
    Malformed,
    /// The tag didn't verify: wrong key, wrong AAD or a modified blob.
    Invalid,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "the sealed value is malformed",
            Self::Invalid => "the sealed value does not decrypt with this key",
        })
    }
}

impl std::error::Error for CryptoError {}

/// Encrypts `plaintext` under `key` with a fresh random nonce, binding `aad`.
pub fn seal(key: &TokenKey, aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).expect("the OS random number generator is available");
    let cipher = XChaCha20Poly1305::new((&key.0).into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("XChaCha20-Poly1305 encryption does not fail for in-memory buffers");
    let mut blob = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
    blob.push(VERSION);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    blob
}

/// Decrypts a blob made by [`seal`] with the same `key` and `aad`.
pub fn open(key: &TokenKey, aad: &[u8], blob: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if blob.len() < 1 + NONCE_LEN + TAG_LEN || blob[0] != VERSION {
        return Err(CryptoError::Malformed);
    }
    let (nonce, ciphertext) = blob[1..].split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new((&key.0).into());
    cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| CryptoError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AAD: &[u8] = b"google_accounts.refresh_token";

    fn key(byte: u8) -> TokenKey {
        TokenKey::new([byte; 32])
    }

    #[test]
    fn round_trip() {
        let blob = seal(&key(7), AAD, b"1//refresh-token");
        assert_eq!(blob[0], VERSION);
        assert_eq!(open(&key(7), AAD, &blob).unwrap(), b"1//refresh-token");
        assert_eq!(open(&key(7), AAD, &seal(&key(7), AAD, b"")).unwrap(), b"");
    }

    #[test]
    fn rejects_tampering_wrong_aad_and_wrong_key() {
        let blob = seal(&key(7), AAD, b"secret");
        for i in 0..blob.len() {
            let mut tampered = blob.clone();
            tampered[i] ^= 0x01;
            assert!(open(&key(7), AAD, &tampered).is_err(), "byte {i} flipped");
        }
        assert_eq!(
            open(&key(7), b"google_accounts.access_token", &blob),
            Err(CryptoError::Invalid)
        );
        assert_eq!(open(&key(8), AAD, &blob), Err(CryptoError::Invalid));
        assert_eq!(open(&key(7), AAD, &blob[..20]), Err(CryptoError::Malformed));
    }

    #[test]
    fn nonces_differ() {
        let a = seal(&key(7), AAD, b"same");
        let b = seal(&key(7), AAD, b"same");
        assert_ne!(a, b);
        assert_ne!(a[1..1 + NONCE_LEN], b[1..1 + NONCE_LEN]);
    }

    #[test]
    fn token_key_parses_base64_and_redacts() {
        let encoded = STANDARD.encode([9u8; 32]);
        let parsed = TokenKey::from_base64(&encoded).unwrap();
        assert_eq!(parsed, key(9));
        assert!(!format!("{parsed:?}").contains("9"));
        assert!(TokenKey::from_base64("not base64!").is_err());
        let short = TokenKey::from_base64(&STANDARD.encode([1u8; 16])).unwrap_err();
        assert!(short.contains("32 bytes"), "{short}");
    }
}
