//! Thin async SNMP client on top of `snmp2` (read-only). `snmp2` has no timeouts of its own, so every
//! request is wrapped in `tokio::time::timeout` here; the poller will use the same client.

#[cfg(test)]
pub(crate) mod testagent;

use std::time::{Duration, Instant};

use snmp2::v3::{Auth, AuthProtocol as WireAuth, Cipher, Security};
use snmp2::{AsyncSession, Oid, Value};

use crate::credentials::{AuthProtocol, Credentials, PrivProtocol};
use crate::i18n::{Lang, Msg};

pub const SYS_DESCR: &[u64] = &[1, 3, 6, 1, 2, 1, 1, 1, 0];
pub const SYS_UPTIME: &[u64] = &[1, 3, 6, 1, 2, 1, 1, 3, 0];
/// The vendor's identification of the device model (an OID below the vendor's enterprise number).
pub const SYS_OBJECT_ID: &[u64] = &[1, 3, 6, 1, 2, 1, 1, 2, 0];

#[derive(Debug, Clone)]
pub struct SnmpTarget {
    pub host: String,
    pub port: u16,
    /// Timeout per attempt.
    pub timeout: Duration,
    /// Additional attempts after a timeout.
    pub retries: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnmpError {
    #[error("no response within the timeout")]
    Timeout,
    #[error("authentication failed")]
    Auth,
    #[error("cannot reach the device")]
    Unreachable,
    #[error("invalid request: {0}")]
    Request(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

impl SnmpError {
    /// Explanation for the UI; never contains secrets.
    pub fn user_message(&self) -> Msg {
        match self {
            Self::Timeout => Msg::new("snmp-timeout"),
            Self::Auth => Msg::new("snmp-auth"),
            Self::Unreachable => Msg::new("snmp-unreachable"),
            Self::Request(m) => Msg::new("snmp-request").with("detail", m),
            Self::Protocol(m) => Msg::new("snmp-protocol").with("detail", m),
        }
    }
}

impl From<snmp2::Error> for SnmpError {
    fn from(e: snmp2::Error) -> Self {
        use snmp2::Error as E;
        match e {
            E::AuthFailure(_) => Self::Auth,
            E::CommunityMismatch => Self::Auth,
            E::Send | E::Receive => Self::Unreachable,
            other => Self::Protocol(format!("{other:?}")),
        }
    }
}

/// An SNMP value, owned and independent of the receive buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnmpValue {
    Text(String),
    Integer(i64),
    Counter32(u32),
    Counter64(u64),
    Gauge32(u32),
    Timeticks(u32),
    /// An OBJECT IDENTIFIER value in dotted form, e.g. `sysObjectID`.
    ObjectId(String),
    Other(String),
    NoSuchObject,
    NoSuchInstance,
    EndOfMibView,
}

impl From<&Value<'_>> for SnmpValue {
    fn from(v: &Value<'_>) -> Self {
        match v {
            Value::OctetString(b) => Self::Text(String::from_utf8_lossy(b).into_owned()),
            Value::Integer(n) => Self::Integer(*n),
            Value::Counter32(n) => Self::Counter32(*n),
            Value::Counter64(n) => Self::Counter64(*n),
            Value::Unsigned32(n) => Self::Gauge32(*n),
            Value::Timeticks(n) => Self::Timeticks(*n),
            Value::NoSuchObject => Self::NoSuchObject,
            Value::NoSuchInstance => Self::NoSuchInstance,
            Value::EndOfMibView => Self::EndOfMibView,
            Value::ObjectIdentifier(oid) => Self::ObjectId(oid.to_id_string()),
            other => Self::Other(format!("{other:?}")),
        }
    }
}

fn wire_auth(p: AuthProtocol) -> WireAuth {
    match p {
        AuthProtocol::Md5 => WireAuth::Md5,
        AuthProtocol::Sha1 => WireAuth::Sha1,
        AuthProtocol::Sha224 => WireAuth::Sha224,
        AuthProtocol::Sha256 => WireAuth::Sha256,
        AuthProtocol::Sha384 => WireAuth::Sha384,
        AuthProtocol::Sha512 => WireAuth::Sha512,
    }
}

fn wire_cipher(p: PrivProtocol) -> Cipher {
    match p {
        PrivProtocol::Des => Cipher::Des,
        PrivProtocol::Aes128 => Cipher::Aes128,
        PrivProtocol::Aes192 => Cipher::Aes192,
        PrivProtocol::Aes256 => Cipher::Aes256,
    }
}

pub struct SnmpClient {
    session: AsyncSession,
    timeout: Duration,
    retries: u32,
}

impl SnmpClient {
    /// Opens a session. For SNMPv3 this includes the engine discovery round trip.
    pub async fn connect(
        target: &SnmpTarget,
        credentials: &Credentials,
    ) -> Result<Self, SnmpError> {
        let address = (target.host.as_str(), target.port);
        let unreachable = |_| SnmpError::Unreachable;
        let session = match credentials {
            Credentials::V2c { community } => {
                AsyncSession::new_v2c(address, community.expose().as_bytes(), 1)
                    .await
                    .map_err(unreachable)?
            }
            Credentials::V3 {
                username,
                auth_protocol,
                auth_password,
                privacy,
            } => {
                let mode = match privacy {
                    Some((protocol, password)) => Auth::AuthPriv {
                        cipher: wire_cipher(*protocol),
                        privacy_password: password.expose().as_bytes().to_vec(),
                    },
                    None => Auth::AuthNoPriv,
                };
                let security =
                    Security::new(username.as_bytes(), auth_password.expose().as_bytes())
                        .with_auth_protocol(wire_auth(*auth_protocol))
                        .with_auth(mode);
                let mut session = AsyncSession::new_v3(address, 1, security)
                    .await
                    .map_err(unreachable)?;
                match tokio::time::timeout(target.timeout, session.init()).await {
                    Ok(result) => result?,
                    Err(_) => return Err(SnmpError::Timeout),
                }
                session
            }
        };
        Ok(Self {
            session,
            timeout: target.timeout,
            retries: target.retries,
        })
    }

    /// GET for several scalar OIDs. Retries on timeouts (and once on a v3 context refresh).
    pub async fn get(&mut self, oids: &[&[u64]]) -> Result<Vec<(String, SnmpValue)>, SnmpError> {
        let oids: Vec<Oid<'static>> = oids
            .iter()
            .map(|parts| {
                Oid::from(parts).map_err(|e| SnmpError::Request(format!("invalid OID: {e:?}")))
            })
            .collect::<Result<_, _>>()?;
        let refs: Vec<&Oid<'_>> = oids.iter().collect();

        let mut attempts_left = self.retries + 1;
        let mut refreshed = false;
        loop {
            attempts_left -= 1;
            match tokio::time::timeout(self.timeout, self.session.get_many(&refs)).await {
                Ok(Ok(pdu)) => {
                    if pdu.error_status != 0 {
                        return Err(SnmpError::Protocol(format!(
                            "agent error status {}",
                            pdu.error_status
                        )));
                    }
                    return Ok(pdu
                        .varbinds
                        .map(|(oid, value)| (oid.to_id_string(), SnmpValue::from(&value)))
                        .collect());
                }
                Ok(Err(snmp2::Error::AuthUpdated)) if !refreshed => refreshed = true,
                Ok(Err(e)) => return Err(e.into()),
                Err(_) if attempts_left > 0 => {}
                Err(_) => return Err(SnmpError::Timeout),
            }
        }
    }
}

impl SnmpClient {
    /// Walks everything below `root` with GETBULK (rows in OID order). More than `max_rows` rows is an
    /// error rather than a silently truncated result; an agent that answers with a non-increasing OID
    /// ends the walk with the rows read so far.
    pub async fn walk(
        &mut self,
        root: &[u64],
        max_rows: usize,
    ) -> Result<Vec<(Vec<u64>, SnmpValue)>, SnmpError> {
        const REPETITIONS: u32 = 20;
        let root_oid =
            Oid::from(root).map_err(|e| SnmpError::Request(format!("invalid OID: {e:?}")))?;
        let mut cursor: Oid<'static> = root_oid.clone();
        let mut rows: Vec<(Vec<u64>, SnmpValue)> = Vec::new();
        'requests: loop {
            let mut attempts_left = self.retries + 1;
            let mut refreshed = false;
            let pdu = loop {
                attempts_left -= 1;
                match tokio::time::timeout(
                    self.timeout,
                    self.session.getbulk(&[&cursor], 0, REPETITIONS),
                )
                .await
                {
                    Ok(Ok(pdu)) => break pdu,
                    Ok(Err(snmp2::Error::AuthUpdated)) if !refreshed => refreshed = true,
                    Ok(Err(e)) => return Err(e.into()),
                    Err(_) if attempts_left > 0 => {}
                    Err(_) => return Err(SnmpError::Timeout),
                }
            };
            if pdu.error_status != 0 {
                return Err(SnmpError::Protocol(format!(
                    "agent error status {}",
                    pdu.error_status
                )));
            }
            let mut advanced = false;
            for (oid, value) in pdu.varbinds {
                if matches!(value, Value::EndOfMibView) || !oid.starts_with(&root_oid) {
                    break 'requests;
                }
                let parts = parse_id_string(&oid.to_id_string())?;
                if rows.last().is_some_and(|(last, _)| *last >= parts) {
                    // Some agents (e.g. TP-Link firmware) go backwards near the end of a subtree.
                    // Keep what was read instead of discarding the whole walk.
                    tracing::debug!(oid = %oid.to_id_string(), "agent returned a non-increasing OID; walk ended early");
                    break 'requests;
                }
                if rows.len() >= max_rows {
                    return Err(SnmpError::Protocol(format!("more than {max_rows} rows")));
                }
                cursor = oid.to_owned();
                rows.push((parts, SnmpValue::from(&value)));
                advanced = true;
            }
            if !advanced {
                break;
            }
        }
        Ok(rows)
    }
}

fn parse_id_string(id: &str) -> Result<Vec<u64>, SnmpError> {
    id.split('.')
        .map(|p| p.parse::<u64>())
        .collect::<Result<_, _>>()
        .map_err(|_| SnmpError::Protocol(format!("unparsable OID {id}")))
}

/// Result of a connection test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    pub sys_descr: String,
    pub uptime_ticks: u32,
    pub round_trip: Duration,
}

/// Asks the device for `sysDescr` and `sysUpTime`.
pub async fn probe(
    target: &SnmpTarget,
    credentials: &Credentials,
) -> Result<ProbeResult, SnmpError> {
    let started = Instant::now();
    let mut client = SnmpClient::connect(target, credentials).await?;
    let values = client.get(&[SYS_DESCR, SYS_UPTIME]).await?;
    let round_trip = started.elapsed();

    let find = |oid: &[u64]| {
        let id = oid.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
        values
            .iter()
            .find(|(o, _)| *o == id)
            .map(|(_, v)| v.clone())
    };
    match (find(SYS_DESCR), find(SYS_UPTIME)) {
        (Some(SnmpValue::Text(sys_descr)), Some(SnmpValue::Timeticks(uptime_ticks))) => {
            Ok(ProbeResult {
                sys_descr,
                uptime_ticks,
                round_trip,
            })
        }
        _ => Err(SnmpError::Protocol(
            "sysDescr/sysUpTime fehlen oder haben einen unerwarteten Typ".to_owned(),
        )),
    }
}

/// Formats TimeTicks (1/100 s) as e.g. `187 Tage 3 Std. 12 Min.`.
pub fn format_uptime(t: Lang, ticks: u32) -> String {
    let seconds = u64::from(ticks) / 100;
    let (days, hours, minutes) = (
        seconds / 86_400,
        (seconds % 86_400) / 3_600,
        (seconds % 3_600) / 60,
    );
    match (days, hours) {
        (0, 0) => t.get1("duration-minutes", "minutes", minutes),
        (0, _) => t.get2("duration-hours-minutes", "hours", hours, "minutes", minutes),
        _ => t.fmt(
            "duration-days-hours-minutes",
            &[
                ("days", days.into()),
                ("hours", hours.into()),
                ("minutes", minutes.into()),
            ],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::Secret;

    #[test]
    fn uptime_is_formatted_for_humans() {
        assert_eq!(format_uptime(Lang::De, 0), "0 Min.");
        assert_eq!(format_uptime(Lang::De, 59 * 6000 + 5900), "59 Min.");
        assert_eq!(format_uptime(Lang::De, 90 * 6000), "1 Std. 30 Min.");
        assert_eq!(
            format_uptime(Lang::De, 86_400 * 100 + 3 * 360_000 + 12 * 6000),
            "1 Tag 3 Std. 12 Min."
        );
        assert_eq!(
            format_uptime(Lang::De, 1_618_844_685),
            "187 Tage 8 Std. 47 Min."
        );
        assert_eq!(
            format_uptime(Lang::En, 86_400 * 100 + 3 * 360_000 + 12 * 6000),
            "1 day 3 h 12 min"
        );
    }

    #[test]
    fn library_errors_map_to_user_facing_categories() {
        use snmp2::v3::AuthErrorKind;
        assert_eq!(
            SnmpError::from(snmp2::Error::AuthFailure(AuthErrorKind::NotAuthenticated)),
            SnmpError::Auth
        );
        assert_eq!(
            SnmpError::from(snmp2::Error::CommunityMismatch),
            SnmpError::Auth
        );
        assert_eq!(
            SnmpError::from(snmp2::Error::Receive),
            SnmpError::Unreachable
        );
        assert_eq!(SnmpError::from(snmp2::Error::Send), SnmpError::Unreachable);
        assert!(matches!(
            SnmpError::from(snmp2::Error::AsnParse),
            SnmpError::Protocol(_)
        ));
        for e in [SnmpError::Timeout, SnmpError::Auth, SnmpError::Unreachable] {
            assert!(!Lang::En.msg(&e.user_message()).is_empty());
        }
    }

    #[test]
    fn values_are_converted_to_owned_variants() {
        assert_eq!(
            SnmpValue::from(&Value::OctetString(b"Omada")),
            SnmpValue::Text("Omada".to_owned())
        );
        assert_eq!(
            SnmpValue::from(&Value::Counter64(7)),
            SnmpValue::Counter64(7)
        );
        assert_eq!(
            SnmpValue::from(&Value::Timeticks(5)),
            SnmpValue::Timeticks(5)
        );
        assert_eq!(
            SnmpValue::from(&Value::NoSuchObject),
            SnmpValue::NoSuchObject
        );
        assert_eq!(
            SnmpValue::from(&Value::EndOfMibView),
            SnmpValue::EndOfMibView
        );
    }

    #[test]
    fn debug_output_of_credentials_hides_secrets() {
        let creds = Credentials::V3 {
            username: "u".to_owned(),
            auth_protocol: AuthProtocol::Md5,
            auth_password: Secret::new("topsecret-1"),
            privacy: Some((PrivProtocol::Aes128, Secret::new("topsecret-2"))),
        };
        let shown = format!("{creds:?}");
        assert!(!shown.contains("topsecret"));
    }

    fn target(port: u16) -> SnmpTarget {
        SnmpTarget {
            host: "127.0.0.1".to_owned(),
            port,
            timeout: Duration::from_millis(400),
            retries: 0,
        }
    }

    #[tokio::test]
    async fn a_silent_port_times_out_and_a_closed_port_is_unreachable() {
        // A bound but silent UDP socket never answers.
        let silent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = silent.local_addr().unwrap().port();
        let creds = Credentials::V2c {
            community: Secret::new("public"),
        };
        assert_eq!(
            probe(&target(port), &creds).await.unwrap_err(),
            SnmpError::Timeout
        );
        drop(silent);

        // Nobody is listening any more: the kernel answers with ICMP port unreachable.
        let err = probe(&target(port), &creds).await.unwrap_err();
        assert!(
            matches!(err, SnmpError::Unreachable | SnmpError::Timeout),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn probe_succeeds_against_a_v2c_agent() {
        use crate::snmp::testagent::FakeAgent;
        let agent =
            FakeAgent::spawn("public", "Omada 8-Port Gigabit Smart Switch", 1_618_844_685).await;
        let creds = Credentials::V2c {
            community: Secret::new("public"),
        };
        let mut t = target(agent.port);
        t.timeout = Duration::from_secs(2);
        let result = probe(&t, &creds).await.unwrap();
        assert_eq!(result.sys_descr, "Omada 8-Port Gigabit Smart Switch");
        assert_eq!(result.uptime_ticks, 1_618_844_685);
        assert!(result.round_trip < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn a_wrong_community_gets_no_answer() {
        use crate::snmp::testagent::FakeAgent;
        let agent = FakeAgent::spawn("public", "x", 1).await;
        let creds = Credentials::V2c {
            community: Secret::new("falsch"),
        };
        assert_eq!(
            probe(&target(agent.port), &creds).await.unwrap_err(),
            SnmpError::Timeout
        );
    }

    #[tokio::test]
    async fn get_reports_unknown_oids_as_no_such_instance() {
        use crate::snmp::testagent::FakeAgent;
        let agent = FakeAgent::spawn("public", "x", 7).await;
        let creds = Credentials::V2c {
            community: Secret::new("public"),
        };
        let mut t = target(agent.port);
        t.timeout = Duration::from_secs(2);
        let mut client = SnmpClient::connect(&t, &creds).await.unwrap();
        let values = client
            .get(&[SYS_UPTIME, &[1, 3, 6, 1, 2, 1, 1, 5, 0]])
            .await
            .unwrap();
        assert_eq!(
            values[0],
            ("1.3.6.1.2.1.1.3.0".to_owned(), SnmpValue::Timeticks(7))
        );
        assert_eq!(values[1].1, SnmpValue::NoSuchInstance);
    }

    #[tokio::test]
    async fn unresolvable_hosts_are_reported_as_unreachable() {
        let creds = Credentials::V2c {
            community: Secret::new("public"),
        };
        let t = SnmpTarget {
            host: "does-not-exist.invalid".to_owned(),
            ..target(161)
        };
        assert_eq!(probe(&t, &creds).await.unwrap_err(), SnmpError::Unreachable);
    }

    async fn walk_client(agent: &crate::snmp::testagent::FakeAgent) -> SnmpClient {
        let creds = Credentials::V2c {
            community: Secret::new("public"),
        };
        let mut t = target(agent.port);
        t.timeout = Duration::from_secs(2);
        SnmpClient::connect(&t, &creds).await.unwrap()
    }

    #[tokio::test]
    async fn walk_returns_exactly_the_subtree_in_order_over_several_requests() {
        use crate::snmp::testagent::{FakeAgent, FakeValue};
        let agent = FakeAgent::spawn("public", "x", 1).await;
        for i in (1..=45u64).rev() {
            agent.set(
                &[1, 3, 6, 1, 2, 1, 2, 2, 1, 2, i],
                FakeValue::Text(format!("port{i}")),
            );
        }
        agent.set(&[1, 3, 6, 1, 2, 1, 2, 2, 1, 3, 1], FakeValue::Integer(6)); // sibling column
        let mut client = walk_client(&agent).await;
        let rows = client
            .walk(&[1, 3, 6, 1, 2, 1, 2, 2, 1, 2], 100)
            .await
            .unwrap();
        assert_eq!(rows.len(), 45);
        assert_eq!(
            rows[0],
            (
                vec![1, 3, 6, 1, 2, 1, 2, 2, 1, 2, 1],
                SnmpValue::Text("port1".to_owned())
            )
        );
        assert!(rows.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[tokio::test]
    async fn walk_of_an_empty_subtree_and_the_end_of_the_mib_is_empty() {
        use crate::snmp::testagent::FakeAgent;
        let agent = FakeAgent::spawn("public", "x", 1).await;
        let mut client = walk_client(&agent).await;
        assert!(
            client
                .walk(&[1, 3, 6, 1, 4, 1, 77], 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(client.walk(&[1, 3, 6, 1, 9], 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn walk_fails_instead_of_truncating() {
        use crate::snmp::testagent::{FakeAgent, FakeValue};
        let agent = FakeAgent::spawn("public", "x", 1).await;
        for i in 1..=30u64 {
            agent.set(&[1, 3, 6, 1, 4, 1, 5, i], FakeValue::Integer(1));
        }
        let mut client = walk_client(&agent).await;
        let err = client.walk(&[1, 3, 6, 1, 4, 1, 5], 10).await.unwrap_err();
        assert!(matches!(err, SnmpError::Protocol(_)), "{err:?}");
    }
}
