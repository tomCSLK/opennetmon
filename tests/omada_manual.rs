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

/// Runs the real sensors twice against a device and prints what they report.
async fn poll_twice(
    host: &str,
    sensor: &mut dyn netmon::sensors::Sensor,
) -> Vec<(String, Option<f64>)> {
    use netmon::sensors::{PollContext, ping::PingClients};
    let mut client = SnmpClient::connect(&target(host), &credentials())
        .await
        .unwrap();
    let ping = PingClients::new();
    let mut last = Vec::new();
    for round in 0..2 {
        let mut ctx = PollContext {
            host,
            now_ms: i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap(),
            timeout: Duration::from_secs(3),
            ping: &ping,
            snmp: Some(&mut client),
        };
        let m = sensor.poll(&mut ctx).await.unwrap();
        last = m.channels.into_iter().map(|c| (c.key, c.value)).collect();
        println!("{host} round {round}: {last:?}");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    last
}

#[tokio::test]
#[ignore = "needs the real Omada network"]
async fn sensors_work_on_switch_and_access_point() {
    use netmon::sensors::spec::SensorSpec;
    // Switch port 1 has 64-bit counters, the access point's first interface only 32-bit ones.
    for host in [SWITCH, ACCESS_POINT] {
        for spec in [
            SensorSpec::Uptime,
            SensorSpec::InterfaceStatus { if_index: 1 },
            SensorSpec::InterfaceTraffic { if_index: 1 },
            SensorSpec::Ping,
        ] {
            let mut sensor = spec.build();
            let last = poll_twice(host, sensor.as_mut()).await;
            assert!(
                last.iter().all(|(_, v)| v.is_some())
                    || matches!(spec, SensorSpec::InterfaceTraffic { .. }),
                "{host} {spec:?}: {last:?}"
            );
        }
    }
}

/// Full path with a temporary database: profile, devices, sensors, two poll cycles, stored state.
#[tokio::test]
#[ignore = "needs the real Omada network"]
async fn poller_cycle_against_real_devices() {
    use netmon::credentials::{ProfileInput, ProfileKind, SecurityLevel};
    use netmon::crypto::SecretBox;
    use netmon::devices::{DeviceInput, Polling};
    use netmon::poller::{SensorSet, poll_device_once};
    use netmon::sensors::ping::PingClients;
    use netmon::sensors::spec::SensorSpec;
    use netmon::status::Status;
    use netmon::storage::{self, credentials, devices, sensors, status};

    let dir = tempfile::tempdir().unwrap();
    let pool = storage::connect(&dir.path().join("m.db")).await.unwrap();
    storage::migrate(&pool).await.unwrap();
    let secrets = SecretBox::from_key([1u8; 32]);
    let profile = credentials::save_profile(
        &pool,
        &secrets,
        None,
        &ProfileInput {
            name: "omada".to_owned(),
            kind: ProfileKind::V3 {
                username: "MCS6_SNMP".to_owned(),
                level: SecurityLevel::AuthNoPriv,
                auth_protocol: AuthProtocol::Md5,
                auth_password: Some(Secret::new(
                    std::env::var("NETMON_PROBE_AUTH_PASS").expect("NETMON_PROBE_AUTH_PASS"),
                )),
                priv_protocol: None,
                priv_password: None,
            },
        },
    )
    .await
    .unwrap();

    let ping = PingClients::new();
    for (name, host) in [("switch", SWITCH), ("ap", ACCESS_POINT)] {
        let device = devices::save_device(
            &pool,
            None,
            DeviceInput {
                name: name.to_owned(),
                host: host.to_owned(),
                group_id: None,
                profile_id: profile,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let mut ids = Vec::new();
        for (sensor_name, spec) in [
            ("Ping", SensorSpec::Ping),
            ("Uptime", SensorSpec::Uptime),
            ("Port 1 status", SensorSpec::InterfaceStatus { if_index: 1 }),
            (
                "Port 1 traffic",
                SensorSpec::InterfaceTraffic { if_index: 1 },
            ),
        ] {
            ids.push(
                sensors::create_sensor(&pool, device, sensor_name, &spec)
                    .await
                    .unwrap(),
            );
        }
        let mut state = SensorSet::default();
        for _ in 0..2 {
            poll_device_once(&pool, &secrets, &ping, device, &mut state)
                .await
                .unwrap()
                .unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        for id in ids {
            let s = status::get_state(&pool, id).await.unwrap().unwrap();
            println!("{name} sensor {id}: {:?} {}", s.status, s.message);
            assert_eq!(s.status, Status::Up, "{name} sensor {id}: {}", s.message);
        }
    }
}

#[tokio::test]
#[ignore = "needs the real Omada network"]
async fn interface_discovery_on_switch_and_access_point() {
    for host in [SWITCH, ACCESS_POINT] {
        let mut client = SnmpClient::connect(&target(host), &credentials())
            .await
            .unwrap();
        let found = netmon::discovery::discover_interfaces(&mut client)
            .await
            .unwrap();
        println!("{host}: {} interfaces", found.len());
        for i in &found {
            println!(
                "  {:>6} {:<24} {:<8} type={:?} speed={:?} suggested={} alias={:?}",
                i.index,
                i.name,
                i.oper_status_label(),
                i.if_type,
                i.speed_mbps,
                i.suggested(),
                i.alias
            );
        }
        assert!(!found.is_empty());
    }
}
