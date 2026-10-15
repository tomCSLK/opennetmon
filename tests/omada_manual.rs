//! Manual tests against the real Omada devices; never part of `cargo test` (all tests are `#[ignore]`).
//!
//! ```bash
//! set -a; . ~/.config/netmon-dev/omada.env; set +a
//! cargo test --test omada_manual -- --ignored --nocapture
//! ```
//! Needs `NETMON_PROBE_AUTH_PASS` (the SNMPv3 password of user `MCS6_SNMP`, authNoPriv/MD5).

use std::time::Duration;

use netmon::credentials::{AuthProtocol, Credentials, Secret};
use netmon::snmp::{SnmpClient, SnmpTarget, SnmpValue, probe};

const SWITCH: &str = "192.168.178.253";
const ACCESS_POINT: &str = "192.168.178.240";
const IF_HC_IN_OCTETS_FIRST: &[u64] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 6, 1];
const IF_IN_OCTETS_FIRST: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 10, 1];

fn credentials() -> Credentials {
    Credentials::V3 {
        username: "MCS6_SNMP".to_owned(),
        auth_protocol: AuthProtocol::Md5,
        auth_password: Secret::new(
            std::env::var("NETMON_PROBE_AUTH_PASS").expect("NETMON_PROBE_AUTH_PASS"),
        ),
        privacy: None,
    }
}

fn target(host: &str) -> SnmpTarget {
    SnmpTarget {
        host: host.to_owned(),
        port: 161,
        timeout: Duration::from_secs(3),
        retries: 1,
    }
}

#[tokio::test]
#[ignore = "needs the real Omada network"]
async fn switch_answers_with_64_bit_counters() {
    let result = probe(&target(SWITCH), &credentials()).await.unwrap();
    println!(
        "{} | {} ms",
        result.sys_descr,
        result.round_trip.as_millis()
    );
    assert!(result.sys_descr.contains("Omada"));
    let mut client = SnmpClient::connect(&target(SWITCH), &credentials())
        .await
        .unwrap();
    let values = client.get(&[IF_HC_IN_OCTETS_FIRST]).await.unwrap();
    assert!(matches!(values[0].1, SnmpValue::Counter64(_)), "{values:?}");
}

#[tokio::test]
#[ignore = "needs the real Omada network"]
async fn access_point_answers_with_32_bit_counters_only() {
    let result = probe(&target(ACCESS_POINT), &credentials()).await.unwrap();
    println!("{}", result.sys_descr);
    let mut client = SnmpClient::connect(&target(ACCESS_POINT), &credentials())
        .await
        .unwrap();
    let values = client
        .get(&[IF_IN_OCTETS_FIRST, IF_HC_IN_OCTETS_FIRST])
        .await
        .unwrap();
    assert!(matches!(values[0].1, SnmpValue::Counter32(_)), "{values:?}");
    assert!(
        matches!(
            values[1].1,
            SnmpValue::NoSuchObject | SnmpValue::NoSuchInstance
        ),
        "{values:?}"
    );
}
