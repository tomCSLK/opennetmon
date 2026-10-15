use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use netmon::config::Config;
use netmon::storage;
use netmon::web::{self, AppState};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "Agentless SNMP network monitor")]
struct Args {
    /// Path to the TOML config file (defaults and NETMON_* environment variables apply without it)
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = Config::load(args.config.as_deref()).context("loading configuration")?;

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&config.log_filter))
        .context("invalid log filter")?;
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let pool = storage::connect(&config.database_path)
        .await
        .context("opening database")?;
    storage::migrate(&pool)
        .await
        .context("running migrations")?;

    if !config.listen.ip().is_loopback() {
        tracing::warn!(
            listen = %config.listen,
            "listening on a non-loopback address; put a TLS-terminating reverse proxy in front"
        );
    }
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("binding {}", config.listen))?;
    tracing::info!(listen = %config.listen, db = %config.database_path.display(), "netmon started");

    axum::serve(listener, web::router(AppState { pool }))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving")?;
    tracing::info!("netmon stopped");
    Ok(())
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
