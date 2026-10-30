//! Sensors for services rather than devices: HTTP(S), TCP port, DNS server and TLS certificate.
//! They need no SNMP; the address is the device's host (the URL of an HTTP sensor is its own).

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::dns::{self, RecordType};
use super::{ChannelReading, Measurement, PollContext, PollFuture, Sensor, SensorError, Unit};
use crate::http_client::{self, HttpError};
use crate::i18n::Msg;

/// Bytes of an HTTP body searched for the keyword.
pub const MAX_BODY: usize = 1024 * 1024;
pub const MAX_KEYWORD_CHARS: usize = 200;
/// Default of the expected HTTP status codes: success and redirects.
pub const DEFAULT_EXPECT: &str = "200-399";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpCheck {
    pub url: String,
    /// HEAD instead of GET (no body, so no keyword).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub head: bool,
    /// Status codes that count as fine, e.g. `200-299,301`.
    pub expect: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    pub follow_redirects: bool,
    pub verify_tls: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsCheck {
    pub name: String,
    pub record: RecordType,
    /// Port of the DNS server (the device).
    pub port: u16,
    /// An address the answer must contain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<IpAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsCheck {
    pub port: u16,
    /// Name sent as SNI and checked against the certificate; `None` = the device's host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    /// Check that the chain is trusted and the name matches.
    pub verify: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpCheck {
    pub port: u16,
}

/// Parses expected status codes: codes and ranges separated by commas, e.g. `200-299, 301`.
pub fn parse_expect(text: &str) -> Result<Vec<(u16, u16)>, Msg> {
    let invalid = || Msg::new("service-expect-invalid");
    let code = |s: &str| {
        s.trim()
            .parse::<u16>()
            .ok()
            .filter(|c| (100..=599).contains(c))
            .ok_or_else(invalid)
    };
    let ranges = text
        .split(',')
        .map(|part| match part.split_once('-') {
            Some((from, to)) => {
                let (from, to) = (code(from)?, code(to)?);
                if from > to {
                    return Err(invalid());
                }
                Ok((from, to))
            }
            None => code(part).map(|c| (c, c)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if ranges.is_empty() || ranges.len() > 20 {
        return Err(invalid());
    }
    Ok(ranges)
}

fn valid_text(text: &str, max: usize) -> bool {
    !text.trim().is_empty() && text.chars().count() <= max && !text.chars().any(char::is_control)
}

impl HttpCheck {
    pub fn validate(&self) -> Result<(), Msg> {
        http_client::parse_url(&self.url).map_err(|_| Msg::new("service-url-invalid"))?;
        parse_expect(&self.expect)?;
        if let Some(keyword) = &self.keyword {
            if !valid_text(keyword, MAX_KEYWORD_CHARS) {
                return Err(Msg::new("service-keyword-invalid"));
            }
            if self.head {
                return Err(Msg::new("service-keyword-needs-get"));
            }
        }
        Ok(())
    }
}

pub(super) fn check_port(port: u16) -> Result<(), Msg> {
    if port == 0 {
        Err(Msg::new("service-port-invalid"))
    } else {
        Ok(())
    }
}

impl TcpCheck {
    pub fn validate(&self) -> Result<(), Msg> {
        check_port(self.port)
    }
}

impl DnsCheck {
    pub fn validate(&self) -> Result<(), Msg> {
        if !dns::valid_name(&self.name) {
            return Err(Msg::new("service-dns-name-invalid"));
        }
        let family_fits = match (self.record, self.expect) {
            (_, None) => true,
            (RecordType::A, Some(ip)) => ip.is_ipv4(),
            (RecordType::Aaaa, Some(ip)) => ip.is_ipv6(),
        };
        if !family_fits {
            return Err(Msg::new("service-dns-expect-family"));
        }
        check_port(self.port)
    }
}

impl TlsCheck {
    pub fn validate(&self) -> Result<(), Msg> {
        if let Some(name) = &self.server_name
            && !(dns::valid_name(name) || name.parse::<IpAddr>().is_ok())
        {
            return Err(Msg::new("service-server-name-invalid"));
        }
        check_port(self.port)
    }
}

fn ms(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0
}

fn response_time(elapsed: Duration) -> ChannelReading {
    ChannelReading {
        key: "rtt".to_owned(),
        unit: Unit::Milliseconds,
        value: Some(ms(elapsed)),
    }
}

fn failed(id: &'static str, detail: impl ToString) -> SensorError {
    SensorError::Check(Msg::new(id).with("detail", detail.to_string()))
}

pub(super) fn http_failure(e: HttpError) -> SensorError {
    match e {
        HttpError::Timeout => SensorError::Check(Msg::new("service-timeout")),
        HttpError::Connect(d) => failed("service-connect-failed", d),
        HttpError::Tls(d) => failed("service-tls-failed", d),
        other => failed("service-http-failed", other),
    }
}

pub struct HttpSensor {
    check: HttpCheck,
}

impl HttpSensor {
    pub fn new(check: HttpCheck) -> Self {
        Self { check }
    }
}

/// Whether `needle` occurs in `haystack` (bytes, case-sensitive).
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || haystack.windows(needle.len()).any(|w| w == needle)
}

impl Sensor for HttpSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            let check = &self.check;
            let expect = parse_expect(&check.expect).map_err(SensorError::Check)?;
            let options = http_client::Options {
                get: !check.head,
                follow_redirects: check.follow_redirects,
                verify_tls: check.verify_tls,
                max_body: if check.keyword.is_some() { MAX_BODY } else { 0 },
                timeout: ctx.timeout,
            };
            let response = http_client::fetch(&check.url, &options)
                .await
                .map_err(http_failure)?;
            let code = response.status.as_u16();
            if !expect
                .iter()
                .any(|(from, to)| (*from..=*to).contains(&code))
            {
                return Err(SensorError::Check(
                    Msg::new("service-http-status").with("code", code.to_string()),
                ));
            }
            if let Some(keyword) = &check.keyword
                && !contains(&response.body, keyword.as_bytes())
            {
                return Err(SensorError::Check(
                    Msg::new("service-keyword-missing").with("keyword", keyword.clone()),
                ));
            }
            Ok(Measurement {
                channels: vec![response_time(response.elapsed)],
                note: Some(Msg::Text(format!("HTTP {code}"))),
            })
        })
    }
}

pub struct TcpSensor {
    port: u16,
}

impl TcpSensor {
    pub fn new(check: &TcpCheck) -> Self {
        Self { port: check.port }
    }
}

impl Sensor for TcpSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            let started = Instant::now();
            tokio::time::timeout(ctx.timeout, http_client::connect_tcp(ctx.host, self.port))
                .await
                .unwrap_or(Err(HttpError::Timeout))
                .map_err(http_failure)?;
            Ok(Measurement {
                channels: vec![response_time(started.elapsed())],
                note: Some(Msg::new("service-port-open").with("port", self.port)),
            })
        })
    }
}

pub struct DnsSensor {
    check: DnsCheck,
}

impl DnsSensor {
    pub fn new(check: DnsCheck) -> Self {
        Self { check }
    }
}

/// The address of the DNS server: the device's host, an IP address or a name of the system resolver.
async fn server_addr(host: &str, port: u16) -> Result<SocketAddr, SensorError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| failed("service-connect-failed", e))?
        .next()
        .ok_or_else(|| failed("service-connect-failed", host))
}

impl Sensor for DnsSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            let check = &self.check;
            let server = server_addr(ctx.host, check.port).await?;
            let started = Instant::now();
            let found = dns::query(server, &check.name, check.record, ctx.timeout)
                .await
                .map_err(|e| match e {
                    dns::DnsError::Timeout => SensorError::Check(Msg::new("service-timeout")),
                    dns::DnsError::Code(3) => SensorError::Check(
                        Msg::new("service-dns-no-such-name").with("name", check.name.clone()),
                    ),
                    other => failed("service-dns-failed", other),
                })?;
            let elapsed = started.elapsed();
            let list = found
                .iter()
                .map(IpAddr::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            if found.is_empty() {
                return Err(SensorError::Check(
                    Msg::new("service-dns-no-address")
                        .with("name", check.name.clone())
                        .with("type", check.record.label()),
                ));
            }
            if let Some(expect) = check.expect
                && !found.contains(&expect)
            {
                return Err(SensorError::Check(
                    Msg::new("service-dns-mismatch")
                        .with("expected", expect.to_string())
                        .with("found", list),
                ));
            }
            Ok(Measurement {
                channels: vec![response_time(elapsed)],
                note: Some(Msg::Text(list)),
            })
        })
    }
}

pub struct TlsSensor {
    check: TlsCheck,
}

impl TlsSensor {
    pub fn new(check: TlsCheck) -> Self {
        Self { check }
    }
}

/// Expiry (Unix seconds) and issuer (common name, else organisation) of a DER certificate.
pub fn certificate_facts(der: &[u8]) -> Option<(i64, String)> {
    let (_, cert) = x509_parser::parse_x509_certificate(der).ok()?;
    let issuer = cert.issuer();
    let issuer = issuer
        .iter_common_name()
        .chain(issuer.iter_organization())
        .find_map(|a| a.as_str().ok())
        .unwrap_or_default()
        .to_owned();
    Some((cert.validity().not_after.timestamp(), issuer))
}

impl Sensor for TlsSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            let check = &self.check;
            let name = check.server_name.as_deref().unwrap_or(ctx.host);
            let handshake = async {
                let tcp = http_client::connect_tcp(ctx.host, check.port).await?;
                // Never refused in the handshake: an untrusted chain is reported below with its reason.
                http_client::connect_tls(tcp, name, false, true).await
            };
            let (_, peer) = tokio::time::timeout(ctx.timeout, handshake)
                .await
                .unwrap_or(Err(HttpError::Timeout))
                .map_err(http_failure)?;
            if check.verify
                && let Some(problem) = &peer.problem
            {
                return Err(failed("service-tls-untrusted", problem));
            }
            let (not_after, issuer) = certificate_facts(&peer.der)
                .ok_or_else(|| SensorError::Check(Msg::new("service-tls-unreadable")))?;
            let days = (not_after as f64 - ctx.now_ms as f64 / 1000.0) / 86_400.0;
            let until = time::OffsetDateTime::from_unix_timestamp(not_after)
                .map(|t| t.date().to_string())
                .unwrap_or_default();
            Ok(Measurement {
                channels: vec![ChannelReading {
                    key: "days".to_owned(),
                    unit: Unit::Days,
                    value: Some(days),
                }],
                note: Some(
                    Msg::new("service-tls-valid")
                        .with("date", until)
                        .with("issuer", issuer),
                ),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_client::testserver::{self, ok, status};
    use crate::sensors::ping::PingClients;

    async fn poll(sensor: &mut dyn Sensor, host: &str) -> Result<Measurement, SensorError> {
        let ping = PingClients::new();
        let mut ctx = PollContext {
            host,
            now_ms: crate::web::sensors::now_ms(),
            timeout: Duration::from_secs(3),
            ping: &ping,
            snmp: None,
        };
        sensor.poll(&mut ctx).await
    }

    fn http(url: String) -> HttpCheck {
        HttpCheck {
            url,
            head: false,
            expect: DEFAULT_EXPECT.to_owned(),
            keyword: None,
            follow_redirects: false,
            verify_tls: true,
        }
    }

    fn message(result: Result<Measurement, SensorError>) -> String {
        match result {
            Ok(m) => crate::i18n::Lang::En.msg(&m.note.unwrap_or(Msg::Text(String::new()))),
            Err(e) => crate::i18n::Lang::En.msg(&e.user_message()),
        }
    }

    #[test]
    fn expected_codes_are_parsed() {
        assert_eq!(parse_expect("200-399"), Ok(vec![(200, 399)]));
        assert_eq!(
            parse_expect(" 200 , 301-302 "),
            Ok(vec![(200, 200), (301, 302)])
        );
        for bad in ["", "abc", "99", "600", "300-200", "200,", "1-2-3"] {
            assert!(parse_expect(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn checks_are_validated() {
        assert!(http("https://example.org/".into()).validate().is_ok());
        assert!(http("example.org".into()).validate().is_err());
        let keyword_with_head = HttpCheck {
            head: true,
            keyword: Some("ok".into()),
            ..http("https://example.org/".into())
        };
        assert!(keyword_with_head.validate().is_err());
        let dns = |name: &str, record, expect: Option<&str>| DnsCheck {
            name: name.to_owned(),
            record,
            port: 53,
            expect: expect.map(|e| e.parse().unwrap()),
        };
        assert!(
            dns("example.org", RecordType::A, Some("192.0.2.1"))
                .validate()
                .is_ok()
        );
        assert!(
            dns("example.org", RecordType::Aaaa, Some("192.0.2.1"))
                .validate()
                .is_err()
        );
        assert!(dns("exa mple", RecordType::A, None).validate().is_err());
        assert!(TcpCheck { port: 0 }.validate().is_err());
        let tls = |name: &str| TlsCheck {
            port: 443,
            server_name: Some(name.to_owned()),
            verify: true,
        };
        assert!(tls("example.org").validate().is_ok());
        assert!(tls("192.0.2.1").validate().is_ok());
        assert!(tls("bad name").validate().is_err());
    }

    #[tokio::test]
    async fn http_checks_status_and_keyword() {
        let addr = testserver::serve(vec![ok("all systems go"); 3], None).await;
        let url = format!("http://{addr}/");
        let mut fine = HttpSensor::new(http(url.clone()));
        let m = poll(&mut fine, "ignored").await;
        assert_eq!(message(m), "HTTP 200");

        let mut keyword = HttpSensor::new(HttpCheck {
            keyword: Some("systems go".into()),
            ..http(url.clone())
        });
        assert!(poll(&mut keyword, "ignored").await.is_ok());
        let mut missing = HttpSensor::new(HttpCheck {
            keyword: Some("lost".into()),
            ..http(url)
        });
        let text = message(poll(&mut missing, "ignored").await);
        assert!(text.contains("lost"), "{text}");

        let addr = testserver::serve(vec![status(503, "")], None).await;
        let mut down = HttpSensor::new(http(format!("http://{addr}/")));
        let text = message(poll(&mut down, "ignored").await);
        assert!(text.contains("503"), "{text}");
    }

    #[tokio::test]
    async fn tcp_reports_open_and_closed_ports() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut open = TcpSensor::new(&TcpCheck { port });
        let m = poll(&mut open, "127.0.0.1").await.unwrap();
        assert_eq!(m.channels[0].key, "rtt");
        drop(listener);
        let mut closed = TcpSensor::new(&TcpCheck { port });
        assert!(poll(&mut closed, "127.0.0.1").await.is_err());
    }

    #[tokio::test]
    async fn dns_asks_the_device_and_compares_the_address() {
        let server = dns::testserver::serve(vec![[192, 0, 2, 53]], 0).await;
        let check = |expect: Option<&str>| DnsCheck {
            name: "example.org".into(),
            record: RecordType::A,
            port: server.port(),
            expect: expect.map(|e| e.parse().unwrap()),
        };
        let mut fine = DnsSensor::new(check(Some("192.0.2.53")));
        assert_eq!(message(poll(&mut fine, "127.0.0.1").await), "192.0.2.53");
        let mut wrong = DnsSensor::new(check(Some("192.0.2.1")));
        let text = message(poll(&mut wrong, "127.0.0.1").await);
        assert!(
            text.contains("192.0.2.1") && text.contains("192.0.2.53"),
            "{text}"
        );

        let nx = dns::testserver::serve(Vec::new(), 3).await;
        let mut missing = DnsSensor::new(DnsCheck {
            port: nx.port(),
            ..check(None)
        });
        let text = message(poll(&mut missing, "127.0.0.1").await);
        assert!(text.contains("example.org"), "{text}");
    }

    #[tokio::test]
    async fn tls_reports_days_until_expiry_and_untrusted_chains() {
        let addr = testserver::serve(vec![String::new(); 2], Some(20)).await;
        let check = |verify| TlsCheck {
            port: addr.port(),
            server_name: Some("localhost".into()),
            verify,
        };
        let mut lax = TlsSensor::new(check(false));
        let m = poll(&mut lax, "127.0.0.1").await.unwrap();
        let days = m.channels[0].value.unwrap();
        assert!((19.9..20.1).contains(&days), "{days}");
        let note = crate::i18n::Lang::En.msg(m.note.as_ref().unwrap());
        assert!(note.contains("opennetmon test CA"), "{note}");

        let mut strict = TlsSensor::new(check(true));
        let text = message(poll(&mut strict, "127.0.0.1").await);
        assert!(text.contains("not trusted"), "{text}");
    }

    #[tokio::test]
    async fn expired_certificates_still_report_their_days() {
        let addr = testserver::serve(vec![String::new()], Some(-2)).await;
        let mut sensor = TlsSensor::new(TlsCheck {
            port: addr.port(),
            server_name: Some("localhost".into()),
            verify: false,
        });
        let m = poll(&mut sensor, "127.0.0.1").await.unwrap();
        assert!(m.channels[0].value.unwrap() < -1.9);
    }
}
