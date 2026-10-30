//! Huawei 5G/LTE routers with the HiLink API: connection, traffic and data volume of the mobile
//! link, without a login. Every poll asks for a session first (`/api/webserver/SesTokInfo`). The
//! API reports errors with HTTP 200 as `<error><code>`, so each answer is checked for them.
//!
//! The meaning of most status numbers is not documented. Only the values seen on a working link
//! count as fine; anything else is shown as the router sent it.

use std::time::Duration;

use hyper::Method;
use serde::{Deserialize, Serialize};

use super::counter::{CounterReading, CounterTracker, CounterWidth};
use super::fritzbox::{MAX_ANSWER, missing, url_host};
use super::service::{check_port, http_failure};
use super::snmp::{rate_value, reading};
use super::spec::ChannelDef;
use super::{Measurement, PollContext, PollFuture, Sensor, SensorError, Unit, xml};
use crate::http_client::{self, ApiCall};
use crate::i18n::Msg;

pub const DEFAULT_PORT: u16 = 80;

/// Signal bars below which a new connection sensor warns (changeable per channel).
pub const WARN_BARS: f64 = 2.0;
/// Download rate above which a new traffic sensor warns that the link is in use (bit/s,
/// changeable per channel).
pub const WARN_RATE: f64 = 1_000_000.0;

const CONNECTED: u32 = 901;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HuaweiMetric {
    Connection,
    Traffic,
    Volume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HuaweiCheck {
    pub metric: HuaweiMetric,
    pub port: u16,
}

impl HuaweiCheck {
    pub fn new(metric: HuaweiMetric) -> Self {
        Self {
            metric,
            port: DEFAULT_PORT,
        }
    }

    pub fn validate(&self) -> Result<(), Msg> {
        check_port(self.port)
    }

    pub fn channels(&self) -> Vec<ChannelDef> {
        let channel = |key, unit| ChannelDef { key, unit };
        match self.metric {
            HuaweiMetric::Connection => vec![
                channel("signal", Unit::Count),
                channel("uptime", Unit::Seconds),
            ],
            HuaweiMetric::Traffic => vec![
                channel("in", Unit::BitsPerSecond),
                channel("out", Unit::BitsPerSecond),
            ],
            HuaweiMetric::Volume => vec![
                channel("month_down", Unit::Bytes),
                channel("month_up", Unit::Bytes),
                channel("day_used", Unit::Bytes),
            ],
        }
    }
}

/// What the API asks for with each request.
struct Session {
    base: String,
    cookie: String,
    token: String,
    timeout: Duration,
}

/// One GET; `session` is `None` for the request that opens the session.
async fn get(
    base: &str,
    path: &str,
    session: Option<&Session>,
    timeout: Duration,
) -> Result<String, SensorError> {
    let url = format!("{base}{path}");
    let headers: Vec<(&str, &str)> = session.map_or_else(Vec::new, |s| {
        vec![
            ("cookie", s.cookie.as_str()),
            ("__requestverificationtoken", s.token.as_str()),
        ]
    });
    let response = http_client::call(&ApiCall {
        method: Method::GET,
        url: &url,
        headers: &headers,
        body: "",
        max_body: MAX_ANSWER,
        timeout,
    })
    .await
    .map_err(http_failure)?;
    if !response.status.is_success() {
        return Err(SensorError::Check(
            Msg::new("service-http-status").with("code", response.status.as_str()),
        ));
    }
    let answer = String::from_utf8_lossy(&response.body).into_owned();
    if answer.contains("<error>") {
        return Err(SensorError::Check(
            Msg::new("huawei-api-error")
                .with("code", xml::text(&answer, "code").unwrap_or_default())
                .with("path", path),
        ));
    }
    Ok(answer)
}

impl Session {
    async fn open(host: &str, port: u16, timeout: Duration) -> Result<Self, SensorError> {
        let base = format!("http://{}:{port}", url_host(host));
        let answer = get(&base, "/api/webserver/SesTokInfo", None, timeout).await?;
        let session = xml::text(&answer, "SesInfo").ok_or_else(|| missing("SesInfo"))?;
        let token = xml::text(&answer, "TokInfo").ok_or_else(|| missing("TokInfo"))?;
        // Some firmware sends the value alone, some with the name of the cookie.
        let id = session.strip_prefix("SessionID=").unwrap_or(&session);
        Ok(Self {
            cookie: format!("SessionID={id}"),
            base,
            token,
            timeout,
        })
    }

    async fn get(&self, path: &str) -> Result<String, SensorError> {
        get(&self.base, path, Some(self), self.timeout).await
    }
}

fn needed(answer: &str, tag: &str) -> Result<u32, SensorError> {
    xml::number(answer, tag).ok_or_else(|| missing(tag))
}

fn failed(id: &'static str, value: u32) -> SensorError {
    // As text: a status number is shown as the router sent it, without number formatting.
    SensorError::Check(Msg::new(id).with("value", value.to_string()))
}

/// Why the link would not carry traffic right now, if so.
fn check_ready(status: &str, data_switch: &str) -> Result<(), SensorError> {
    if xml::number::<u32>(status, "flymode") == Some(1) {
        return Err(SensorError::Check(Msg::new("huawei-flight-mode")));
    }
    let sim = needed(status, "SimStatus")?;
    if sim != 1 {
        return Err(failed("huawei-sim-not-ready", sim));
    }
    if needed(data_switch, "dataswitch")? != 1 {
        return Err(SensorError::Check(Msg::new("huawei-data-off")));
    }
    let connection = needed(status, "ConnectionStatus")?;
    if connection != CONNECTED {
        return Err(failed("huawei-not-connected", connection));
    }
    Ok(())
}

pub struct HuaweiSensor {
    check: HuaweiCheck,
    inbound: CounterTracker,
    outbound: CounterTracker,
}

impl HuaweiSensor {
    pub fn new(check: HuaweiCheck) -> Self {
        Self {
            check,
            inbound: CounterTracker::new(CounterWidth::Bits64, None),
            outbound: CounterTracker::new(CounterWidth::Bits64, None),
        }
    }

    async fn connection(session: &Session) -> Result<Measurement, SensorError> {
        let status = session.get("/api/monitoring/status").await?;
        let data_switch = session.get("/api/dialup/mobile-dataswitch").await?;
        check_ready(&status, &data_switch)?;
        let traffic = session.get("/api/monitoring/traffic-statistics").await?;
        let network = session.get("/api/net/current-plmn").await?;
        Ok(Measurement {
            note: Some(
                Msg::new("huawei-connected")
                    .with(
                        "operator",
                        xml::text(&network, "FullName").unwrap_or_default(),
                    )
                    .with(
                        "network",
                        xml::text(&status, "CurrentNetworkTypeEx").unwrap_or_default(),
                    ),
            ),
            channels: vec![
                reading("signal", Unit::Count, xml::number(&status, "SignalIcon")),
                reading(
                    "uptime",
                    Unit::Seconds,
                    xml::number(&traffic, "CurrentConnectTime"),
                ),
            ],
        })
    }

    async fn traffic(
        &mut self,
        session: &Session,
        now_ms: i64,
    ) -> Result<Measurement, SensorError> {
        let answer = session.get("/api/monitoring/traffic-statistics").await?;
        let counter = |tag: &str| {
            xml::number::<u64>(&answer, tag)
                .map(|value| CounterReading {
                    value,
                    at_ms: now_ms,
                    uptime_ticks: None,
                })
                .ok_or_else(|| missing(tag))
        };
        let received = counter("TotalDownload")?;
        let sent = counter("TotalUpload")?;
        // Octets per second -> bits per second.
        Ok(Measurement {
            note: None,
            channels: vec![
                reading(
                    "in",
                    Unit::BitsPerSecond,
                    rate_value(self.inbound.update(received), 8.0),
                ),
                reading(
                    "out",
                    Unit::BitsPerSecond,
                    rate_value(self.outbound.update(sent), 8.0),
                ),
            ],
        })
    }

    async fn volume(session: &Session) -> Result<Measurement, SensorError> {
        let answer = session.get("/api/monitoring/month_statistics").await?;
        let bytes = |tag: &str| xml::number::<f64>(&answer, tag).ok_or_else(|| missing(tag));
        Ok(Measurement {
            note: None,
            channels: vec![
                reading(
                    "month_down",
                    Unit::Bytes,
                    Some(bytes("CurrentMonthDownload")?),
                ),
                reading("month_up", Unit::Bytes, Some(bytes("CurrentMonthUpload")?)),
                reading("day_used", Unit::Bytes, Some(bytes("CurrentDayUsed")?)),
            ],
        })
    }
}

impl Sensor for HuaweiSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            // One deadline for all requests of this poll.
            let (host, now_ms, timeout) = (ctx.host, ctx.now_ms, ctx.timeout);
            let measured = async {
                let session = Session::open(host, self.check.port, timeout).await?;
                match self.check.metric {
                    HuaweiMetric::Connection => Self::connection(&session).await,
                    HuaweiMetric::Traffic => self.traffic(&session, now_ms).await,
                    HuaweiMetric::Volume => Self::volume(&session).await,
                }
            };
            tokio::time::timeout(timeout, measured)
                .await
                .unwrap_or_else(|_| Err(SensorError::Check(Msg::new("service-timeout"))))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_client::testserver::{ok, serve, serve_and_record, status};
    use crate::i18n::Lang;
    use crate::sensors::ping::PingClients;

    const SESSION: &str =
        "<response><SesInfo>0123abcd</SesInfo><TokInfo>to&#x2F;ken</TokInfo></response>";
    const STATUS: &str = "<response><ConnectionStatus>901</ConnectionStatus><SignalIcon>3</SignalIcon>\
        <flymode>0</flymode><SimStatus>1</SimStatus><CurrentNetworkTypeEx>1011</CurrentNetworkTypeEx></response>";
    const SWITCH_ON: &str = "<response><dataswitch>1</dataswitch></response>";
    const TRAFFIC: &str = "<response><CurrentConnectTime>7200</CurrentConnectTime>\
        <TotalUpload>2000</TotalUpload><TotalDownload>1000</TotalDownload></response>";
    const PLMN: &str = "<response><FullName>Example &amp; Co</FullName></response>";
    const VOLUME: &str = "<response><CurrentMonthDownload>300</CurrentMonthDownload>\
        <CurrentMonthUpload>200</CurrentMonthUpload><CurrentDayUsed>100</CurrentDayUsed></response>";

    async fn poll(sensor: &mut HuaweiSensor, now_ms: i64) -> Result<Measurement, SensorError> {
        let ping = PingClients::new();
        let mut ctx = PollContext {
            host: "127.0.0.1",
            now_ms,
            timeout: Duration::from_secs(5),
            ping: &ping,
            snmp: None,
        };
        sensor.poll(&mut ctx).await
    }

    fn sensor(metric: HuaweiMetric, port: u16) -> HuaweiSensor {
        HuaweiSensor::new(HuaweiCheck { metric, port })
    }

    fn value(m: &Measurement, key: &str) -> Option<f64> {
        m.channels.iter().find(|c| c.key == key)?.value
    }

    fn message(result: Result<Measurement, SensorError>) -> String {
        match result {
            Err(SensorError::Check(m)) => Lang::En.msg(&m),
            other => panic!("expected a failed check, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn connection_sends_the_session_and_reports_signal_uptime_and_network() {
        let replies = [SESSION, STATUS, SWITCH_ON, TRAFFIC, PLMN].map(ok).to_vec();
        let (addr, mut requests) = serve_and_record(replies, None).await;
        let mut s = sensor(HuaweiMetric::Connection, addr.port());
        let m = poll(&mut s, 1).await.unwrap();
        assert_eq!(value(&m, "signal"), Some(3.0));
        assert_eq!(value(&m, "uptime"), Some(7200.0));
        assert_eq!(
            Lang::En.msg(&m.note.unwrap()),
            "Connected via Example & Co, network type 1011"
        );

        let first = requests.recv().await.unwrap();
        assert!(
            first.starts_with("GET /api/webserver/SesTokInfo "),
            "{first}"
        );
        assert!(!first.to_ascii_lowercase().contains("cookie"), "{first}");
        let second = requests.recv().await.unwrap().to_ascii_lowercase();
        assert!(
            second.starts_with("get /api/monitoring/status "),
            "{second}"
        );
        assert!(
            second.contains("cookie: sessionid=0123abcd\r\n"),
            "{second}"
        );
        assert!(
            second.contains("__requestverificationtoken: to/ken\r\n"),
            "{second}"
        );
    }

    #[tokio::test]
    async fn a_cookie_name_in_the_session_is_not_doubled() {
        let named = SESSION.replace("<SesInfo>", "<SesInfo>SessionID=");
        let replies = vec![ok(&named), ok(VOLUME)];
        let (addr, mut requests) = serve_and_record(replies, None).await;
        let mut s = sensor(HuaweiMetric::Volume, addr.port());
        poll(&mut s, 1).await.unwrap();
        requests.recv().await.unwrap();
        let second = requests.recv().await.unwrap().to_ascii_lowercase();
        assert!(
            second.contains("cookie: sessionid=0123abcd\r\n"),
            "{second}"
        );
    }

    #[tokio::test]
    async fn the_link_counts_only_when_everything_is_ready() {
        for (status_answer, switch, expected) in [
            (
                STATUS.replace("<flymode>0", "<flymode>1"),
                SWITCH_ON,
                "Flight mode",
            ),
            (
                STATUS.replace("<SimStatus>1", "<SimStatus>255"),
                SWITCH_ON,
                "255",
            ),
            (
                STATUS.to_owned(),
                "<response><dataswitch>0</dataswitch></response>",
                "Mobile data",
            ),
            (STATUS.replace(">901<", ">1902<"), SWITCH_ON, "1902"),
        ] {
            let replies = vec![ok(SESSION), ok(&status_answer), ok(switch)];
            let addr = serve(replies, None).await;
            let mut s = sensor(HuaweiMetric::Connection, addr.port());
            let text = message(poll(&mut s, 1).await);
            assert!(text.contains(expected), "{text}");
        }
    }

    #[tokio::test]
    async fn errors_arrive_with_http_200() {
        let locked = "<error><code>125002</code><message/></error>";
        let addr = serve(vec![ok(SESSION), ok(locked)], None).await;
        let mut s = sensor(HuaweiMetric::Volume, addr.port());
        let text = message(poll(&mut s, 1).await);
        assert!(text.contains("125002"), "{text}");
        assert!(text.contains("month_statistics"), "{text}");

        let denied = "<error><code>100003</code><message/></error>";
        let addr = serve(vec![ok(denied)], None).await;
        let mut s = sensor(HuaweiMetric::Volume, addr.port());
        assert!(message(poll(&mut s, 1).await).contains("100003"));

        let addr = serve(vec![status(503, "")], None).await;
        let mut s = sensor(HuaweiMetric::Volume, addr.port());
        assert!(message(poll(&mut s, 1).await).contains("503"));
    }

    #[tokio::test]
    async fn traffic_comes_from_the_counters_and_a_reset_costs_one_value() {
        let later = TRAFFIC
            .replace(">1000<", ">126000<")
            .replace(">2000<", ">27000<");
        let replies = [SESSION, TRAFFIC, SESSION, &later, SESSION, TRAFFIC]
            .map(ok)
            .to_vec();
        let addr = serve(replies, None).await;
        let mut s = sensor(HuaweiMetric::Traffic, addr.port());
        let first = poll(&mut s, 10_000).await.unwrap();
        assert_eq!(value(&first, "in"), None);
        let second = poll(&mut s, 20_000).await.unwrap();
        assert_eq!(value(&second, "in"), Some(100_000.0));
        assert_eq!(value(&second, "out"), Some(20_000.0));
        let reset = poll(&mut s, 30_000).await.unwrap();
        assert_eq!(value(&reset, "in"), None);
    }

    #[tokio::test]
    async fn volume_reports_month_and_day() {
        let addr = serve(vec![ok(SESSION), ok(VOLUME)], None).await;
        let mut s = sensor(HuaweiMetric::Volume, addr.port());
        let m = poll(&mut s, 1).await.unwrap();
        assert_eq!(value(&m, "month_down"), Some(300.0));
        assert_eq!(value(&m, "month_up"), Some(200.0));
        assert_eq!(value(&m, "day_used"), Some(100.0));
    }

    #[tokio::test]
    async fn a_missing_value_is_reported_and_not_invented() {
        let addr = serve(vec![ok("<response/>")], None).await;
        let mut s = sensor(HuaweiMetric::Volume, addr.port());
        assert!(message(poll(&mut s, 1).await).contains("SesInfo"));
        let addr = serve(vec![ok(SESSION), ok("<response/>")], None).await;
        let mut s = sensor(HuaweiMetric::Volume, addr.port());
        assert!(message(poll(&mut s, 1).await).contains("CurrentMonthDownload"));
    }

    #[test]
    fn settings_survive_the_database() {
        let check = HuaweiCheck::new(HuaweiMetric::Volume);
        assert_eq!(
            serde_json::to_string(&check).unwrap(),
            r#"{"metric":"volume","port":80}"#
        );
        assert!(check.validate().is_ok());
    }
}
