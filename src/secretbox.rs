//! Encryption for secrets kept in rqlite (notification channel tokens, passwords) — ADR 0015.
//!
//! AES-256-GCM with a key from `ORCH_SECRET_KEY` (32 bytes, base64 or hex). Stored form:
//! `v1:<base64(nonce ‖ ciphertext)>`. The key type never prints itself.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rand::RngCore;

const PREFIX: &str = "v1:";
const NONCE_LEN: usize = 12;

#[derive(Clone)]
pub struct SecretKey([u8; 32]);

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(<redacted>)")
    }
}

impl SecretKey {
    /// Reads 32 bytes from base64 (standard or URL-safe) or 64 hex characters.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let s = raw.trim();
        let bytes = if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            hex::decode(s).map_err(|e| e.to_string())?
        } else {
            B64.decode(s)
                .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(s))
                .map_err(|_| "ORCH_SECRET_KEY is not base64 or hex".to_string())?
        };
        let key: [u8; 32] = bytes.try_into().map_err(|b: Vec<u8>| {
            format!(
                "ORCH_SECRET_KEY must be 32 bytes, got {} (generate one with `openssl rand -base64 32`)",
                b.len()
            )
        })?;
        Ok(Self(key))
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(&self.0).expect("32-byte key")
    }

    pub fn encrypt(&self, plain: &str) -> String {
        let mut nonce = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = self
            .cipher()
            .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
            .expect("AES-GCM encryption does not fail for in-memory input");
        let mut out = nonce.to_vec();
        out.extend(ct);
        format!("{PREFIX}{}", B64.encode(out))
    }

    pub fn decrypt(&self, stored: &str) -> Result<String, String> {
        let body = stored
            .strip_prefix(PREFIX)
            .ok_or_else(|| "stored secret has an unknown format".to_string())?;
        let raw = B64
            .decode(body)
            .map_err(|_| "stored secret is not valid base64".to_string())?;
        if raw.len() <= NONCE_LEN {
            return Err("stored secret is truncated".into());
        }
        let (nonce, ct) = raw.split_at(NONCE_LEN);
        let plain = self
            .cipher()
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| {
                "secret cannot be decrypted (ORCH_SECRET_KEY changed?); enter it again".to_string()
            })?;
        String::from_utf8(plain).map_err(|_| "decrypted secret is not UTF-8".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> SecretKey {
        SecretKey([byte; 32])
    }

    #[test]
    fn round_trips_and_uses_a_fresh_nonce_each_time() {
        let k = key(7);
        let a = k.encrypt("line-token");
        let b = k.encrypt("line-token");
        assert!(a.starts_with("v1:"));
        assert_ne!(a, b, "same plaintext must not give the same ciphertext");
        assert_eq!(k.decrypt(&a).unwrap(), "line-token");
        assert_eq!(k.decrypt(&b).unwrap(), "line-token");
    }

    #[test]
    fn a_different_key_or_tampered_value_is_refused() {
        let stored = key(1).encrypt("p@ss");
        assert!(key(2)
            .decrypt(&stored)
            .unwrap_err()
            .contains("cannot be decrypted"));
        let mut tampered = stored.clone();
        tampered.pop();
        tampered.push(if stored.ends_with('A') { 'B' } else { 'A' });
        assert!(key(1).decrypt(&tampered).is_err());
        assert!(key(1).decrypt("plain-text").is_err());
    }

    #[test]
    fn parses_base64_and_hex_and_rejects_wrong_lengths() {
        let b64 = B64.encode([9u8; 32]);
        assert!(SecretKey::parse(&b64).is_ok());
        assert!(SecretKey::parse(&"ab".repeat(32)).is_ok());
        assert!(SecretKey::parse(&B64.encode([1u8; 16]))
            .unwrap_err()
            .contains("32 bytes"));
        assert!(SecretKey::parse("not a key!").is_err());
    }

    #[test]
    fn debug_output_hides_the_key() {
        assert_eq!(format!("{:?}", key(3)), "SecretKey(<redacted>)");
    }
}
