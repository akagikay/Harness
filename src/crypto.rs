//! At-rest encryption for stored credentials.
//!
//! Secrets are no longer written into `app_config.config_json` in plaintext.
//! Instead the harness splits them out and stores them in a separate
//! `app_secrets` row encrypted with AES-256-GCM.
//!
//! Key management (in order of precedence):
//! 1. `HARNESS_MASTER_KEY` — 64 hex chars (32 bytes) or base64 of 32 bytes.
//!    Useful for CI, containers, and headless deployments.
//! 2. `<db_dir>/harness.key` — 32 random bytes created on first use with
//!    owner-only permissions (0600 on Unix).
//!
//! Ciphertext format: `v1:` + base64(12-byte nonce || ciphertext || 16-byte tag).
//! A value without the `v1:` prefix is treated as legacy plaintext so existing
//! databases migrate transparently on first save.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::RngCore;
use std::path::{Path, PathBuf};

/// Marker prefix for encrypted payloads.
pub const CIPHER_PREFIX: &str = "v1:";
/// Key file name placed next to the SQLite database.
pub const KEY_FILE_NAME: &str = "harness.key";

#[derive(Debug)]
pub enum CryptoError {
    BadKey(String),
    Io(String),
    Encrypt(String),
    Decrypt(String),
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoError::BadKey(m) => write!(f, "invalid master key: {m}"),
            CryptoError::Io(m) => write!(f, "key file error: {m}"),
            CryptoError::Encrypt(m) => write!(f, "encryption failed: {m}"),
            CryptoError::Decrypt(m) => write!(f, "decryption failed: {m}"),
        }
    }
}

impl std::error::Error for CryptoError {}

/// A 32-byte AES-256 master key.
#[derive(Clone)]
pub struct MasterKey([u8; 32]);

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print key material.
        f.write_str("MasterKey(<redacted>)")
    }
}

impl MasterKey {
    /// Load from `HARNESS_MASTER_KEY`, else from `<db_dir>/harness.key`
    /// (creating it on first use).
    pub fn load_or_create(db_dir: &Path) -> Result<Self, CryptoError> {
        if let Ok(raw) = std::env::var("HARNESS_MASTER_KEY") {
            if !raw.trim().is_empty() {
                return Self::from_encoded(raw.trim());
            }
        }
        let path = db_dir.join(KEY_FILE_NAME);
        if path.exists() {
            let bytes = std::fs::read(&path).map_err(|e| CryptoError::Io(e.to_string()))?;
            return Self::from_bytes(&bytes);
        }
        let key = Self::generate();
        write_key_file(&path, key.as_bytes())?;
        Ok(key)
    }

    /// Parse a hex (64 chars) or base64 (32 bytes) key.
    pub fn from_encoded(raw: &str) -> Result<Self, CryptoError> {
        let bytes = if raw.len() == 64 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
            hex::decode(raw).map_err(|e| CryptoError::BadKey(e.to_string()))?
        } else {
            B64.decode(raw.trim())
                .map_err(|_| CryptoError::BadKey("expected 64 hex chars or base64 of 32 bytes".into()))?
        };
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() != 32 {
            return Err(CryptoError::BadKey(format!(
                "expected 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(bytes);
        Ok(Self(out))
    }

    /// Generate a fresh random key.
    pub fn generate() -> Self {
        let mut buf = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut buf);
        Self(buf)
    }

    fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    fn cipher(&self) -> Aes256Gcm {
        let key = Key::<Aes256Gcm>::from_slice(self.as_bytes());
        Aes256Gcm::new(key)
    }

    /// Encrypt to `v1:<base64>`.
    pub fn encrypt(&self, plaintext: &str) -> Result<String, CryptoError> {
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = self
            .cipher()
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| CryptoError::Encrypt(e.to_string()))?;
        let mut payload = Vec::with_capacity(12 + ct.len());
        payload.extend_from_slice(&nonce_bytes);
        payload.extend_from_slice(&ct);
        Ok(format!("{CIPHER_PREFIX}{}", B64.encode(payload)))
    }

    /// Decrypt a `v1:` payload. Legacy plaintext (no prefix) is returned as-is.
    pub fn decrypt(&self, stored: &str) -> Result<String, CryptoError> {
        let Some(body) = stored.strip_prefix(CIPHER_PREFIX) else {
            return Ok(stored.to_string());
        };
        let raw = B64
            .decode(body)
            .map_err(|e| CryptoError::Decrypt(format!("bad base64: {e}")))?;
        if raw.len() < 12 + 16 {
            return Err(CryptoError::Decrypt("payload too short".into()));
        }
        let (nonce_bytes, ct) = raw.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let pt = self
            .cipher()
            .decrypt(nonce, ct)
            .map_err(|_| CryptoError::Decrypt("authentication failed (wrong key?)".into()))?;
        String::from_utf8(pt).map_err(|e| CryptoError::Decrypt(e.to_string()))
    }
}

fn write_key_file(path: &PathBuf, bytes: &[u8; 32]) -> Result<(), CryptoError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CryptoError::Io(e.to_string()))?;
    }
    std::fs::write(path, bytes).map_err(|e| CryptoError::Io(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Resolve the directory that holds the database, for key placement.
pub fn db_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// SHA-256 of a byte slice, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_encrypt_decrypt() {
        let key = MasterKey::generate();
        let secret = r#"{"api_key":"sk-live-123","nested":{"token":"abc"}}"#;
        let enc = key.encrypt(secret).unwrap();
        assert!(enc.starts_with(CIPHER_PREFIX));
        assert!(!enc.contains("sk-live-123"), "ciphertext must not contain plaintext");
        assert_eq!(key.decrypt(&enc).unwrap(), secret);
    }

    #[test]
    fn nonce_is_unique_per_encryption() {
        let key = MasterKey::generate();
        let a = key.encrypt("same").unwrap();
        let b = key.encrypt("same").unwrap();
        assert_ne!(a, b, "each encryption must use a fresh nonce");
    }

    #[test]
    fn wrong_key_fails_authentication() {
        let k1 = MasterKey::generate();
        let k2 = MasterKey::generate();
        let enc = k1.encrypt("secret").unwrap();
        assert!(k2.decrypt(&enc).is_err(), "GCM must reject a wrong key");
    }

    #[test]
    fn legacy_plaintext_passes_through() {
        let key = MasterKey::generate();
        assert_eq!(key.decrypt("plain-old-value").unwrap(), "plain-old-value");
    }

    #[test]
    fn key_from_hex_and_base64() {
        let bytes = [7u8; 32];
        let hex_key = hex::encode(bytes);
        assert_eq!(MasterKey::from_encoded(&hex_key).unwrap().as_bytes(), &bytes);
        let b64 = B64.encode(bytes);
        assert_eq!(MasterKey::from_encoded(&b64).unwrap().as_bytes(), &bytes);
        assert!(MasterKey::from_encoded("too-short").is_err());
    }

    #[test]
    fn key_file_is_created_and_reloaded() {
        let dir = std::env::temp_dir().join(format!("harness-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let k1 = MasterKey::load_or_create(&dir).unwrap();
        let enc = k1.encrypt("hello").unwrap();
        let k2 = MasterKey::load_or_create(&dir).unwrap();
        assert_eq!(k2.decrypt(&enc).unwrap(), "hello");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
