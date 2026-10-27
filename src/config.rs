//! Configuration: TOML file with environment overrides.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::auth::throttle::ClientIpHeader;

const ENV_LISTEN: &str = "OPENNETMON_LISTEN";
const ENV_DATABASE_PATH: &str = "OPENNETMON_DATABASE_PATH";
const ENV_LOG_FILTER: &str = "OPENNETMON_LOG_FILTER";
const ENV_SECRET_KEY_FILE: &str = "OPENNETMON_SECRET_KEY_FILE";
const MAX_SESSION_TTL_HOURS: u64 = 24 * 90;
const MAX_SESSION_IDLE_MINUTES: u64 = 24 * 60;
const MAX_POLL_CONCURRENCY: usize = 256;

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
    /// Address the web server binds to. Default is loopback only (plain HTTP behind a TLS reverse proxy);
    /// with `tls_cert_file`/`tls_key_file` it may be any address, e.g. `0.0.0.0:8443`.
    pub listen: SocketAddr,
    /// Path of the SQLite database file.
    pub database_path: PathBuf,
    /// `tracing` filter directive, e.g. `info,opennetmon=debug`. `RUST_LOG` takes precedence.
    pub log_filter: String,
    /// File with the 256-bit key (64 hex characters, mode 0600) that encrypts stored SNMP secrets.
    /// Create it with `opennetmon key generate`; losing it makes stored secrets unreadable.
    pub secret_key_file: PathBuf,
    /// Certificate chain for built-in HTTPS (PEM, server certificate first, then intermediates). Together
    /// with `tls_key_file` the server speaks only HTTPS on `listen`; plain HTTP there gets a redirect.
    pub tls_cert_file: Option<PathBuf>,
    /// Private key of that certificate (PEM, mode 0600, readable by the service user).
    pub tls_key_file: Option<PathBuf>,
    /// Mark session cookies `Secure`. Keep `true` behind a TLS reverse proxy; only plain-HTTP development needs `false`.
    pub cookie_secure: bool,
    /// Absolute session lifetime in hours.
    pub session_ttl_hours: u64,
    /// Sign users out after this many minutes without a click or key press (automatic reloads do not count);
    /// `0` switches it off. Users marked "stay signed in" are exempt; `session_ttl_hours` applies to everyone.
    pub session_idle_minutes: u64,
    /// Header that carries the real client address behind a reverse proxy (`x-forwarded-for`, last entry, or
    /// `x-real-ip`). Only set it if the proxy always overwrites/appends it and the service is not reachable
    /// directly; the login throttling uses this address. Unset: the TCP peer address.
    pub client_ip_header: Option<ClientIpHeader>,
    /// Run the poller. Disable to use the web interface without polling (e.g. on a second instance).
    pub poller_enabled: bool,
    /// How many devices are polled at the same time (1 to 256).
    pub poll_concurrency: usize,
    /// Days raw measurements are kept (1 to 3650); hourly and daily rollups serve longer ranges.
    pub retention_raw_days: u32,
    /// Days hourly rollups and status events are kept (at least `retention_raw_days`); daily rollups are kept forever.
    pub retention_hourly_days: u32,
    /// Look once a day on GitHub whether a newer release exists and tell administrators in the sidebar
    /// (GitHub sees the address of the server; nothing else is sent). Off unless set; the shipped config sets it.
    pub update_check: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 8080)),
            database_path: PathBuf::from("/var/lib/opennetmon/opennetmon.db"),
            log_filter: "info".to_owned(),
            secret_key_file: PathBuf::from("/etc/opennetmon/secret.key"),
            tls_cert_file: None,
            tls_key_file: None,
            cookie_secure: true,
            session_ttl_hours: 168,
            session_idle_minutes: 30,
            client_ip_header: None,
            poller_enabled: true,
            poll_concurrency: 16,
            retention_raw_days: 7,
            retention_hourly_days: 365,
            update_check: false,
        }
    }
}

impl Config {
    /// Loads the config from an optional file, then applies `OPENNETMON_*` environment overrides.
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

    /// Certificate and key file when built-in HTTPS is configured.
    pub fn tls_files(&self) -> Option<(&Path, &Path)> {
        Some((
            self.tls_cert_file.as_deref()?,
            self.tls_key_file.as_deref()?,
        ))
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.database_path.as_os_str().is_empty() {
            return Err(ConfigError::Invalid("database_path must not be empty"));
        }
        if self.secret_key_file.as_os_str().is_empty() {
            return Err(ConfigError::Invalid("secret_key_file must not be empty"));
        }
        match (&self.tls_cert_file, &self.tls_key_file) {
            (Some(cert), Some(key))
                if cert.as_os_str().is_empty() || key.as_os_str().is_empty() =>
            {
                return Err(ConfigError::Invalid(
                    "tls_cert_file and tls_key_file must not be empty",
                ));
            }
            (Some(_), None) | (None, Some(_)) => {
                return Err(ConfigError::Invalid(
                    "set both tls_cert_file and tls_key_file, or neither",
                ));
            }
            _ => {}
        }
        if self.log_filter.trim().is_empty() {
            return Err(ConfigError::Invalid("log_filter must not be empty"));
        }
        if !(1..=MAX_SESSION_TTL_HOURS).contains(&self.session_ttl_hours) {
            return Err(ConfigError::Invalid(
                "session_ttl_hours must be between 1 and 2160",
            ));
        }
        if self.session_idle_minutes > MAX_SESSION_IDLE_MINUTES {
            return Err(ConfigError::Invalid(
                "session_idle_minutes must be between 0 (off) and 1440",
            ));
        }
        if !(1..=MAX_POLL_CONCURRENCY).contains(&self.poll_concurrency) {
            return Err(ConfigError::Invalid(
                "poll_concurrency must be between 1 and 256",
            ));
        }
        if !(1..=3_650).contains(&self.retention_raw_days) {
            return Err(ConfigError::Invalid(
                "retention_raw_days must be between 1 and 3650",
            ));
        }
        if !(self.retention_raw_days..=36_500).contains(&self.retention_hourly_days) {
            return Err(ConfigError::Invalid(
                "retention_hourly_days must be between retention_raw_days and 36500",
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
    fn poller_settings_have_defaults_and_are_validated() {
        let config = Config::from_sources(None, no_env).unwrap();
        assert!(config.poller_enabled);
        assert_eq!(config.poll_concurrency, 16);
        let parsed =
            Config::from_sources(Some("poller_enabled = false\npoll_concurrency = 4"), no_env)
                .unwrap();
        assert!(!parsed.poller_enabled);
        assert_eq!(parsed.poll_concurrency, 4);
        for bad in ["poll_concurrency = 0", "poll_concurrency = 257"] {
            assert!(Config::from_sources(Some(bad), no_env).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_update_check_is_off_unless_switched_on() {
        assert!(!Config::from_sources(None, no_env).unwrap().update_check);
        let on = Config::from_sources(Some("update_check = true"), no_env).unwrap();
        assert!(on.update_check);
    }

    #[test]
    fn tls_needs_both_files() {
        let config = Config::from_sources(None, no_env).unwrap();
        assert_eq!(config.tls_files(), None);
        let both = "listen = \"0.0.0.0:8443\"\ntls_cert_file = \"/etc/c.pem\"\ntls_key_file = \"/etc/k.pem\"";
        let config = Config::from_sources(Some(both), no_env).unwrap();
        assert_eq!(
            config.tls_files(),
            Some((Path::new("/etc/c.pem"), Path::new("/etc/k.pem")))
        );
        for bad in [
            "tls_cert_file = \"/etc/c.pem\"",
            "tls_key_file = \"/etc/k.pem\"",
            "tls_cert_file = \"\"\ntls_key_file = \"/etc/k.pem\"",
        ] {
            assert!(Config::from_sources(Some(bad), no_env).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_shipped_example_config_is_valid() {
        let text = include_str!("../deploy/config.example.toml");
        let config = Config::from_sources(Some(text), no_env).unwrap();
        assert!(config.tls_files().is_some());
        assert_eq!(config.listen, "0.0.0.0:8443".parse().unwrap());
    }

    #[test]
    fn retention_defaults_and_validation() {
        let config = Config::from_sources(None, no_env).unwrap();
        assert_eq!(
            (config.retention_raw_days, config.retention_hourly_days),
            (7, 365)
        );
        let ok = Config::from_sources(
            Some("retention_raw_days = 14\nretention_hourly_days = 14"),
            no_env,
        );
        assert!(ok.is_ok());
        for bad in [
            "retention_raw_days = 0",
            "retention_raw_days = 3651",
            "retention_raw_days = 30\nretention_hourly_days = 29",
            "retention_hourly_days = 36501",
        ] {
            assert!(Config::from_sources(Some(bad), no_env).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_proxy_header_is_off_by_default_and_accepts_known_names_only() {
        assert_eq!(
            Config::from_sources(None, no_env).unwrap().client_ip_header,
            None
        );
        let xff =
            Config::from_sources(Some("client_ip_header = \"x-forwarded-for\""), no_env).unwrap();
        assert_eq!(xff.client_ip_header, Some(ClientIpHeader::XForwardedFor));
        let real = Config::from_sources(Some("client_ip_header = \"x-real-ip\""), no_env).unwrap();
        assert_eq!(real.client_ip_header, Some(ClientIpHeader::XRealIp));
        assert!(Config::from_sources(Some("client_ip_header = \"forwarded\""), no_env).is_err());
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
            PathBuf::from("/etc/opennetmon/secret.key")
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
        assert_eq!(config.session_idle_minutes, 30);
        let config = Config::from_sources(Some("session_idle_minutes = 0"), no_env).unwrap();
        assert_eq!(config.session_idle_minutes, 0);

        let config = Config::from_sources(Some("cookie_secure = false"), no_env).unwrap();
        assert!(!config.cookie_secure);

        for bad in [
            "session_ttl_hours = 0",
            "session_ttl_hours = 2161",
            "session_idle_minutes = 1441",
        ] {
            let err = Config::from_sources(Some(bad), no_env).unwrap_err();
            assert!(matches!(err, ConfigError::Invalid(_)), "{bad}");
        }
    }

    #[test]
    fn missing_file_reports_path() {
        let err = Config::load(Some(Path::new("/nonexistent/opennetmon.toml"))).unwrap_err();
        assert!(matches!(err, ConfigError::Read { .. }));
    }
}
