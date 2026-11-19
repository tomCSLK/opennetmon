//! Configuration: TOML file with environment overrides.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::auth::throttle::ClientIpHeader;

/// Environment variable with the path of the config file (used when `--config` is not given).
pub const ENV_CONFIG: &str = "OPENNETMON_CONFIG";
const ENV_LISTEN: &str = "OPENNETMON_LISTEN";
const ENV_DATABASE_PATH: &str = "OPENNETMON_DATABASE_PATH";
const ENV_LOG_FILTER: &str = "OPENNETMON_LOG_FILTER";
const ENV_SECRET_KEY_FILE: &str = "OPENNETMON_SECRET_KEY_FILE";
const ENV_TLS_CERT_FILE: &str = "OPENNETMON_TLS_CERT_FILE";
const ENV_TLS_KEY_FILE: &str = "OPENNETMON_TLS_KEY_FILE";
const ENV_GENERATE_MISSING: &str = "OPENNETMON_GENERATE_MISSING";
const ENV_COOKIE_SECURE: &str = "OPENNETMON_COOKIE_SECURE";
const ENV_SESSION_TTL_HOURS: &str = "OPENNETMON_SESSION_TTL_HOURS";
const ENV_SESSION_IDLE_MINUTES: &str = "OPENNETMON_SESSION_IDLE_MINUTES";
const ENV_CLIENT_IP_HEADER: &str = "OPENNETMON_CLIENT_IP_HEADER";
const ENV_POLLER_ENABLED: &str = "OPENNETMON_POLLER_ENABLED";
const ENV_POLL_CONCURRENCY: &str = "OPENNETMON_POLL_CONCURRENCY";
const ENV_RETENTION_RAW_DAYS: &str = "OPENNETMON_RETENTION_RAW_DAYS";
const ENV_RETENTION_HOURLY_DAYS: &str = "OPENNETMON_RETENTION_HOURLY_DAYS";
const ENV_RETENTION_AUDIT_DAYS: &str = "OPENNETMON_RETENTION_AUDIT_DAYS";
const ENV_UPDATE_CHECK: &str = "OPENNETMON_UPDATE_CHECK";
const ENV_TIME_ZONE: &str = "OPENNETMON_TIME_ZONE";
const ENV_BACKUP_DIR: &str = "OPENNETMON_BACKUP_DIR";
const ENV_BACKUP_KEEP: &str = "OPENNETMON_BACKUP_KEEP";
const ENV_BACKUP_TIME: &str = "OPENNETMON_BACKUP_TIME";
const ENV_PACKAGING: &str = "OPENNETMON_PACKAGING";
/// Most backups `backup_keep` may ask for.
const MAX_BACKUP_KEEP: u32 = 1_000;
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
    /// Create a missing `secret_key_file` and a missing certificate (self-signed, when both TLS files are
    /// set and neither exists) when the server starts. Off by default: the `.deb` and the installer create
    /// them; the container image switches it on.
    pub generate_missing: bool,
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
    /// Days entries of the audit log (changes, sign-ins) are kept (1 to 36500).
    pub retention_audit_days: u32,
    /// Look once a day on GitHub whether a newer release exists and tell administrators in the sidebar
    /// (GitHub sees the address of the server; nothing else is sent). Off unless set; the shipped config sets it.
    pub update_check: bool,
    /// Time zone of the weekly maintenance windows and the backup time, e.g. `Europe/Berlin`; unset = the
    /// system's (`TZ`, else `/etc/localtime`).
    pub time_zone: Option<String>,
    /// Directory for a daily backup made by the server itself (the container image sets it); unset = none
    /// (the `.deb` uses a systemd timer instead).
    pub backup_dir: Option<PathBuf>,
    /// How many of those backups are kept (1 to 1000); older ones are deleted.
    pub backup_keep: u32,
    /// Local time of that backup, `HH:MM` (24 hours).
    pub backup_time: String,
    /// How this copy was installed; decides which update steps the pages show.
    pub packaging: Packaging,
}

/// How opennetmon was installed, for the update steps on the pages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Packaging {
    /// The `.deb` package or the installer: update with `apt install`.
    #[default]
    Deb,
    /// The container image: update with `docker compose pull`.
    Container,
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
            generate_missing: false,
            cookie_secure: true,
            session_ttl_hours: 168,
            session_idle_minutes: 30,
            client_ip_header: None,
            poller_enabled: true,
            poll_concurrency: 16,
            retention_raw_days: 7,
            retention_hourly_days: 365,
            retention_audit_days: 365,
            update_check: false,
            time_zone: None,
            backup_dir: None,
            backup_keep: 14,
            backup_time: "03:30".to_owned(),
            packaging: Packaging::Deb,
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

        let env = Env(&env);
        if let Some(value) = env.parsed(ENV_LISTEN)? {
            config.listen = value;
        }
        if let Some(value) = env.text(ENV_DATABASE_PATH) {
            config.database_path = PathBuf::from(value);
        }
        if let Some(value) = env.text(ENV_LOG_FILTER) {
            config.log_filter = value;
        }
        if let Some(value) = env.text(ENV_SECRET_KEY_FILE) {
            config.secret_key_file = PathBuf::from(value);
        }
        if let Some(value) = env.optional(ENV_TLS_CERT_FILE) {
            config.tls_cert_file = value.map(PathBuf::from);
        }
        if let Some(value) = env.optional(ENV_TLS_KEY_FILE) {
            config.tls_key_file = value.map(PathBuf::from);
        }
        if let Some(value) = env.parsed(ENV_GENERATE_MISSING)? {
            config.generate_missing = value;
        }
        if let Some(value) = env.parsed(ENV_COOKIE_SECURE)? {
            config.cookie_secure = value;
        }
        if let Some(value) = env.parsed(ENV_SESSION_TTL_HOURS)? {
            config.session_ttl_hours = value;
        }
        if let Some(value) = env.parsed(ENV_SESSION_IDLE_MINUTES)? {
            config.session_idle_minutes = value;
        }
        if let Some(value) = env.optional(ENV_CLIENT_IP_HEADER) {
            config.client_ip_header = value
                .map(|v| env.keyword(ENV_CLIENT_IP_HEADER, &v))
                .transpose()?;
        }
        if let Some(value) = env.parsed(ENV_POLLER_ENABLED)? {
            config.poller_enabled = value;
        }
        if let Some(value) = env.parsed(ENV_POLL_CONCURRENCY)? {
            config.poll_concurrency = value;
        }
        if let Some(value) = env.parsed(ENV_RETENTION_RAW_DAYS)? {
            config.retention_raw_days = value;
        }
        if let Some(value) = env.parsed(ENV_RETENTION_HOURLY_DAYS)? {
            config.retention_hourly_days = value;
        }
        if let Some(value) = env.parsed(ENV_RETENTION_AUDIT_DAYS)? {
            config.retention_audit_days = value;
        }
        if let Some(value) = env.parsed(ENV_UPDATE_CHECK)? {
            config.update_check = value;
        }
        if let Some(value) = env.optional(ENV_TIME_ZONE) {
            config.time_zone = value;
        }
        if let Some(value) = env.optional(ENV_BACKUP_DIR) {
            config.backup_dir = value.map(PathBuf::from);
        }
        if let Some(value) = env.parsed(ENV_BACKUP_KEEP)? {
            config.backup_keep = value;
        }
        if let Some(value) = env.text(ENV_BACKUP_TIME) {
            config.backup_time = value;
        }
        if let Some(value) = env.text(ENV_PACKAGING) {
            config.packaging = env.keyword(ENV_PACKAGING, &value)?;
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

    /// Minutes after local midnight of the daily backup (`backup_time`, checked by [`Config::validate`]).
    pub fn backup_minute(&self) -> u16 {
        parse_time_of_day(&self.backup_time).unwrap_or(3 * 60 + 30)
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
        if !(1..=36_500).contains(&self.retention_audit_days) {
            return Err(ConfigError::Invalid(
                "retention_audit_days must be between 1 and 36500",
            ));
        }
        if let Some(zone) = &self.time_zone
            && jiff::tz::TimeZone::get(zone).is_err()
        {
            return Err(ConfigError::Invalid(
                "time_zone is not a known time zone (e.g. \"Europe/Berlin\", see timedatectl list-timezones)",
            ));
        }
        if self
            .backup_dir
            .as_ref()
            .is_some_and(|dir| dir.as_os_str().is_empty())
        {
            return Err(ConfigError::Invalid("backup_dir must not be empty"));
        }
        if !(1..=MAX_BACKUP_KEEP).contains(&self.backup_keep) {
            return Err(ConfigError::Invalid(
                "backup_keep must be between 1 and 1000",
            ));
        }
        if parse_time_of_day(&self.backup_time).is_none() {
            return Err(ConfigError::Invalid(
                "backup_time must be a time of day like \"03:30\"",
            ));
        }
        Ok(())
    }
}

/// `HH:MM` (24 hours) as minutes after midnight.
fn parse_time_of_day(text: &str) -> Option<u16> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let digits = |s: &str| (1..=2).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(hours) || minutes.len() != 2 || !digits(minutes) {
        return None;
    }
    let (hours, minutes): (u16, u16) = (hours.parse().ok()?, minutes.parse().ok()?);
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

/// Reads the `OPENNETMON_*` variables. An empty value of an optional setting (a path, the proxy header,
/// the time zone) unsets it, e.g. `OPENNETMON_TLS_CERT_FILE=` switches off a certificate from the file.
struct Env<'a, F: Fn(&str) -> Option<String>>(&'a F);

impl<F: Fn(&str) -> Option<String>> Env<'_, F> {
    fn text(&self, name: &str) -> Option<String> {
        (self.0)(name)
    }

    /// `Some(None)` for an empty value.
    fn optional(&self, name: &str) -> Option<Option<String>> {
        self.text(name)
            .map(|value| Some(value).filter(|v| !v.trim().is_empty()))
    }

    fn parsed<T>(&self, name: &'static str) -> Result<Option<T>, ConfigError>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        self.text(name)
            .map(|value| {
                value
                    .trim()
                    .parse()
                    .map_err(|e: T::Err| ConfigError::InvalidEnv {
                        name,
                        reason: e.to_string(),
                    })
            })
            .transpose()
    }

    /// A value of a setting that takes one of a few words (as in the file, e.g. `x-forwarded-for`).
    fn keyword<T: serde::de::DeserializeOwned>(
        &self,
        name: &'static str,
        value: &str,
    ) -> Result<T, ConfigError> {
        use serde::de::IntoDeserializer;
        let word: serde::de::value::StrDeserializer<'_, serde::de::value::Error> =
            value.trim().into_deserializer();
        T::deserialize(word).map_err(|e| ConfigError::InvalidEnv {
            name,
            reason: e.to_string(),
        })
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
    fn the_audit_log_is_kept_for_a_year_by_default() {
        assert_eq!(
            Config::from_sources(None, no_env)
                .unwrap()
                .retention_audit_days,
            365
        );
        assert!(Config::from_sources(Some("retention_audit_days = 0"), no_env).is_err());
    }

    #[test]
    fn the_time_zone_must_exist() {
        assert_eq!(Config::from_sources(None, no_env).unwrap().time_zone, None);
        let berlin = Config::from_sources(Some("time_zone = \"Europe/Berlin\""), no_env).unwrap();
        assert_eq!(berlin.time_zone.as_deref(), Some("Europe/Berlin"));
        assert!(Config::from_sources(Some("time_zone = \"Mars/Olympus\""), no_env).is_err());
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

    /// Every setting can come from the environment; the exhaustive literal makes a new setting show up here.
    #[test]
    fn every_setting_has_an_environment_variable() {
        let env = |name: &str| {
            let value = match name {
                ENV_LISTEN => "0.0.0.0:9443",
                ENV_DATABASE_PATH => "/data/x.db",
                ENV_LOG_FILTER => "debug",
                ENV_SECRET_KEY_FILE => "/conf/secret.key",
                ENV_TLS_CERT_FILE => "/conf/cert.pem",
                ENV_TLS_KEY_FILE => "/conf/key.pem",
                ENV_GENERATE_MISSING => "true",
                ENV_COOKIE_SECURE => "false",
                ENV_SESSION_TTL_HOURS => "12",
                ENV_SESSION_IDLE_MINUTES => "0",
                ENV_CLIENT_IP_HEADER => "x-real-ip",
                ENV_POLLER_ENABLED => "false",
                ENV_POLL_CONCURRENCY => "3",
                ENV_RETENTION_RAW_DAYS => "2",
                ENV_RETENTION_HOURLY_DAYS => "30",
                ENV_RETENTION_AUDIT_DAYS => "40",
                ENV_UPDATE_CHECK => "true",
                ENV_TIME_ZONE => "Europe/Berlin",
                ENV_BACKUP_DIR => "/data/backups",
                ENV_BACKUP_KEEP => "7",
                ENV_BACKUP_TIME => "4:05",
                ENV_PACKAGING => "container",
                _ => return None,
            };
            Some(value.to_owned())
        };
        let config = Config::from_sources(None, env).unwrap();
        let expected = Config {
            listen: "0.0.0.0:9443".parse().unwrap(),
            database_path: PathBuf::from("/data/x.db"),
            log_filter: "debug".to_owned(),
            secret_key_file: PathBuf::from("/conf/secret.key"),
            tls_cert_file: Some(PathBuf::from("/conf/cert.pem")),
            tls_key_file: Some(PathBuf::from("/conf/key.pem")),
            generate_missing: true,
            cookie_secure: false,
            session_ttl_hours: 12,
            session_idle_minutes: 0,
            client_ip_header: Some(ClientIpHeader::XRealIp),
            poller_enabled: false,
            poll_concurrency: 3,
            retention_raw_days: 2,
            retention_hourly_days: 30,
            retention_audit_days: 40,
            update_check: true,
            time_zone: Some("Europe/Berlin".to_owned()),
            backup_dir: Some(PathBuf::from("/data/backups")),
            backup_keep: 7,
            backup_time: "4:05".to_owned(),
            packaging: Packaging::Container,
        };
        assert_eq!(config, expected);
        assert_eq!(config.backup_minute(), 4 * 60 + 5);
    }

    #[test]
    fn an_empty_variable_unsets_an_optional_setting() {
        let file = "tls_cert_file = \"/c.pem\"\ntls_key_file = \"/k.pem\"\ntime_zone = \"UTC\"";
        let env = |name: &str| {
            [ENV_TLS_CERT_FILE, ENV_TLS_KEY_FILE, ENV_TIME_ZONE]
                .contains(&name)
                .then(String::new)
        };
        let config = Config::from_sources(Some(file), env).unwrap();
        assert_eq!(config.tls_files(), None);
        assert_eq!(config.time_zone, None);
    }

    #[test]
    fn invalid_environment_values_name_the_variable() {
        for (name, value) in [
            (ENV_COOKIE_SECURE, "yes"),
            (ENV_POLL_CONCURRENCY, "many"),
            (ENV_CLIENT_IP_HEADER, "forwarded"),
            (ENV_PACKAGING, "snap"),
        ] {
            let env = |n: &str| (n == name).then(|| value.to_owned());
            let err = Config::from_sources(None, env).unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidEnv { name: n, .. } if n == name),
                "{name}: {err}"
            );
        }
        // Range checks apply to the environment as to the file.
        let env = |n: &str| (n == ENV_POLL_CONCURRENCY).then(|| "0".to_owned());
        assert!(matches!(
            Config::from_sources(None, env).unwrap_err(),
            ConfigError::Invalid(_)
        ));
    }

    #[test]
    fn the_backup_is_off_by_default_and_its_settings_are_checked() {
        let config = Config::from_sources(None, no_env).unwrap();
        assert_eq!(config.backup_dir, None);
        assert_eq!((config.backup_keep, config.backup_minute()), (14, 210));
        assert!(!config.generate_missing);
        assert_eq!(config.packaging, Packaging::Deb);
        let ok = "backup_dir = \"/b\"\nbackup_keep = 1000\nbackup_time = \"23:59\"";
        assert_eq!(
            Config::from_sources(Some(ok), no_env)
                .unwrap()
                .backup_minute(),
            23 * 60 + 59
        );
        for bad in [
            "backup_dir = \"\"",
            "backup_keep = 0",
            "backup_keep = 1001",
            "backup_time = \"24:00\"",
            "backup_time = \"3:5\"",
            "backup_time = \"0330\"",
            "backup_time = \"-1:30\"",
            "packaging = \"snap\"",
        ] {
            assert!(Config::from_sources(Some(bad), no_env).is_err(), "{bad}");
        }
    }

    #[test]
    fn missing_file_reports_path() {
        let err = Config::load(Some(Path::new("/nonexistent/opennetmon.toml"))).unwrap_err();
        assert!(matches!(err, ConfigError::Read { .. }));
    }
}
