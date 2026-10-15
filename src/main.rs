use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};
use netmon::auth::{self, Role};
use netmon::config::Config;
use netmon::crypto::SecretBox;
use netmon::maintenance::Retention;
use netmon::poller::Poller;
use netmon::storage::{self, users};
use netmon::web::{self, AppState};
use sqlx::SqlitePool;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "Agentless SNMP network monitor")]
struct Args {
    /// Path to the TOML config file (defaults and NETMON_* environment variables apply without it)
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
enum UserCommand {
    /// Create a user; the password is prompted for (or read from stdin when piped)
    Add {
        username: String,
        #[arg(long, value_enum, default_value = "viewer")]
        role: RoleArg,
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
        Command::Key(_) | Command::Backup { .. } => Ok(()),
    }
}

async fn serve(config: Config, pool: SqlitePool) -> anyhow::Result<()> {
    let secrets = SecretBox::from_key_file(&config.secret_key_file).with_context(|| {
        format!(
            "loading the encryption key (create one with `netmon key generate`; config key secret_key_file = {})",
            config.secret_key_file.display()
        )
    })?;
    if users::count_users(&pool).await? == 0 {
        tracing::warn!(
            "no users exist yet; create an admin with `netmon user add <name> --role admin`"
        );
    }
    if !config.listen.ip().is_loopback() {
        tracing::warn!(
            listen = %config.listen,
            "listening on a non-loopback address; put a TLS-terminating reverse proxy in front"
        );
    }
    if !config.cookie_secure {
        tracing::warn!("cookie_secure is disabled; only use this for plain-HTTP development");
    }
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("binding {}", config.listen))?;
    tracing::info!(listen = %config.listen, db = %config.database_path.display(), "netmon started");

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

    let served = axum::serve(listener, web::router(AppState::new(pool, &config, secrets)))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving");
    if let Some(poller) = poller {
        poller.stop().await;
    }
    served?;
    tracing::info!("netmon stopped");
    Ok(())
}

async fn manage_users(pool: &SqlitePool, cmd: UserCommand) -> anyhow::Result<()> {
    match cmd {
        UserCommand::Add { username, role } => {
            auth::validate_username(&username).map_err(anyhow::Error::msg)?;
            let password = read_new_password()?;
            let hash = auth::hash_password(&password)?;
            users::create_user(pool, &username, &hash, role.into()).await?;
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
    auth::validate_password(&password).map_err(anyhow::Error::msg)?;
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
