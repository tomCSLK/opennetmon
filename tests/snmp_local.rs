//! Integration tests against a local net-snmp `snmpd` (see docs/dev-testagent.md).
//! Each test skips itself (with a note on stderr) when the agent or its credentials are not available,
//! so `cargo test` also passes on machines without `snmpd`.
//!
//! v3 tests read `NETMON_TEST_V3_USER`, `NETMON_TEST_V3_AUTH_PASS` and `NETMON_TEST_V3_PRIV_PASS`
//! (SHA-256 / AES-128), e.g. via `set -a; . ~/.config/netmon-dev/test-agent.env; set +a`.

use std::time::Duration;

use netmon::credentials::{AuthProtocol, Credentials, PrivProtocol, Secret};
use netmon::snmp::{SnmpClient, SnmpError, SnmpTarget, SnmpValue, probe};

const IF_HC_IN_OCTETS: &[u64] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 6, 1];

fn target() -> SnmpTarget {
    SnmpTarget {
        host: "127.0.0.1".to_owned(),
        port: 161,
        timeout: Duration::from_secs(2),
        retries: 0,
    }
}

fn v2c() -> Credentials {
    Credentials::V2c {
        community: Secret::new("public"),
    }
}

fn v3_from_env() -> Option<Credentials> {
    Some(Credentials::V3 {
        username: std::env::var("NETMON_TEST_V3_USER").ok()?,
        auth_protocol: AuthProtocol::Sha256,
        auth_password: Secret::new(std::env::var("NETMON_TEST_V3_AUTH_PASS").ok()?),
        privacy: Some((
            PrivProtocol::Aes128,
            Secret::new(std::env::var("NETMON_TEST_V3_PRIV_PASS").ok()?),
        )),
    })
}

async fn agent_available() -> bool {
    let up = probe(&target(), &v2c()).await.is_ok();
    if !up {
        eprintln!("skipped: no snmpd with community 'public' on 127.0.0.1:161");
    }
    up
}

#[tokio::test]
async fn v2c_probe_reads_system_group() {
    if !agent_available().await {
        return;
    }
    let result = probe(&target(), &v2c()).await.unwrap();
    assert!(!result.sys_descr.is_empty());
    assert!(result.uptime_ticks > 0);
}

#[tokio::test]
async fn v2c_with_wrong_community_times_out() {
    if !agent_available().await {
        return;
    }
    let wrong = Credentials::V2c {
        community: Secret::new("definitely-wrong"),
    };
    assert_eq!(
        probe(&target(), &wrong).await.unwrap_err(),
        SnmpError::Timeout
    );
}

#[tokio::test]
async fn v3_auth_priv_probe_matches_v2c() {
    if !agent_available().await {
        return;
    }
    let Some(creds) = v3_from_env() else {
        eprintln!("skipped: NETMON_TEST_V3_* not set");
        return;
    };
    let v3 = probe(&target(), &creds).await.unwrap();
    let v2 = probe(&target(), &v2c()).await.unwrap();
    assert_eq!(v3.sys_descr, v2.sys_descr);
}

#[tokio::test]
async fn v3_with_wrong_password_is_an_authentication_error() {
    if !agent_available().await {
        return;
    }
    let Some(Credentials::V3 {
        username,
        auth_protocol,
        privacy,
        ..
    }) = v3_from_env()
    else {
        eprintln!("skipped: NETMON_TEST_V3_* not set");
        return;
    };
    let wrong = Credentials::V3 {
        username,
        auth_protocol,
        auth_password: Secret::new("wrong-password-1"),
        privacy,
    };
    assert_eq!(probe(&target(), &wrong).await.unwrap_err(), SnmpError::Auth);
}

#[tokio::test]
async fn v3_can_read_interface_counters() {
    if !agent_available().await {
        return;
    }
    let Some(creds) = v3_from_env() else {
        eprintln!("skipped: NETMON_TEST_V3_* not set");
        return;
    };
    let mut client = SnmpClient::connect(&target(), &creds).await.unwrap();
    let values = client.get(&[IF_HC_IN_OCTETS]).await.unwrap();
    assert!(matches!(values[0].1, SnmpValue::Counter64(_)), "{values:?}");
}
