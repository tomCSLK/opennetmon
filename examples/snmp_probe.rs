//! Phase 0 spike: probes an SNMP agent with `snmp2` (v2c and v3 authPriv).
//!
//! Secrets are read from the environment only, never from the command line:
//!   NETMON_PROBE_COMMUNITY                    (v2c)
//!   NETMON_PROBE_AUTH_PASS, NETMON_PROBE_PRIV_PASS (v3; the priv password only with `--level auth-priv`)
//!
//! Example:
//!   NETMON_PROBE_COMMUNITY=public cargo run --example snmp_probe -- -t 127.0.0.1:161 v2c
//!   cargo run --example snmp_probe -- -t 127.0.0.1:161 v3 -u netmontest --auth sha256 --priv aes128

use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use snmp2::v3::{Auth, AuthProtocol, Cipher, Security};
use snmp2::{AsyncSession, Oid, Value};

const SYS_DESCR: &[u64] = &[1, 3, 6, 1, 2, 1, 1, 1, 0];
const SYS_UPTIME: &[u64] = &[1, 3, 6, 1, 2, 1, 1, 3, 0];
const IF_HC_IN_OCTETS: &[u64] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 6];
const IF_TABLE: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2];
const IF_X_TABLE: &[u64] = &[1, 3, 6, 1, 2, 1, 31, 1, 1];

#[derive(Parser)]
#[command(about = "Phase 0 SNMP probe (read-only)")]
struct Cli {
    /// Target as host:port
    #[arg(short, long, default_value = "127.0.0.1:161")]
    target: String,
    /// Per-request timeout in seconds
    #[arg(long, default_value_t = 5)]
    timeout: u64,
    /// GETBULK max-repetitions
    #[arg(long, default_value_t = 20)]
    max_repetitions: u32,
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// SNMPv2c (community from NETMON_PROBE_COMMUNITY)
    V2c,
    /// SNMPv3 (passwords from NETMON_PROBE_AUTH_PASS / NETMON_PROBE_PRIV_PASS)
    V3 {
        #[arg(short, long)]
        user: String,
        /// Security level (Omada switches use auth-no-priv)
        #[arg(long, value_enum, default_value = "auth-priv")]
        level: LevelArg,
        #[arg(long, value_enum, default_value = "sha256")]
        auth: AuthArg,
        #[arg(long = "priv", value_enum, default_value = "aes128")]
        priv_: PrivArg,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum LevelArg {
    AuthNoPriv,
    AuthPriv,
}

#[derive(Clone, Copy, ValueEnum)]
enum AuthArg {
    Md5,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

#[derive(Clone, Copy, ValueEnum)]
enum PrivArg {
    Des,
    Aes128,
    Aes192,
    Aes256,
}

impl From<AuthArg> for AuthProtocol {
    fn from(a: AuthArg) -> Self {
        match a {
            AuthArg::Md5 => Self::Md5,
            AuthArg::Sha1 => Self::Sha1,
            AuthArg::Sha224 => Self::Sha224,
            AuthArg::Sha256 => Self::Sha256,
            AuthArg::Sha384 => Self::Sha384,
            AuthArg::Sha512 => Self::Sha512,
        }
    }
}

impl From<PrivArg> for Cipher {
    fn from(p: PrivArg) -> Self {
        match p {
            PrivArg::Des => Self::Des,
            PrivArg::Aes128 => Self::Aes128,
            PrivArg::Aes192 => Self::Aes192,
            PrivArg::Aes256 => Self::Aes256,
        }
    }
}

fn oid(parts: &[u64]) -> Result<Oid<'static>> {
    Oid::from(parts).map_err(|e| anyhow!("invalid OID: {e:?}"))
}

fn env_secret(name: &str) -> Result<Vec<u8>> {
    let v =
        std::env::var(name).with_context(|| format!("environment variable {name} is not set"))?;
    Ok(v.into_bytes())
}

fn show(v: &Value<'_>) -> String {
    match v {
        Value::OctetString(b) => String::from_utf8_lossy(b).into_owned(),
        Value::Counter64(n) => n.to_string(),
        Value::Counter32(n) | Value::Unsigned32(n) | Value::Timeticks(n) => n.to_string(),
        Value::Integer(n) => n.to_string(),
        other => format!("{other:?}"),
    }
}

async fn with_timeout<T>(
    secs: u64,
    fut: impl Future<Output = Result<T, snmp2::Error>>,
) -> Result<T> {
    match tokio::time::timeout(Duration::from_secs(secs), fut).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(anyhow!("SNMP error: {e:?}")),
        Err(_) => Err(anyhow!("timeout after {secs}s")),
    }
}

async fn open(cli: &Cli) -> Result<AsyncSession> {
    match &cli.mode {
        Mode::V2c => {
            let community = env_secret("NETMON_PROBE_COMMUNITY")?;
            Ok(AsyncSession::new_v2c(cli.target.as_str(), &community, 1).await?)
        }
        Mode::V3 {
            user,
            level,
            auth,
            priv_,
        } => {
            let auth_pass = env_secret("NETMON_PROBE_AUTH_PASS")?;
            let auth_mode = match level {
                LevelArg::AuthNoPriv => Auth::AuthNoPriv,
                LevelArg::AuthPriv => Auth::AuthPriv {
                    cipher: (*priv_).into(),
                    privacy_password: env_secret("NETMON_PROBE_PRIV_PASS")?,
                },
            };
            let security = Security::new(user.as_bytes(), &auth_pass)
                .with_auth_protocol((*auth).into())
                .with_auth(auth_mode);
            let mut session = AsyncSession::new_v3(cli.target.as_str(), 1, security).await?;
            with_timeout(cli.timeout, session.init())
                .await
                .context("SNMPv3 engine discovery failed")?;
            Ok(session)
        }
    }
}

/// GETBULK walk of everything below `root`.
async fn walk(
    session: &mut AsyncSession,
    root: &Oid<'static>,
    cli: &Cli,
) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut cursor = root.clone();
    loop {
        let pdu = with_timeout(
            cli.timeout,
            session.getbulk(&[&cursor], 0, cli.max_repetitions),
        )
        .await?;
        if pdu.error_status != 0 {
            bail!("agent returned error status {}", pdu.error_status);
        }
        let mut last = None;
        let mut finished = false;
        for (o, v) in pdu.varbinds {
            if matches!(v, Value::EndOfMibView) || !o.starts_with(root) {
                finished = true;
                break;
            }
            out.push((o.to_id_string(), show(&v)));
            last = Some(o.to_owned());
        }
        match last {
            Some(next) if !finished => cursor = next,
            _ => return Ok(out),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut session = open(&cli).await?;

    for (name, parts) in [("sysDescr", SYS_DESCR), ("sysUpTime", SYS_UPTIME)] {
        let o = oid(parts)?;
        let pdu = with_timeout(cli.timeout, session.get(&o)).await?;
        for (_, v) in pdu.varbinds {
            println!("{name:<10} {}", show(&v));
        }
    }

    let hc = walk(&mut session, &oid(IF_HC_IN_OCTETS)?, &cli).await?;
    println!("ifHCInOctets: {} interfaces", hc.len());
    for (o, v) in hc.iter().take(3) {
        println!("  {o} = {v}");
    }

    let if_x = walk(&mut session, &oid(IF_X_TABLE)?, &cli).await?;
    println!("ifXTable walk: {} varbinds", if_x.len());
    let if_t = walk(&mut session, &oid(IF_TABLE)?, &cli).await?;
    println!("ifTable walk:  {} varbinds", if_t.len());
    Ok(())
}
