//! Sensors: one measurement at one device, producing one or more channel values.
//!
//! New sensor types implement [`Sensor`]; the poller never special-cases a type.

pub mod counter;

use std::future::Future;
use std::pin::Pin;

use crate::snmp::{SnmpClient, SnmpError};

/// Unit of a channel value (for display and graph axes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    BitsPerSecond,
    Milliseconds,
    Seconds,
    Percent,
    Count,
    /// Status code, e.g. `ifOperStatus` (1 = up).
    State,
}

/// One channel value of a poll. `value: None` means "nothing to store" (first poll of a
/// counter, counter reset): the channel gets no sample, which is not an error.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelReading {
    pub key: String,
    pub unit: Unit,
    pub value: Option<f64>,
}

/// Result of one successful poll.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Measurement {
    pub channels: Vec<ChannelReading>,
}

#[derive(Debug, thiserror::Error)]
pub enum SensorError {
    #[error("SNMP: {0}")]
    Snmp(#[from] SnmpError),
    #[error("device returned no usable value: {0}")]
    BadValue(String),
    #[error("sensor needs an SNMP session but none is available")]
    NoSession,
}

/// What a sensor may use during a poll.
pub struct PollContext<'a> {
    pub host: &'a str,
    /// Poll time, milliseconds since the Unix epoch (UTC).
    pub now_ms: i64,
    /// `None` for sensors that do not use SNMP (ping) or if the session could not be opened.
    pub snmp: Option<&'a mut SnmpClient>,
}

impl<'a> PollContext<'a> {
    pub fn snmp(&mut self) -> Result<&mut SnmpClient, SensorError> {
        self.snmp.as_deref_mut().ok_or(SensorError::NoSession)
    }
}

pub type PollFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Measurement, SensorError>> + Send + 'a>>;

pub trait Sensor: Send {
    /// Takes one measurement. Sensors keep their own state (e.g. the previous counter value).
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed;

    impl Sensor for Fixed {
        fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
            Box::pin(async move {
                ctx.snmp()?;
                Ok(Measurement::default())
            })
        }
    }

    #[tokio::test]
    async fn sensor_is_object_safe_and_reports_missing_session() {
        let mut sensor: Box<dyn Sensor> = Box::new(Fixed);
        let mut ctx = PollContext {
            host: "192.0.2.1",
            now_ms: 0,
            snmp: None,
        };
        assert!(matches!(
            sensor.poll(&mut ctx).await,
            Err(SensorError::NoSession)
        ));
    }
}
