use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};
use opennetmon::auth::{self, Role};
use opennetmon::config::Config;
use opennetmon::crypto::SecretBox;
use opennetmon::i18n::Lang;
use opennetmon::maintenance::Retention;
use opennetmon::poller::Poller;
use opennetmon::storage::{self, users};
use opennetmon::tls;
use opennetmon::web::{self, AppState};
use sqlx::SqlitePool;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "Agentless SNMP network monitor")]
struct Args {
    /// Path to the TOML config file (defaults and OPENNETMON_* environment variables apply without it)
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the web server (default)
    Serve,
    /// Manage local users
    #[command(subcommand)]
    User(UserCommand),
    /// Manage the key that encrypts stored SNMP secrets
    #[command(subcommand)]
    Key(KeyCommand),
    /// Manage the certificate for built-in HTTPS
    #[command(subcommand)]
    Tls(TlsCommand),
    /// Write a consistent copy of the database (safe while the server runs; never overwrites)
    Backup {
        /// Target file; created with mode 0600
        target: PathBuf,
    },
}

#[derive(Subcommand)]
enum KeyCommand {
    /// Create a new random key file (mode 0600); never overwrites an existing file
    Generate,
}

#[derive(Subcommand)]
enum TlsCommand {
    /// Create a self-signed certificate (ECDSA P-256, ten years) for this server; never overwrites existing files.
    /// It covers the host name, localhost and the loopback addresses, plus every --name.
    Generate {
        /// Host name or IP address clients use to reach the server (repeatable)
        #[arg(long = "name")]
        names: Vec<String>,
        /// Certificate file (default: tls_cert_file from the config)
        #[arg(long)]
        cert: Option<PathBuf>,
        /// Key file, created with mode 0600 (default: tls_key_file from the config)
        #[arg(long)]
        key: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum UserCommand {
    /// Create a user; the password is prompted for (or read from stdin when piped)
    Add {
        username: String,
        #[arg(long, value_enum, default_value = "viewer")]
        role: RoleArg,
        /// Language of the user interface (de, en); without it the browser's language is used
        #[arg(long, value_parser = parse_language)]
        language: Option<Lang>,
    },
    /// Set a new password and end all sessions of the user
    Passwd { username: String },
    /// List users
    List,
}

#[derive(Clone, Copy, ValueEnum)]
enum RoleArg {
    Admin,
    Viewer,
}

fn parse_language(code: &str) -> Result<Lang, String> {
    Lang::parse(code).ok_or_else(|| {
        let known: Vec<_> = Lang::ALL.iter().map(|l| l.code()).collect();
        format!("unknown language '{code}' (known: {})", known.join(", "))
    })
}

impl From<RoleArg> for Role {
    fn from(r: RoleArg) -> Self {
        match r {
            RoleArg::Admin => Role::Admin,
            RoleArg::Viewer => Role::Viewer,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = Config::load(args.config.as_deref()).context("loading configuration")?;

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&config.log_filter))
        .context("invalid log filter")?;
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let command = args.command.unwrap_or(Command::Serve);
    if let Command::Key(KeyCommand::Generate) = command {
        SecretBox::generate_key_file(&config.secret_key_file).context("creating key file")?;
        println!("Key file created: {}", config.secret_key_file.display());
        println!(
            "Back it up separately from the database: without it, stored SNMP secrets cannot be decrypted."
        );
        return Ok(());
    }

    if let Command::Tls(TlsCommand::Generate { names, cert, key }) = &command {
        let cert = cert.clone().or_else(|| config.tls_cert_file.clone());
        let key = key.clone().or_else(|| config.tls_key_file.clone());
        let (Some(cert), Some(key)) = (cert, key) else {
            bail!(
                "no certificate files: set tls_cert_file and tls_key_file in the config, or pass --cert and --key"
            );
        };
        // Names found on the system (e.g. a host name with "_") must not make the installation fail.
        let (valid, skipped): (Vec<_>, Vec<_>) = tls::certificate_names(names)
            .into_iter()
            .partition(|name| tls::is_valid_name(name));
        for name in &skipped {
            eprintln!("Skipping '{name}': not a valid host name or IP address.");
        }
        let generated = tls::generate(&cert, &key, &valid).context("creating the certificate")?;
        println!(
            "Certificate created: {} (key: {})",
            cert.display(),
            key.display()
        );
        println!("Valid for: {}", generated.names.join(", "));
        println!("SHA-256 fingerprint: {}", generated.fingerprint);
        println!(
            "It is self-signed: browsers warn on the first visit; compare the fingerprint before accepting it."
        );
        return Ok(());
    }

    if let Command::Backup { target } = &command {
        let pool = storage::connect(&config.database_path)
            .await
            .context("opening database")?;
        storage::backup_to(&pool, target)
            .await
            .context("writing the backup")?;
        println!("Backup written: {}", target.display());
        println!(
            "It contains the encrypted secrets but not the key file ({}); back that up separately.",
            config.secret_key_file.display()
        );
        return Ok(());
    }

    let pool = storage::connect(&config.database_path)
        .await
        .context("opening database")?;
    storage::migrate(&pool)
        .await
        .context("running migrations")?;

    match command {
        Command::Serve => serve(config, pool).await,
        Command::User(cmd) => manage_users(&pool, cmd).await,
        Command::Key(_) | Command::Tls(_) | Command::Backup { .. } => Ok(()),
    }
}

async fn serve(config: Config, pool: SqlitePool) -> anyhow::Result<()> {
    let secrets = SecretBox::from_key_file(&config.secret_key_file).with_context(|| {
        format!(
            "loading the encryption key (create one with `opennetmon key generate`; config key secret_key_file = {})",
            config.secret_key_file.display()
        )
    })?;
    if users::count_users(&pool).await? == 0 {
        tracing::warn!(
            "no users exist yet; create an admin with `opennetmon user add <name> --role admin`"
        );
    }
    let tls = match config.tls_files() {
        Some((cert, key)) => {
            let loaded = tls::load(cert, key).with_context(|| {
                format!(
                    "loading the HTTPS certificate (create one with `opennetmon tls generate`; config keys tls_cert_file = {}, tls_key_file = {})",
                    cert.display(),
                    key.display()
                )
            })?;
            Some(loaded)
        }
        None => None,
    };
    if tls.is_none() && !config.listen.ip().is_loopback() {
        tracing::warn!(
            listen = %config.listen,
            "serving plain HTTP on a non-loopback address; set tls_cert_file and tls_key_file or put a TLS-terminating reverse proxy in front"
        );
    }
    if !config.cookie_secure {
        if tls.is_some() {
            tracing::warn!("cookie_secure is disabled although HTTPS is on; set it to true");
        } else {
            tracing::warn!("cookie_secure is disabled; only use this for plain-HTTP development");
        }
    }
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| {
            let hint = if config.listen.port() < 1024 {
                " (ports below 1024 need CAP_NET_BIND_SERVICE, see docs/betrieb.md)"
            } else {
                ""
            };
            format!("binding {}{hint}", config.listen)
        })?;
    if let Some(tls) = &tls {
        tracing::info!(fingerprint = %tls.fingerprint, "HTTPS only; plain HTTP is redirected");
    }
    tracing::info!(listen = %config.listen, db = %config.database_path.display(), "opennetmon started");

    let secrets = Arc::new(secrets);
    let poller = if config.poller_enabled {
        Some(Poller::spawn(
            pool.clone(),
            Arc::clone(&secrets),
            config.poll_concurrency,
            Retention {
                raw_days: config.retention_raw_days,
                hourly_days: config.retention_hourly_days,
            },
        ))
    } else {
        tracing::warn!("poller_enabled is false; no device is polled");
        None
    };

    let state = AppState::new(pool, &config, secrets);
    state.reload_mibs().await.context("loading the MIB files")?;
    let mibs = state.mib_index();
    tracing::info!(
        objects = mibs.len(),
        modules = mibs.reports().len(),
        "MIBs loaded"
    );
    let app = web::router(state);
    let served = match tls {
        Some(tls) => {
            tls::serve(listener, app, tls.config, shutdown_signal()).await;
            Ok(())
        }
        None => axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving"),
    };
    if let Some(poller) = poller {
        poller.stop().await;
    }
    served?;
    tracing::info!("opennetmon stopped");
    Ok(())
}

async fn manage_users(pool: &SqlitePool, cmd: UserCommand) -> anyhow::Result<()> {
    match cmd {
        UserCommand::Add {
            username,
            role,
            language,
        } => {
            auth::validate_username(&username).map_err(|m| anyhow::anyhow!(Lang::En.msg(&m)))?;
            let password = read_new_password()?;
            let hash = auth::hash_password(&password)?;
            users::create_user(pool, &username, &hash, role.into(), language).await?;
            println!("User '{username}' created.");
        }
        UserCommand::Passwd { username } => {
            if users::find_by_username(pool, &username).await?.is_none() {
                bail!("user '{username}' does not exist");
            }
            let password = read_new_password()?;
            let hash = auth::hash_password(&password)?;
            users::set_password_hash(pool, &username, &hash).await?;
            users::delete_sessions_of_user(pool, &username).await?;
            println!("Password for '{username}' changed; existing sessions were ended.");
        }
        UserCommand::List => {
            for user in users::list_users(pool).await? {
                println!("{:<32} {}", user.username, user.role.as_str());
            }
        }
    }
    Ok(())
}

/// Prompts twice on a terminal; reads a single line from stdin when piped.
fn read_new_password() -> anyhow::Result<String> {
    let password = if std::io::stdin().is_terminal() {
        let first = rpassword::prompt_password("New password: ")?;
        let second = rpassword::prompt_password("Repeat password: ")?;
        if first != second {
            bail!("passwords do not match");
        }
        first
    } else {
        let mut line = String::new();
        std::io::stdin().take(1024).read_to_string(&mut line)?;
        line.lines().next().unwrap_or_default().to_owned()
    };
    auth::validate_password(&password).map_err(|m| anyhow::anyhow!(Lang::En.msg(&m)))?;
    Ok(password)
}

async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let ctrl_c = tokio::signal::ctrl_c();
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::select! {
                _ = ctrl_c => {}
                _ = term.recv() => {}
            }
        }
        Err(_) => {
            let _ = ctrl_c.await;
        }
    }
}
