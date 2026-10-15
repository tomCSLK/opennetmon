//! Thin async SNMP client on top of `snmp2` (read-only). `snmp2` has no timeouts of its own, so every
//! request is wrapped in `tokio::time::timeout` here; the poller will use the same client.

#[cfg(test)]
pub(crate) mod testagent;

use std::time::{Duration, Instant};

use snmp2::v3::{Auth, AuthProtocol as WireAuth, Cipher, Security};
use snmp2::{AsyncSession, Oid, Value};

use crate::credentials::{AuthProtocol, Credentials, PrivProtocol};

pub const SYS_DESCR: &[u64] = &[1, 3, 6, 1, 2, 1, 1, 1, 0];
pub const SYS_UPTIME: &[u64] = &[1, 3, 6, 1, 2, 1, 1, 3, 0];

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
    /// Explanation for the UI (German); never contains secrets.
    pub fn user_message(&self) -> String {
        match self {
            Self::Timeout => "Keine Antwort innerhalb des Timeouts. Mögliche Ursachen: Gerät oder Port nicht erreichbar, \
                              SNMP nicht aktiviert, falsche Community oder falsche SNMPv3-Benutzerdaten."
                .to_owned(),
            Self::Auth => "Anmeldung fehlgeschlagen. Benutzer, Passwort, Authentifizierungs- oder Verschlüsselungsverfahren \
                           stimmen nicht mit dem Gerät überein."
                .to_owned(),
            Self::Unreachable => "Das Gerät ist nicht erreichbar oder der SNMP-Port ist geschlossen.".to_owned(),
            Self::Request(m) => format!("Die Anfrage konnte nicht aufgebaut werden: {m}"),
            Self::Protocol(m) => format!("Unerwartete Antwort des Geräts: {m}"),
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
pub fn format_uptime(ticks: u32) -> String {
    let seconds = u64::from(ticks) / 100;
    let (days, hours, minutes) = (
        seconds / 86_400,
        (seconds % 86_400) / 3_600,
        (seconds % 3_600) / 60,
    );
    match (days, hours) {
        (0, 0) => format!("{minutes} Min."),
        (0, _) => format!("{hours} Std. {minutes} Min."),
        (1, _) => format!("1 Tag {hours} Std. {minutes} Min."),
        _ => format!("{days} Tage {hours} Std. {minutes} Min."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::Secret;

    #[test]
    fn uptime_is_formatted_for_humans() {
        assert_eq!(format_uptime(0), "0 Min.");
        assert_eq!(format_uptime(59 * 6000 + 5900), "59 Min.");
        assert_eq!(format_uptime(90 * 6000), "1 Std. 30 Min.");
        assert_eq!(
            format_uptime(86_400 * 100 + 3 * 360_000 + 12 * 6000),
            "1 Tag 3 Std. 12 Min."
        );
        assert_eq!(format_uptime(1_618_844_685), "187 Tage 8 Std. 47 Min.");
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
            assert!(!e.user_message().is_empty());
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
}
