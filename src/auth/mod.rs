//! Authentication primitives: roles, password hashing, session tokens.

pub mod throttle;

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::error::{Error, Result};

pub const MIN_PASSWORD_LEN: usize = 10;
/// Upper bound that keeps Argon2 work (and request size) bounded.
pub const MAX_PASSWORD_LEN: usize = 128;
pub const MIN_USERNAME_LEN: usize = 3;
pub const MAX_USERNAME_LEN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Viewer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Viewer => "viewer",
        }
    }

    /// UI label (German).
    pub fn label(self) -> &'static str {
        match self {
            Self::Admin => "Administrator",
            Self::Viewer => "Leser",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(Self::Admin),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }

    pub fn is_admin(self) -> bool {
        self == Self::Admin
    }
}

/// Validates a new username. Messages are user-facing (German).
pub fn validate_username(name: &str) -> Result<(), &'static str> {
    let len = name.chars().count();
    if !(MIN_USERNAME_LEN..=MAX_USERNAME_LEN).contains(&len) {
        return Err("Der Benutzername muss 3 bis 32 Zeichen lang sein.");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err("Erlaubt sind Buchstaben, Ziffern sowie . _ -");
    }
    Ok(())
}

/// Validates a new password. Messages are user-facing (German).
pub fn validate_password(password: &str) -> Result<(), &'static str> {
    let len = password.chars().count();
    if len < MIN_PASSWORD_LEN {
        return Err("Das Passwort muss mindestens 10 Zeichen lang sein.");
    }
    if len > MAX_PASSWORD_LEN {
        return Err("Das Passwort darf höchstens 128 Zeichen lang sein.");
    }
    Ok(())
}

/// Hashes a password with Argon2id (library defaults) and a random salt.
pub fn hash_password(password: &str) -> Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|e| Error::Internal(format!("password hashing failed: {e}")))
}

/// Verifies a password against a stored PHC hash string.
pub fn verify_password(hash: &str, password: &str) -> bool {
    Argon2::default()
        .verify_password(password.as_bytes(), hash)
        .is_ok()
}

/// A hash to verify against when the user does not exist, so that unknown users
/// and wrong passwords take the same time.
pub fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| hash_password("opennetmon-dummy-password").unwrap_or_default())
}

/// 256 random bits as lowercase hex (session tokens, CSRF tokens).
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex(&bytes)
}

/// SHA-256 hex digest; sessions are stored under this, never under the raw token.
pub fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// Constant-time equality for secrets (tokens). Empty values never match.
pub fn secrets_match(a: &str, b: &str) -> bool {
    !a.is_empty() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let hash = hash_password("correct-horse-battery").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password(&hash, "correct-horse-battery"));
        assert!(!verify_password(&hash, "wrong-horse-battery"));
        assert!(!verify_password("not-a-hash", "correct-horse-battery"));
    }

    #[test]
    fn hashes_are_salted() {
        let a = hash_password("same-password-1").unwrap();
        let b = hash_password("same-password-1").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn dummy_hash_never_verifies_real_input() {
        assert!(!dummy_hash().is_empty());
        assert!(!verify_password(dummy_hash(), "anything-at-all"));
    }

    #[test]
    fn tokens_are_random_hex_and_hashes_differ() {
        let a = random_token();
        let b = random_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert_eq!(token_hash(&a).len(), 64);
        assert_ne!(token_hash(&a), a);
        assert_eq!(token_hash(&a), token_hash(&a));
    }

    #[test]
    fn secrets_match_is_exact_and_rejects_empty() {
        assert!(secrets_match("abc", "abc"));
        assert!(!secrets_match("abc", "abd"));
        assert!(!secrets_match("abc", "abcd"));
        assert!(!secrets_match("", ""));
    }

    #[test]
    fn username_rules() {
        assert!(validate_username("admin").is_ok());
        assert!(validate_username("a.b_c-d9").is_ok());
        assert!(validate_username("ab").is_err());
        assert!(validate_username(&"x".repeat(33)).is_err());
        assert!(validate_username("has space").is_err());
        assert!(validate_username("umlaut-ä").is_err());
    }

    #[test]
    fn password_rules() {
        assert!(validate_password("0123456789").is_ok());
        assert!(validate_password("012345678").is_err());
        assert!(validate_password(&"x".repeat(129)).is_err());
    }

    #[test]
    fn role_parse_roundtrip() {
        for role in [Role::Admin, Role::Viewer] {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("root"), None);
        assert!(Role::Admin.is_admin() && !Role::Viewer.is_admin());
    }
}
