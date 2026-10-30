//! FRITZ!Box: the state of the internet connection over UPnP-IGD (SOAP on port 49000). This needs
//! no login, and the box has no SNMP. Tested with a cable box; DSL boxes may use another service.

use std::time::Duration;

use hyper::Method;
use serde::{Deserialize, Serialize};

use super::counter::{CounterReading, CounterTracker, CounterWidth};
use super::service::{check_port, http_failure};
use super::snmp::{rate_value, reading};
use super::spec::ChannelDef;
use super::{Measurement, PollContext, PollFuture, Sensor, SensorError, Unit, xml};
use crate::http_client::{self, ApiCall};
use crate::i18n::Msg;

pub const DEFAULT_PORT: u16 = 49000;
/// Bytes of an answer that are read.
pub(super) const MAX_ANSWER: usize = 64 * 1024;

const COMMON_PATH: &str = "WANCommonIFC1";
const COMMON_SERVICE: &str = "WANCommonInterfaceConfig:1";
const IP_PATH: &str = "WANIPConn1";
const IP_SERVICE: &str = "WANIPConnection:1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FritzMetric {
    Traffic,
    Connection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FritzCheck {
    pub metric: FritzMetric,
    pub port: u16,
}

impl FritzCheck {
    pub fn new(metric: FritzMetric) -> Self {
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
            FritzMetric::Traffic => vec![
                channel("in", Unit::BitsPerSecond),
                channel("out", Unit::BitsPerSecond),
            ],
            FritzMetric::Connection => vec![
                channel("uptime", Unit::Seconds),
                channel("sync_down", Unit::BitsPerSecond),
                channel("sync_up", Unit::BitsPerSecond),
            ],
        }
    }
}

/// The name of an address inside a URL.
pub(super) fn url_host(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

pub(super) fn missing(tag: &str) -> SensorError {
    SensorError::Check(Msg::new("router-value-missing").with("tag", tag))
}

/// Calls `action` and returns the answer. Boxes with the setting "transmit status information over
/// UPnP" switched off answer with an error status.
async fn soap(
    host: &str,
    port: u16,
    (path, service): (&str, &str),
    action: &str,
    timeout: Duration,
) -> Result<String, SensorError> {
    let url = format!("http://{}:{port}/igdupnp/control/{path}", url_host(host));
    let urn = format!("urn:schemas-upnp-org:service:{service}");
    let body = format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>\
         <u:{action} xmlns:u=\"{urn}\"/></s:Body></s:Envelope>"
    );
    let soap_action = format!("{urn}#{action}");
    let response = http_client::call(&ApiCall {
        method: Method::POST,
        url: &url,
        headers: &[
            ("content-type", "text/xml; charset=\"utf-8\""),
            ("soapaction", &soap_action),
        ],
        body: &body,
        max_body: MAX_ANSWER,
        timeout,
    })
    .await
    .map_err(http_failure)?;
    if !response.status.is_success() {
        return Err(SensorError::Check(
            Msg::new("fritzbox-http-status").with("code", response.status.as_str()),
        ));
    }
    Ok(String::from_utf8_lossy(&response.body).into_owned())
}

pub struct FritzSensor {
    check: FritzCheck,
    inbound: CounterTracker,
    outbound: CounterTracker,
}

impl FritzSensor {
    pub fn new(check: FritzCheck) -> Self {
        Self {
            check,
            inbound: CounterTracker::new(CounterWidth::Bits64, None),
            outbound: CounterTracker::new(CounterWidth::Bits64, None),
        }
    }

    async fn traffic(&mut self, ctx: &PollContext<'_>) -> Result<Measurement, SensorError> {
        let answer = soap(
            ctx.host,
            self.check.port,
            (COMMON_PATH, COMMON_SERVICE),
            "GetAddonInfos",
            ctx.timeout,
        )
        .await?;
        let counter = |tag: &str| {
            xml::number::<u64>(&answer, tag)
                .map(|value| CounterReading {
                    value,
                    at_ms: ctx.now_ms,
                    uptime_ticks: None,
                })
                .ok_or_else(|| missing(tag))
        };
        let received = counter("NewX_AVM_DE_TotalBytesReceived64")?;
        let sent = counter("NewX_AVM_DE_TotalBytesSent64")?;
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

    async fn connection(&self, ctx: &PollContext<'_>) -> Result<Measurement, SensorError> {
        let (host, port) = (ctx.host, self.check.port);
        let status = soap(
            host,
            port,
            (IP_PATH, IP_SERVICE),
            "GetStatusInfo",
            ctx.timeout,
        )
        .await?;
        let state = xml::text(&status, "NewConnectionStatus")
            .ok_or_else(|| missing("NewConnectionStatus"))?;
        if state != "Connected" {
            return Err(SensorError::Check(
                Msg::new("fritzbox-disconnected").with("state", state).with(
                    "error",
                    xml::text(&status, "NewLastConnectionError").unwrap_or_default(),
                ),
            ));
        }
        let uptime: f64 = xml::number(&status, "NewUptime").ok_or_else(|| missing("NewUptime"))?;
        let infos = soap(
            host,
            port,
            (COMMON_PATH, COMMON_SERVICE),
            "GetAddonInfos",
            ctx.timeout,
        )
        .await?;
        let sync = |tag: &str| xml::number::<f64>(&infos, tag).ok_or_else(|| missing(tag));
        Ok(Measurement {
            note: Some(Msg::new("fritzbox-connected").with(
                "access",
                xml::text(&infos, "NewX_AVM_DE_WANAccessType").unwrap_or_default(),
            )),
            channels: vec![
                reading("uptime", Unit::Seconds, Some(uptime)),
                reading(
                    "sync_down",
                    Unit::BitsPerSecond,
                    Some(sync("NewX_AVM_DE_Layer1DownstreamMaxBitRate64")?),
                ),
                reading(
                    "sync_up",
                    Unit::BitsPerSecond,
                    Some(sync("NewX_AVM_DE_Layer1UpstreamMaxBitRate64")?),
                ),
            ],
        })
    }
}

impl Sensor for FritzSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            // One deadline for all requests of this poll.
            let timeout = ctx.timeout;
            let ctx = &*ctx;
            let measured = async {
                match self.check.metric {
                    FritzMetric::Traffic => self.traffic(ctx).await,
                    FritzMetric::Connection => self.connection(ctx).await,
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

    const ADDON: &str = "<u:GetAddonInfosResponse>\
        <NewByteReceiveRate>127041</NewByteReceiveRate>\
        <NewTotalBytesReceived>3275612381</NewTotalBytesReceived>\
        <NewX_AVM_DE_TotalBytesSent64>2000</NewX_AVM_DE_TotalBytesSent64>\
        <NewX_AVM_DE_TotalBytesReceived64>1000</NewX_AVM_DE_TotalBytesReceived64>\
        <NewX_AVM_DE_WANAccessType>Cable</NewX_AVM_DE_WANAccessType>\
        <NewX_AVM_DE_Layer1UpstreamMaxBitRate64>50000000</NewX_AVM_DE_Layer1UpstreamMaxBitRate64>\
        <NewX_AVM_DE_Layer1DownstreamMaxBitRate64>1000000000</NewX_AVM_DE_Layer1DownstreamMaxBitRate64>\
        </u:GetAddonInfosResponse>";
    const CONNECTED: &str = "<u:GetStatusInfoResponse>\
        <NewConnectionStatus>Connected</NewConnectionStatus>\
        <NewLastConnectionError>ERROR_NONE</NewLastConnectionError>\
        <NewUptime>600</NewUptime></u:GetStatusInfoResponse>";

    async fn poll(sensor: &mut FritzSensor, now_ms: i64) -> Result<Measurement, SensorError> {
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

    fn sensor(metric: FritzMetric, port: u16) -> FritzSensor {
        FritzSensor::new(FritzCheck { metric, port })
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
    async fn traffic_comes_from_the_64_bit_counters_and_starts_with_the_second_poll() {
        let later = ADDON
            .replace(">1000<", ">126000<")
            .replace(">2000<", ">27000<");
        let (addr, mut requests) = serve_and_record(vec![ok(ADDON), ok(&later)], None).await;
        let mut s = sensor(FritzMetric::Traffic, addr.port());
        let first = poll(&mut s, 10_000).await.unwrap();
        assert_eq!(value(&first, "in"), None);
        let second = poll(&mut s, 20_000).await.unwrap();
        assert_eq!(value(&second, "in"), Some(100_000.0));
        assert_eq!(value(&second, "out"), Some(20_000.0));

        let request = requests.recv().await.unwrap();
        assert!(
            request.starts_with("POST /igdupnp/control/WANCommonIFC1 HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.to_ascii_lowercase().contains(
                "soapaction: urn:schemas-upnp-org:service:wancommoninterfaceconfig:1#getaddoninfos"
            ),
            "{request}"
        );
        assert!(request.contains("<u:GetAddonInfos xmlns:u="), "{request}");
    }

    #[tokio::test]
    async fn a_falling_counter_costs_one_value_and_no_outlier() {
        let lower = ADDON.replace(">1000<", ">10<").replace(">2000<", ">20<");
        let addr = serve(vec![ok(ADDON), ok(&lower)], None).await;
        let mut s = sensor(FritzMetric::Traffic, addr.port());
        poll(&mut s, 10_000).await.unwrap();
        let after = poll(&mut s, 20_000).await.unwrap();
        assert_eq!(value(&after, "in"), None);
        assert_eq!(value(&after, "out"), None);
    }

    #[tokio::test]
    async fn connection_reports_uptime_and_sync_rates() {
        let addr = serve(vec![ok(CONNECTED), ok(ADDON)], None).await;
        let mut s = sensor(FritzMetric::Connection, addr.port());
        let m = poll(&mut s, 1).await.unwrap();
        assert_eq!(value(&m, "uptime"), Some(600.0));
        assert_eq!(value(&m, "sync_down"), Some(1e9));
        assert_eq!(value(&m, "sync_up"), Some(5e7));
        assert_eq!(Lang::En.msg(&m.note.unwrap()), "Connected (Cable)");
    }

    #[tokio::test]
    async fn a_lost_connection_names_the_box_s_reason() {
        let down = CONNECTED
            .replace("Connected", "Disconnected")
            .replace("ERROR_NONE", "ERROR_NO_CARRIER");
        let addr = serve(vec![ok(&down)], None).await;
        let mut s = sensor(FritzMetric::Connection, addr.port());
        let text = message(poll(&mut s, 1).await);
        assert!(text.contains("Disconnected"), "{text}");
        assert!(text.contains("ERROR_NO_CARRIER"), "{text}");
    }

    #[tokio::test]
    async fn an_error_status_points_to_the_upnp_setting() {
        let addr = serve(vec![status(500, "")], None).await;
        let mut s = sensor(FritzMetric::Traffic, addr.port());
        let text = message(poll(&mut s, 1).await);
        assert!(text.contains("500"), "{text}");
        assert!(text.contains("UPnP"), "{text}");
    }

    #[tokio::test]
    async fn a_missing_value_is_reported_and_not_invented() {
        let addr = serve(vec![ok("<nothing/>"), ok("<nothing/>")], None).await;
        let mut s = sensor(FritzMetric::Traffic, addr.port());
        let text = message(poll(&mut s, 1).await);
        assert!(text.contains("NewX_AVM_DE_TotalBytesReceived64"), "{text}");
        let mut s = sensor(FritzMetric::Connection, addr.port());
        let text = message(poll(&mut s, 1).await);
        assert!(text.contains("NewConnectionStatus"), "{text}");
    }

    #[tokio::test]
    async fn a_silent_box_runs_into_the_deadline() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let ping = PingClients::new();
        let mut ctx = PollContext {
            host: "127.0.0.1",
            now_ms: 1,
            timeout: Duration::from_millis(50),
            ping: &ping,
            snmp: None,
        };
        let mut s = sensor(FritzMetric::Traffic, port);
        let text = message(s.poll(&mut ctx).await);
        assert!(text.contains("timeout"), "{text}");
        drop(listener);
    }

    #[test]
    fn settings_survive_the_database() {
        let check = FritzCheck::new(FritzMetric::Connection);
        assert_eq!(
            serde_json::to_string(&check).unwrap(),
            r#"{"metric":"connection","port":49000}"#
        );
        assert!(check.validate().is_ok());
        assert!(FritzCheck { port: 0, ..check }.validate().is_err());
    }
}
