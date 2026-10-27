//! SNMP-based sensors: uptime, interface traffic, interface status and a custom OID.
//!
//! All sensors read with a single GET per poll. Counters additionally fetch `sysUpTime`, which
//! lets [`CounterTracker`] tell a counter wrap from a device restart.

use crate::snmp::{SYS_UPTIME, SnmpValue};

use super::counter::{CounterReading, CounterTracker, CounterWidth, Rate};
use super::{ChannelReading, Measurement, PollContext, PollFuture, Sensor, SensorError, Unit};

const IF_OPER_STATUS: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 8];
const IF_IN_OCTETS: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 10];
const IF_OUT_OCTETS: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 16];
const IF_HC_IN_OCTETS: &[u64] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 6];
const IF_HC_OUT_OCTETS: &[u64] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 10];

fn with_index(base: &[u64], index: u32) -> Vec<u64> {
    let mut oid = base.to_vec();
    oid.push(u64::from(index));
    oid
}

fn reading(key: &str, unit: Unit, value: Option<f64>) -> ChannelReading {
    ChannelReading {
        key: key.to_owned(),
        unit,
        value,
    }
}

/// A numeric value of any numeric SNMP type; `None` for text and "no such object".
pub(crate) fn numeric(value: &SnmpValue) -> Option<f64> {
    match value {
        SnmpValue::Integer(n) => Some(*n as f64),
        SnmpValue::Counter32(n) | SnmpValue::Gauge32(n) | SnmpValue::Timeticks(n) => {
            Some(f64::from(*n))
        }
        SnmpValue::Counter64(n) => Some(*n as f64),
        _ => None,
    }
}

fn counter(value: &SnmpValue) -> Option<(u64, CounterWidth)> {
    match value {
        SnmpValue::Counter32(n) => Some((u64::from(*n), CounterWidth::Bits32)),
        SnmpValue::Counter64(n) => Some((*n, CounterWidth::Bits64)),
        _ => None,
    }
}

fn uptime_ticks(value: Option<&SnmpValue>) -> Option<u32> {
    match value {
        Some(SnmpValue::Timeticks(t)) => Some(*t),
        _ => None,
    }
}

fn rate_value(rate: Rate, factor: f64) -> Option<f64> {
    match rate {
        Rate::PerSecond(r) => Some(r * factor),
        Rate::First | Rate::Skipped(_) => None,
    }
}

/// Runs one GET and checks that the agent answered every requested OID.
async fn get(ctx: &mut PollContext<'_>, oids: &[&[u64]]) -> Result<Vec<SnmpValue>, SensorError> {
    let values = ctx.snmp()?.get(oids).await?;
    if values.len() != oids.len() {
        return Err(SensorError::BadValue(format!(
            "expected {} values, got {}",
            oids.len(),
            values.len()
        )));
    }
    Ok(values.into_iter().map(|(_, v)| v).collect())
}

fn missing(value: &SnmpValue) -> bool {
    matches!(
        value,
        SnmpValue::NoSuchObject | SnmpValue::NoSuchInstance | SnmpValue::EndOfMibView
    )
}

/// `sysUpTime` in seconds.
pub struct UptimeSensor;

impl Sensor for UptimeSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            let values = get(ctx, &[SYS_UPTIME]).await?;
            let ticks = uptime_ticks(values.first())
                .ok_or_else(|| SensorError::BadValue("sysUpTime is not TimeTicks".to_owned()))?;
            Ok(Measurement {
                note: None,
                channels: vec![reading(
                    "uptime",
                    Unit::Seconds,
                    Some(f64::from(ticks) / 100.0),
                )],
            })
        })
    }
}

/// `ifOperStatus` of one interface (1 = up, 2 = down, ...).
pub struct InterfaceStatusSensor {
    oid: Vec<u64>,
}

impl InterfaceStatusSensor {
    pub fn new(if_index: u32) -> Self {
        Self {
            oid: with_index(IF_OPER_STATUS, if_index),
        }
    }
}

impl Sensor for InterfaceStatusSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            let values = get(ctx, &[&self.oid]).await?;
            match values.first() {
                Some(SnmpValue::Integer(n)) => Ok(Measurement {
                    note: None,
                    channels: vec![reading("oper_status", Unit::State, Some(*n as f64))],
                }),
                _ => Err(SensorError::BadValue(
                    "interface does not exist or ifOperStatus has an unexpected type".to_owned(),
                )),
            }
        })
    }
}

/// In/out traffic of one interface in bit/s. Prefers the 64-bit `ifXTable` counters and falls
/// back to the 32-bit `ifTable` counters (access points).
pub struct InterfaceTrafficSensor {
    oids: [Vec<u64>; 4],
    width: Option<CounterWidth>,
    inbound: CounterTracker,
    outbound: CounterTracker,
}

impl InterfaceTrafficSensor {
    pub fn new(if_index: u32) -> Self {
        Self {
            oids: [
                with_index(IF_HC_IN_OCTETS, if_index),
                with_index(IF_HC_OUT_OCTETS, if_index),
                with_index(IF_IN_OCTETS, if_index),
                with_index(IF_OUT_OCTETS, if_index),
            ],
            width: None,
            inbound: CounterTracker::new(CounterWidth::Bits64, None),
            outbound: CounterTracker::new(CounterWidth::Bits64, None),
        }
    }

    fn pick(values: &[SnmpValue]) -> Result<(CounterWidth, &SnmpValue, &SnmpValue), SensorError> {
        // values: sysUpTime, hcIn, hcOut, in, out
        if let (Some(_), Some(_)) = (counter(&values[1]), counter(&values[2])) {
            return Ok((CounterWidth::Bits64, &values[1], &values[2]));
        }
        if let (Some(_), Some(_)) = (counter(&values[3]), counter(&values[4])) {
            return Ok((CounterWidth::Bits32, &values[3], &values[4]));
        }
        if values[1..].iter().all(missing) {
            return Err(SensorError::BadValue(
                "interface does not exist (no octet counters)".to_owned(),
            ));
        }
        Err(SensorError::BadValue(
            "octet counters have an unexpected type".to_owned(),
        ))
    }
}

impl Sensor for InterfaceTrafficSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            let [hc_in, hc_out, inb, outb] = &self.oids;
            let values = get(ctx, &[SYS_UPTIME, hc_in, hc_out, inb, outb]).await?;
            let uptime = uptime_ticks(values.first());
            let (width, in_value, out_value) = Self::pick(&values)?;
            if self.width != Some(width) {
                // First poll, or the device switched counter width: start over.
                self.width = Some(width);
                self.inbound = CounterTracker::new(width, None);
                self.outbound = CounterTracker::new(width, None);
            }
            let at = |value: &SnmpValue| {
                counter(value).map(|(value, _)| CounterReading {
                    value,
                    at_ms: ctx.now_ms,
                    uptime_ticks: uptime,
                })
            };
            let in_rate = at(in_value).map(|r| self.inbound.update(r));
            let out_rate = at(out_value).map(|r| self.outbound.update(r));
            // Octets per second -> bits per second.
            Ok(Measurement {
                note: None,
                channels: vec![
                    reading(
                        "in",
                        Unit::BitsPerSecond,
                        in_rate.and_then(|r| rate_value(r, 8.0)),
                    ),
                    reading(
                        "out",
                        Unit::BitsPerSecond,
                        out_rate.and_then(|r| rate_value(r, 8.0)),
                    ),
                ],
            })
        })
    }
}

/// How a custom OID is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OidMode {
    /// Use the value as is.
    Gauge,
    /// Counter32/Counter64: report the rate per second.
    Counter,
}

/// A user-defined scalar OID, optionally scaled by a factor.
pub struct OidSensor {
    oid: Vec<u64>,
    mode: OidMode,
    scale: f64,
    tracker: Option<CounterTracker>,
}

impl OidSensor {
    pub fn new(oid: Vec<u64>, mode: OidMode, scale: f64) -> Self {
        Self {
            oid,
            mode,
            scale,
            tracker: None,
        }
    }
}

impl Sensor for OidSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            // sysUpTime is only needed to confirm counter wraps.
            let values = match self.mode {
                OidMode::Gauge => get(ctx, &[&self.oid]).await?,
                OidMode::Counter => get(ctx, &[&self.oid, SYS_UPTIME]).await?,
            };
            let value = &values[0];
            if missing(value) {
                return Err(SensorError::BadValue("OID does not exist".to_owned()));
            }
            match self.mode {
                OidMode::Gauge => {
                    let number = numeric(value)
                        .or_else(|| match value {
                            SnmpValue::Text(t) => t.trim().parse::<f64>().ok(),
                            _ => None,
                        })
                        .filter(|n| n.is_finite())
                        .ok_or_else(|| SensorError::BadValue("value is not numeric".to_owned()))?;
                    Ok(Measurement {
                        note: None,
                        channels: vec![reading("value", Unit::Count, Some(number * self.scale))],
                    })
                }
                OidMode::Counter => {
                    let (raw, width) = counter(value).ok_or_else(|| {
                        SensorError::BadValue("value is not a Counter32/Counter64".to_owned())
                    })?;
                    let tracker = self
                        .tracker
                        .get_or_insert_with(|| CounterTracker::new(width, None));
                    let rate = tracker.update(CounterReading {
                        value: raw,
                        at_ms: ctx.now_ms,
                        uptime_ticks: uptime_ticks(values.get(1)),
                    });
                    Ok(Measurement {
                        note: None,
                        channels: vec![reading(
                            "value",
                            Unit::PerSecond,
                            rate_value(rate, self.scale),
                        )],
                    })
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::credentials::{Credentials, Secret};
    use crate::sensors::ping::PingClients;
    use crate::snmp::testagent::{FakeAgent, FakeValue};
    use crate::snmp::{SnmpClient, SnmpTarget};

    struct Rig {
        agent: FakeAgent,
        client: SnmpClient,
        ping: PingClients,
        now_ms: i64,
    }

    impl Rig {
        async fn new() -> Self {
            let agent = FakeAgent::spawn("public", "test", 1_000).await;
            let target = SnmpTarget {
                host: "127.0.0.1".to_owned(),
                port: agent.port,
                timeout: Duration::from_millis(500),
                retries: 0,
            };
            let creds = Credentials::V2c {
                community: Secret::new("public"),
            };
            let client = SnmpClient::connect(&target, &creds).await.unwrap();
            Self {
                agent,
                client,
                ping: PingClients::new(),
                now_ms: 1_000_000,
            }
        }

        /// Polls the sensor, then advances the clock by 10 s.
        async fn poll(&mut self, sensor: &mut dyn Sensor) -> Result<Measurement, SensorError> {
            let mut ctx = PollContext {
                host: "127.0.0.1",
                now_ms: self.now_ms,
                timeout: Duration::from_millis(500),
                ping: &self.ping,
                snmp: Some(&mut self.client),
            };
            let result = sensor.poll(&mut ctx).await;
            self.now_ms += 10_000;
            result
        }
    }

    fn values(m: &Measurement) -> Vec<(&str, Option<f64>)> {
        m.channels
            .iter()
            .map(|c| (c.key.as_str(), c.value))
            .collect()
    }

    #[tokio::test]
    async fn uptime_reports_seconds() {
        let mut rig = Rig::new().await;
        rig.agent.set_uptime(123_456);
        let m = rig.poll(&mut UptimeSensor).await.unwrap();
        assert_eq!(values(&m), [("uptime", Some(1234.56))]);
        assert_eq!(m.channels[0].unit, Unit::Seconds);
    }

    #[tokio::test]
    async fn interface_status_reports_oper_status() {
        let mut rig = Rig::new().await;
        rig.agent
            .set(&with_index(IF_OPER_STATUS, 49153), FakeValue::Integer(1));
        let mut sensor = InterfaceStatusSensor::new(49153);
        let m = rig.poll(&mut sensor).await.unwrap();
        assert_eq!(values(&m), [("oper_status", Some(1.0))]);
        rig.agent
            .set(&with_index(IF_OPER_STATUS, 49153), FakeValue::Integer(2));
        let m = rig.poll(&mut sensor).await.unwrap();
        assert_eq!(values(&m), [("oper_status", Some(2.0))]);
    }

    #[tokio::test]
    async fn interface_status_of_missing_interface_is_an_error() {
        let mut rig = Rig::new().await;
        let err = rig
            .poll(&mut InterfaceStatusSensor::new(77))
            .await
            .unwrap_err();
        assert!(matches!(err, SensorError::BadValue(_)), "{err:?}");
    }

    #[tokio::test]
    async fn traffic_uses_64_bit_counters_and_converts_to_bits() {
        let mut rig = Rig::new().await;
        let (hc_in, hc_out) = (
            with_index(IF_HC_IN_OCTETS, 5),
            with_index(IF_HC_OUT_OCTETS, 5),
        );
        let mut sensor = InterfaceTrafficSensor::new(5);

        rig.agent.set(&hc_in, FakeValue::Counter64(1_000));
        rig.agent.set(&hc_out, FakeValue::Counter64(5_000));
        let first = rig.poll(&mut sensor).await.unwrap();
        assert_eq!(values(&first), [("in", None), ("out", None)]);

        // 10 s later: +1250 and +2500 octets = 1000 and 2000 bit/s.
        rig.agent.set(&hc_in, FakeValue::Counter64(2_250));
        rig.agent.set(&hc_out, FakeValue::Counter64(7_500));
        rig.agent.set_uptime(2_000);
        let second = rig.poll(&mut sensor).await.unwrap();
        assert_eq!(
            values(&second),
            [("in", Some(1000.0)), ("out", Some(2000.0))]
        );
        assert_eq!(second.channels[0].unit, Unit::BitsPerSecond);
    }

    #[tokio::test]
    async fn traffic_falls_back_to_32_bit_counters_and_survives_a_wrap() {
        let mut rig = Rig::new().await;
        let (c_in, c_out) = (with_index(IF_IN_OCTETS, 1), with_index(IF_OUT_OCTETS, 1));
        let mut sensor = InterfaceTrafficSensor::new(1);

        rig.agent.set(&c_in, FakeValue::Counter32(u32::MAX - 999));
        rig.agent.set(&c_out, FakeValue::Counter32(0));
        rig.agent.set_uptime(1_000);
        assert_eq!(
            values(&rig.poll(&mut sensor).await.unwrap()),
            [("in", None), ("out", None)]
        );

        // The in counter wrapped: 999 up to the limit, 1 for the wrap itself, 250 after it.
        rig.agent.set(&c_in, FakeValue::Counter32(250));
        rig.agent.set(&c_out, FakeValue::Counter32(1_250));
        rig.agent.set_uptime(2_000);
        let m = rig.poll(&mut sensor).await.unwrap();
        // 1250 octets each in 10 s = 1000 bit/s.
        assert_eq!(values(&m), [("in", Some(1000.0)), ("out", Some(1000.0))]);
    }

    #[tokio::test]
    async fn traffic_skips_the_sample_after_a_device_restart() {
        let mut rig = Rig::new().await;
        let hc_in = with_index(IF_HC_IN_OCTETS, 2);
        let hc_out = with_index(IF_HC_OUT_OCTETS, 2);
        let mut sensor = InterfaceTrafficSensor::new(2);
        rig.agent.set(&hc_in, FakeValue::Counter64(1_000_000));
        rig.agent.set(&hc_out, FakeValue::Counter64(1_000_000));
        rig.agent.set_uptime(50_000);
        rig.poll(&mut sensor).await.unwrap();

        rig.agent.set(&hc_in, FakeValue::Counter64(300));
        rig.agent.set(&hc_out, FakeValue::Counter64(300));
        rig.agent.set_uptime(900);
        let m = rig.poll(&mut sensor).await.unwrap();
        assert_eq!(values(&m), [("in", None), ("out", None)]);

        // Next poll compares against the post-restart baseline again.
        rig.agent.set(&hc_in, FakeValue::Counter64(1_550));
        rig.agent.set(&hc_out, FakeValue::Counter64(300));
        rig.agent.set_uptime(1_900);
        let m = rig.poll(&mut sensor).await.unwrap();
        assert_eq!(values(&m), [("in", Some(1000.0)), ("out", Some(0.0))]);
    }

    #[tokio::test]
    async fn traffic_for_a_missing_interface_is_an_error() {
        let mut rig = Rig::new().await;
        let err = rig
            .poll(&mut InterfaceTrafficSensor::new(99))
            .await
            .unwrap_err();
        assert!(matches!(err, SensorError::BadValue(_)), "{err:?}");
    }

    #[tokio::test]
    async fn oid_gauge_is_scaled_and_accepts_numeric_text() {
        let mut rig = Rig::new().await;
        let oid = vec![1, 3, 6, 1, 4, 1, 99, 1, 0];
        let mut sensor = OidSensor::new(oid.clone(), OidMode::Gauge, 0.1);
        rig.agent.set(&oid, FakeValue::Gauge32(215));
        let m = rig.poll(&mut sensor).await.unwrap();
        assert!((m.channels[0].value.unwrap() - 21.5).abs() < 1e-9);
        rig.agent.set(&oid, FakeValue::Text(" 40 ".to_owned()));
        let m = rig.poll(&mut sensor).await.unwrap();
        assert!((m.channels[0].value.unwrap() - 4.0).abs() < 1e-9);
        rig.agent.set(&oid, FakeValue::Text("warm".to_owned()));
        assert!(matches!(
            rig.poll(&mut sensor).await,
            Err(SensorError::BadValue(_))
        ));
    }

    #[tokio::test]
    async fn oid_counter_reports_a_scaled_rate() {
        let mut rig = Rig::new().await;
        let oid = vec![1, 3, 6, 1, 4, 1, 99, 2, 0];
        let mut sensor = OidSensor::new(oid.clone(), OidMode::Counter, 2.0);
        rig.agent.set(&oid, FakeValue::Counter32(100));
        assert_eq!(
            values(&rig.poll(&mut sensor).await.unwrap()),
            [("value", None)]
        );
        rig.agent.set(&oid, FakeValue::Counter32(600));
        rig.agent.set_uptime(2_000);
        let m = rig.poll(&mut sensor).await.unwrap();
        assert_eq!(values(&m), [("value", Some(100.0))]); // 500 / 10 s * 2
        assert_eq!(m.channels[0].unit, Unit::PerSecond);
    }

    #[tokio::test]
    async fn oid_counter_rejects_gauges_and_missing_oids() {
        let mut rig = Rig::new().await;
        let oid = vec![1, 3, 6, 1, 4, 1, 99, 3, 0];
        let mut sensor = OidSensor::new(oid.clone(), OidMode::Counter, 1.0);
        assert!(matches!(
            rig.poll(&mut sensor).await,
            Err(SensorError::BadValue(_))
        ));
        rig.agent.set(&oid, FakeValue::Gauge32(5));
        assert!(matches!(
            rig.poll(&mut sensor).await,
            Err(SensorError::BadValue(_))
        ));
    }
}
