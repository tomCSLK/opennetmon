//! Configuration: TOML file with environment overrides.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const ENV_LISTEN: &str = "NETMON_LISTEN";
const ENV_DATABASE_PATH: &str = "NETMON_DATABASE_PATH";
const ENV_LOG_FILTER: &str = "NETMON_LOG_FILTER";
const ENV_SECRET_KEY_FILE: &str = "NETMON_SECRET_KEY_FILE";
const MAX_SESSION_TTL_HOURS: u64 = 24 * 90;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read config file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid config file: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid value in {name}: {reason}")]
    InvalidEnv { name: &'static str, reason: String },
    #[error("invalid configuration: {0}")]
    Invalid(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Address the web server binds to. Default is loopback only; TLS is done by a reverse proxy.
    pub listen: SocketAddr,
    /// Path of the SQLite database file.
    pub database_path: PathBuf,
    /// `tracing` filter directive, e.g. `info,netmon=debug`. `RUST_LOG` takes precedence.
    pub log_filter: String,
    /// File with the 256-bit key (64 hex characters, mode 0600) that encrypts stored SNMP secrets.
    /// Create it with `netmon key generate`; losing it makes stored secrets unreadable.
    pub secret_key_file: PathBuf,
    /// Mark session cookies `Secure`. Keep `true` behind a TLS reverse proxy; only plain-HTTP development needs `false`.
    pub cookie_secure: bool,
    /// Absolute session lifetime in hours.
    pub session_ttl_hours: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 8080)),
            database_path: PathBuf::from("/var/lib/netmon/netmon.db"),
            log_filter: "info".to_owned(),
            secret_key_file: PathBuf::from("/etc/netmon/secret.key"),
            cookie_secure: true,
            session_ttl_hours: 168,
        }
    }
}

impl Config {
    /// Loads the config from an optional file, then applies `NETMON_*` environment overrides.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let text = match path {
            Some(path) => Some(
                fs::read_to_string(path).map_err(|source| ConfigError::Read {
                    path: path.to_owned(),
                    source,
                })?,
            ),
            None => None,
        };
        Self::from_sources(text.as_deref(), |name| std::env::var(name).ok())
    }

    /// Builds the config from TOML text and an environment lookup (injectable for tests).
    pub fn from_sources(
        toml_text: Option<&str>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let mut config: Self = match toml_text {
            Some(text) => toml::from_str(text)?,
            None => Self::default(),
        };

        if let Some(value) = env(ENV_LISTEN) {
            config.listen =
                value
                    .parse()
                    .map_err(|e: std::net::AddrParseError| ConfigError::InvalidEnv {
                        name: ENV_LISTEN,
                        reason: e.to_string(),
                    })?;
        }
        if let Some(value) = env(ENV_DATABASE_PATH) {
            config.database_path = PathBuf::from(value);
        }
        if let Some(value) = env(ENV_LOG_FILTER) {
            config.log_filter = value;
        }
        if let Some(value) = env(ENV_SECRET_KEY_FILE) {
            config.secret_key_file = PathBuf::from(value);
        }

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.database_path.as_os_str().is_empty() {
            return Err(ConfigError::Invalid("database_path must not be empty"));
        }
        if self.secret_key_file.as_os_str().is_empty() {
            return Err(ConfigError::Invalid("secret_key_file must not be empty"));
        }
        if self.log_filter.trim().is_empty() {
            return Err(ConfigError::Invalid("log_filter must not be empty"));
        }
        if !(1..=MAX_SESSION_TTL_HOURS).contains(&self.session_ttl_hours) {
            return Err(ConfigError::Invalid(
                "session_ttl_hours must be between 1 and 2160",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn defaults_bind_to_loopback() {
        let config = Config::from_sources(None, no_env).unwrap();
        assert_eq!(config.listen, "127.0.0.1:8080".parse().unwrap());
        assert_eq!(config, Config::default());
    }

    #[test]
    fn file_values_override_defaults() {
        let config = Config::from_sources(
            Some("listen = \"0.0.0.0:9000\"\ndatabase_path = \"/tmp/x.db\""),
            no_env,
        )
        .unwrap();
        assert_eq!(config.listen, "0.0.0.0:9000".parse().unwrap());
        assert_eq!(config.database_path, PathBuf::from("/tmp/x.db"));
        assert_eq!(config.log_filter, "info");
    }

    #[test]
    fn env_overrides_file() {
        let env = |name: &str| match name {
            ENV_LISTEN => Some("127.0.0.1:9999".to_owned()),
            ENV_DATABASE_PATH => Some("/data/env.db".to_owned()),
            _ => None,
        };
        let config = Config::from_sources(Some("listen = \"0.0.0.0:9000\""), env).unwrap();
        assert_eq!(config.listen, "127.0.0.1:9999".parse().unwrap());
        assert_eq!(config.database_path, PathBuf::from("/data/env.db"));
    }

    #[test]
    fn secret_key_file_default_and_override() {
        let config = Config::from_sources(None, no_env).unwrap();
        assert_eq!(
            config.secret_key_file,
            PathBuf::from("/etc/netmon/secret.key")
        );
        let env = |name: &str| (name == ENV_SECRET_KEY_FILE).then(|| "/k/x.key".to_owned());
        let config = Config::from_sources(None, env).unwrap();
        assert_eq!(config.secret_key_file, PathBuf::from("/k/x.key"));
        let err = Config::from_sources(Some("secret_key_file = \"\""), no_env).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let err = Config::from_sources(Some("lissen = \"x\""), no_env).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));
    }

    #[test]
    fn invalid_listen_in_file_is_rejected() {
        let err = Config::from_sources(Some("listen = \"not-an-address\""), no_env).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));
    }

    #[test]
    fn invalid_listen_in_env_is_rejected() {
        let env = |name: &str| (name == ENV_LISTEN).then(|| "nope".to_owned());
        let err = Config::from_sources(None, env).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidEnv {
                name: ENV_LISTEN,
                ..
            }
        ));
    }

    #[test]
    fn empty_database_path_is_rejected() {
        let err = Config::from_sources(Some("database_path = \"\""), no_env).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn cookies_are_secure_by_default_and_ttl_is_validated() {
        let config = Config::from_sources(None, no_env).unwrap();
        assert!(config.cookie_secure);
        assert_eq!(config.session_ttl_hours, 168);

        let config = Config::from_sources(Some("cookie_secure = false"), no_env).unwrap();
        assert!(!config.cookie_secure);

        for bad in ["session_ttl_hours = 0", "session_ttl_hours = 2161"] {
            let err = Config::from_sources(Some(bad), no_env).unwrap_err();
            assert!(matches!(err, ConfigError::Invalid(_)), "{bad}");
        }
    }

    #[test]
    fn missing_file_reports_path() {
        let err = Config::load(Some(Path::new("/nonexistent/netmon.toml"))).unwrap_err();
        assert!(matches!(err, ConfigError::Read { .. }));
    }
}
