//! SNMP credential profile domain types and input validation.
//!
//! Secrets are wrapped in [`Secret`] (redacted `Debug`, no `Display`). Messages returned by validation are
//! user-facing (German).

use std::fmt;

/// A secret string that never shows up in logs or debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => ($key:literal, $label:literal)),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// Stable key used in the database and in forms.
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $key),+ }
            }

            /// UI label.
            pub fn label(self) -> &'static str {
                match self { $(Self::$variant => $label),+ }
            }

            pub fn parse(s: &str) -> Option<Self> {
                match s { $($key => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

string_enum!(SnmpVersion {
    V2c => ("v2c", "SNMPv2c"),
    V3 => ("v3", "SNMPv3"),
});

string_enum!(SecurityLevel {
    AuthNoPriv => ("auth_no_priv", "authNoPriv (nur Authentifizierung)"),
    AuthPriv => ("auth_priv", "authPriv (Authentifizierung und Verschlüsselung)"),
});

string_enum!(AuthProtocol {
    Md5 => ("md5", "MD5"),
    Sha1 => ("sha1", "SHA-1"),
    Sha224 => ("sha224", "SHA-224"),
    Sha256 => ("sha256", "SHA-256"),
    Sha384 => ("sha384", "SHA-384"),
    Sha512 => ("sha512", "SHA-512"),
});

string_enum!(PrivProtocol {
    Des => ("des", "DES"),
    Aes128 => ("aes128", "AES-128"),
    Aes192 => ("aes192", "AES-192"),
    Aes256 => ("aes256", "AES-256"),
});

pub const MAX_NAME_LEN: usize = 64;
pub const MAX_COMMUNITY_LEN: usize = 128;
pub const MAX_V3_USERNAME_LEN: usize = 32;
/// RFC 3414 requires at least 8 characters for USM passwords.
pub const MIN_V3_PASSWORD_LEN: usize = 8;
pub const MAX_V3_PASSWORD_LEN: usize = 128;

/// What the user submitted for a profile. `None` secrets mean "keep the stored one" when editing.
#[derive(Debug, Clone)]
pub struct ProfileInput {
    pub name: String,
    pub kind: ProfileKind,
}

#[derive(Debug, Clone)]
pub enum ProfileKind {
    V2c {
        community: Option<Secret>,
    },
    V3 {
        username: String,
        level: SecurityLevel,
        auth_protocol: AuthProtocol,
        auth_password: Option<Secret>,
        priv_protocol: Option<PrivProtocol>,
        priv_password: Option<Secret>,
    },
}

/// Fully resolved, decrypted credentials, as the poller will need them.
#[derive(Debug, Clone)]
pub enum Credentials {
    V2c {
        community: Secret,
    },
    V3 {
        username: String,
        auth_protocol: AuthProtocol,
        auth_password: Secret,
        privacy: Option<(PrivProtocol, Secret)>,
    },
}

/// A stored profile without any secret (secrets are only ever reported as "set").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileView {
    pub id: i64,
    pub name: String,
    pub version: SnmpVersion,
    pub username: Option<String>,
    pub level: Option<SecurityLevel>,
    pub auth_protocol: Option<AuthProtocol>,
    pub priv_protocol: Option<PrivProtocol>,
}

impl ProfileView {
    /// One-line description for lists, e.g. `SNMPv3 · authNoPriv · MD5 · Benutzer MCS6_SNMP`.
    pub fn summary(&self) -> String {
        let mut parts = vec![self.version.label().to_owned()];
        if let Some(level) = self.level {
            parts.push(
                match level {
                    SecurityLevel::AuthNoPriv => "authNoPriv",
                    SecurityLevel::AuthPriv => "authPriv",
                }
                .to_owned(),
            );
        }
        if let Some(p) = self.auth_protocol {
            parts.push(p.label().to_owned());
        }
        if let Some(p) = self.priv_protocol {
            parts.push(p.label().to_owned());
        }
        if let Some(user) = &self.username {
            parts.push(format!("Benutzer {user}"));
        }
        parts.join(" · ")
    }
}

fn has_control_chars(s: &str) -> bool {
    s.chars().any(char::is_control)
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

impl ProfileInput {
    /// Checks everything that does not depend on stored data. Returns a German message on failure.
    pub fn validate(&self) -> Result<(), &'static str> {
        let name = self.name.trim();
        if name.is_empty() || char_len(name) > MAX_NAME_LEN || has_control_chars(name) {
            return Err(
                "Der Name muss 1 bis 64 Zeichen lang sein und darf keine Steuerzeichen enthalten.",
            );
        }
        match &self.kind {
            ProfileKind::V2c { community } => {
                if let Some(c) = community {
                    let c = c.expose();
                    if c.is_empty() || char_len(c) > MAX_COMMUNITY_LEN || has_control_chars(c) {
                        return Err("Die Community muss 1 bis 128 Zeichen lang sein.");
                    }
                }
            }
            ProfileKind::V3 {
                username,
                level,
                auth_password,
                priv_protocol,
                priv_password,
                ..
            } => {
                if username.is_empty()
                    || char_len(username) > MAX_V3_USERNAME_LEN
                    || has_control_chars(username)
                    || username.chars().any(char::is_whitespace)
                {
                    return Err(
                        "Der SNMPv3-Benutzer muss 1 bis 32 Zeichen lang sein und darf keine Leerzeichen enthalten.",
                    );
                }
                for secret in [auth_password, priv_password].into_iter().flatten() {
                    let len = char_len(secret.expose());
                    if !(MIN_V3_PASSWORD_LEN..=MAX_V3_PASSWORD_LEN).contains(&len) {
                        return Err("SNMPv3-Passwörter müssen 8 bis 128 Zeichen lang sein.");
                    }
                }
                if *level == SecurityLevel::AuthPriv && priv_protocol.is_none() {
                    return Err("Für authPriv ist ein Verschlüsselungsverfahren erforderlich.");
                }
            }
        }
        Ok(())
    }

    pub fn trimmed_name(&self) -> &str {
        self.name.trim()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v3(
        username: &str,
        level: SecurityLevel,
        auth: Option<&str>,
        prot: Option<PrivProtocol>,
        privp: Option<&str>,
    ) -> ProfileInput {
        ProfileInput {
            name: "Omada".to_owned(),
            kind: ProfileKind::V3 {
                username: username.to_owned(),
                level,
                auth_protocol: AuthProtocol::Md5,
                auth_password: auth.map(Secret::new),
                priv_protocol: prot,
                priv_password: privp.map(Secret::new),
            },
        }
    }

    #[test]
    fn secret_debug_is_redacted() {
        let input = v3(
            "user",
            SecurityLevel::AuthNoPriv,
            Some("topsecret1"),
            None,
            None,
        );
        let shown = format!("{input:?}");
        assert!(!shown.contains("topsecret1"));
        assert!(shown.contains("Secret(***)"));
    }

    #[test]
    fn enums_roundtrip_through_keys() {
        for v in SnmpVersion::ALL {
            assert_eq!(SnmpVersion::parse(v.as_str()), Some(*v));
        }
        for v in SecurityLevel::ALL {
            assert_eq!(SecurityLevel::parse(v.as_str()), Some(*v));
        }
        for v in AuthProtocol::ALL {
            assert_eq!(AuthProtocol::parse(v.as_str()), Some(*v));
        }
        for v in PrivProtocol::ALL {
            assert_eq!(PrivProtocol::parse(v.as_str()), Some(*v));
        }
        assert_eq!(AuthProtocol::parse("rot13"), None);
    }

    #[test]
    fn valid_inputs_pass() {
        assert!(
            v3(
                "MCS6_SNMP",
                SecurityLevel::AuthNoPriv,
                Some("12345678"),
                None,
                None
            )
            .validate()
            .is_ok()
        );
        assert!(
            v3(
                "u",
                SecurityLevel::AuthPriv,
                Some("12345678"),
                Some(PrivProtocol::Aes128),
                Some("87654321")
            )
            .validate()
            .is_ok()
        );
        let v2c = ProfileInput {
            name: " public ".to_owned(),
            kind: ProfileKind::V2c {
                community: Some(Secret::new("public")),
            },
        };
        assert!(v2c.validate().is_ok());
        assert_eq!(v2c.trimmed_name(), "public");
        // Keeping stored secrets (None) is valid.
        assert!(
            v3("u", SecurityLevel::AuthNoPriv, None, None, None)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn invalid_inputs_fail() {
        let bad_name = |name: &str| ProfileInput {
            name: name.to_owned(),
            kind: ProfileKind::V2c { community: None },
        };
        assert!(bad_name("   ").validate().is_err());
        assert!(bad_name(&"x".repeat(65)).validate().is_err());
        assert!(bad_name("a\nb").validate().is_err());

        assert!(
            v3("", SecurityLevel::AuthNoPriv, None, None, None)
                .validate()
                .is_err()
        );
        assert!(
            v3("has space", SecurityLevel::AuthNoPriv, None, None, None)
                .validate()
                .is_err()
        );
        assert!(
            v3(&"u".repeat(33), SecurityLevel::AuthNoPriv, None, None, None)
                .validate()
                .is_err()
        );
        assert!(
            v3("u", SecurityLevel::AuthNoPriv, Some("short"), None, None)
                .validate()
                .is_err()
        );
        assert!(
            v3(
                "u",
                SecurityLevel::AuthPriv,
                Some("12345678"),
                None,
                Some("87654321")
            )
            .validate()
            .is_err()
        );
        assert!(
            v3(
                "u",
                SecurityLevel::AuthPriv,
                Some("12345678"),
                Some(PrivProtocol::Des),
                Some("short")
            )
            .validate()
            .is_err()
        );

        let long_community = ProfileInput {
            name: "n".to_owned(),
            kind: ProfileKind::V2c {
                community: Some(Secret::new("c".repeat(129))),
            },
        };
        assert!(long_community.validate().is_err());
        let empty_community = ProfileInput {
            name: "n".to_owned(),
            kind: ProfileKind::V2c {
                community: Some(Secret::new("")),
            },
        };
        assert!(empty_community.validate().is_err());
    }

    #[test]
    fn summary_describes_without_secrets() {
        let view = ProfileView {
            id: 1,
            name: "Omada".to_owned(),
            version: SnmpVersion::V3,
            username: Some("MCS6_SNMP".to_owned()),
            level: Some(SecurityLevel::AuthNoPriv),
            auth_protocol: Some(AuthProtocol::Md5),
            priv_protocol: None,
        };
        assert_eq!(
            view.summary(),
            "SNMPv3 · authNoPriv · MD5 · Benutzer MCS6_SNMP"
        );
    }
}
