//! Encryption of stored secrets (SNMP communities and passwords) with XChaCha20-Poly1305.
//!
//! Blob format: `0x01 || nonce (24 bytes) || ciphertext+tag`. The version byte allows a later key or
//! algorithm rotation. A context string is authenticated as associated data, so a ciphertext cannot be
//! moved from one column to another.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, Generate, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand::RngCore;

const FORMAT_V1: u8 = 1;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("secret key file {path}: {reason}")]
    KeyFile { path: PathBuf, reason: String },
    #[error("encryption failed")]
    Encrypt,
    #[error("decryption failed (wrong key, wrong context or corrupted data)")]
    Decrypt,
}

/// Holds the encryption key. Never prints or serializes it.
pub struct SecretBox {
    cipher: XChaCha20Poly1305,
}

impl std::fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBox(***)")
    }
}

impl SecretBox {
    pub fn from_key(key: [u8; KEY_LEN]) -> Self {
        Self {
            cipher: XChaCha20Poly1305::new(&Key::from(key)),
        }
    }

    /// Loads a key file (64 hex characters). Refuses files readable by group or others.
    pub fn from_key_file(path: &Path) -> Result<Self, CryptoError> {
        let err = |reason: String| CryptoError::KeyFile {
            path: path.to_owned(),
            reason,
        };
        let meta = fs::metadata(path).map_err(|e| err(format!("cannot read: {e}")))?;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(err(
                "must not be accessible by group or others (run `chmod 600`)".to_owned(),
            ));
        }
        let text = fs::read_to_string(path).map_err(|e| err(format!("cannot read: {e}")))?;
        let bytes = decode_hex(text.trim())
            .filter(|b| b.len() == KEY_LEN)
            .ok_or_else(|| err("expected 64 hexadecimal characters".to_owned()))?;
        let mut key = [0u8; KEY_LEN];
        key.copy_from_slice(&bytes);
        Ok(Self::from_key(key))
    }

    /// Creates a new random key file with mode 0600. Never overwrites an existing file.
    pub fn generate_key_file(path: &Path) -> Result<(), CryptoError> {
        let err = |reason: String| CryptoError::KeyFile {
            path: path.to_owned(),
            reason,
        };
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| err(format!("cannot create directory: {e}")))?;
        }
        let mut key = [0u8; KEY_LEN];
        rand::rng().fill_bytes(&mut key);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| {
                err(format!(
                    "cannot create (an existing file is never overwritten): {e}"
                ))
            })?;
        writeln!(file, "{}", crate::auth::hex(&key))
            .map_err(|e| err(format!("cannot write: {e}")))?;
        Ok(())
    }

    pub fn encrypt(&self, plaintext: &str, context: &str) -> Result<Vec<u8>, CryptoError> {
        let nonce = XNonce::generate();
        let ciphertext = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| CryptoError::Encrypt)?;
        let mut blob = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        blob.push(FORMAT_V1);
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(blob)
    }

    pub fn decrypt(&self, blob: &[u8], context: &str) -> Result<String, CryptoError> {
        let (&version, rest) = blob.split_first().ok_or(CryptoError::Decrypt)?;
        if version != FORMAT_V1 || rest.len() <= NONCE_LEN {
            return Err(CryptoError::Decrypt);
        }
        let (nonce, ciphertext) = rest.split_at(NONCE_LEN);
        let nonce = XNonce::try_from(nonce).map_err(|_| CryptoError::Decrypt)?;
        let plaintext = self
            .cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| CryptoError::Decrypt)?;
        String::from_utf8(plaintext).map_err(|_| CryptoError::Decrypt)
    }
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret_box() -> SecretBox {
        SecretBox::from_key([42u8; KEY_LEN])
    }

    #[test]
    fn roundtrip_and_format() {
        let sb = secret_box();
        let blob = sb.encrypt("s3cret-community", "snmp.community").unwrap();
        assert_eq!(blob[0], FORMAT_V1);
        assert!(blob.len() > 1 + NONCE_LEN + 16);
        assert!(!blob.windows(6).any(|w| w == b"s3cret"));
        assert_eq!(
            sb.decrypt(&blob, "snmp.community").unwrap(),
            "s3cret-community"
        );
    }

    #[test]
    fn empty_and_unicode_secrets_roundtrip() {
        let sb = secret_box();
        for text in ["", "pässwörd-ß-日本"] {
            let blob = sb.encrypt(text, "ctx").unwrap();
            assert_eq!(sb.decrypt(&blob, "ctx").unwrap(), text);
        }
    }

    #[test]
    fn same_plaintext_gives_different_blobs() {
        let sb = secret_box();
        assert_ne!(sb.encrypt("x", "c").unwrap(), sb.encrypt("x", "c").unwrap());
    }

    #[test]
    fn wrong_context_wrong_key_and_tampering_fail() {
        let sb = secret_box();
        let blob = sb.encrypt("value", "snmp.auth_password").unwrap();
        assert!(sb.decrypt(&blob, "snmp.priv_password").is_err());
        assert!(
            SecretBox::from_key([1u8; KEY_LEN])
                .decrypt(&blob, "snmp.auth_password")
                .is_err()
        );

        let mut tampered = blob.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(sb.decrypt(&tampered, "snmp.auth_password").is_err());

        let mut bad_version = blob.clone();
        bad_version[0] = 9;
        assert!(sb.decrypt(&bad_version, "snmp.auth_password").is_err());
        assert!(sb.decrypt(&[], "x").is_err());
        assert!(sb.decrypt(&[FORMAT_V1, 1, 2, 3], "x").is_err());
    }

    #[test]
    fn debug_does_not_reveal_anything() {
        assert_eq!(format!("{:?}", secret_box()), "SecretBox(***)");
    }

    #[test]
    fn key_file_generate_load_and_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("secret.key");
        SecretBox::generate_key_file(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.trim().len(), 64);

        // Never overwrites.
        assert!(SecretBox::generate_key_file(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), text);

        let sb = SecretBox::from_key_file(&path).unwrap();
        let blob = sb.encrypt("a", "c").unwrap();
        assert_eq!(
            SecretBox::from_key_file(&path)
                .unwrap()
                .decrypt(&blob, "c")
                .unwrap(),
            "a"
        );

        // Group/other access is refused.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(SecretBox::from_key_file(&path).is_err());
    }

    #[test]
    fn invalid_key_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, content: &str| {
            let path = dir.path().join(name);
            fs::write(&path, content).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            path
        };
        assert!(SecretBox::from_key_file(&dir.path().join("missing")).is_err());
        assert!(SecretBox::from_key_file(&write("short", "abcd\n")).is_err());
        assert!(SecretBox::from_key_file(&write("nothex", &"zz".repeat(32))).is_err());
        assert!(SecretBox::from_key_file(&write("ok", &"ab".repeat(32))).is_ok());
    }
}
