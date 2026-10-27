//! ICMP echo sensor. Uses unprivileged datagram sockets where available
//! (`net.ipv4.ping_group_range`), otherwise raw sockets (needs `CAP_NET_RAW`).
//! Like PRTG it sends several echo requests per poll and reports the average, minimum and maximum
//! response time, the jitter and the packet loss; a poll fails only when no packet comes back.

use std::net::IpAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use surge_ping::{Client, Config, ICMP, PingIdentifier, PingSequence, SurgeError};
use tokio::task::JoinSet;

use super::{ChannelReading, Measurement, PollContext, PollFuture, Sensor, SensorError, Unit};
use crate::i18n::Msg;

/// Echo requests per poll unless the sensor says otherwise (sensors from before 0.5.5 too).
pub const DEFAULT_PACKETS: u8 = 5;
pub const MAX_PACKETS: u8 = 10;
/// Pause between the requests of one poll. Each waits for its reply up to the device's timeout on
/// its own, so a poll takes at most the timeout plus (packets - 1) × this, within the grace the
/// poller adds to the timeout.
const PACKET_GAP: Duration = Duration::from_millis(100);

/// Settings of a ping sensor, stored as JSON in `sensors.params`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PingCheck {
    /// Echo requests per poll (1 to [`MAX_PACKETS`]).
    #[serde(default = "default_packets")]
    pub packets: u8,
}

fn default_packets() -> u8 {
    DEFAULT_PACKETS
}

impl Default for PingCheck {
    fn default() -> Self {
        Self {
            packets: DEFAULT_PACKETS,
        }
    }
}

impl PingCheck {
    pub fn validate(&self) -> Result<(), Msg> {
        if (1..=MAX_PACKETS).contains(&self.packets) {
            Ok(())
        } else {
            Err(Msg::new("ping-packets-invalid").with("max", u32::from(MAX_PACKETS)))
        }
    }
}

/// Shared ICMP sockets, one per address family; created once and cloned cheaply.
pub struct PingClients {
    v4: Option<Client>,
    v6: Option<Client>,
}

impl PingClients {
    /// A family whose socket cannot be opened (no permission, no IPv6) simply stays unavailable;
    /// pings to it then fail with a clear error instead of crashing the server.
    pub fn new() -> Self {
        let open = |kind| Client::new(&Config::builder().kind(kind).build()).ok();
        Self {
            v4: open(ICMP::V4),
            v6: open(ICMP::V6),
        }
    }

    /// Whether IPv4 echo requests can be sent (false: no permission for ICMP sockets).
    pub fn ipv4_available(&self) -> bool {
        self.v4.is_some()
    }

    fn for_addr(&self, addr: IpAddr) -> Option<&Client> {
        if addr.is_ipv4() {
            self.v4.as_ref()
        } else {
            self.v6.as_ref()
        }
    }
}

impl Default for PingClients {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PingSensor {
    packets: u8,
    /// Sequence number of the next request. Starts at random, so two sensors pinging the same host
    /// (whose replies may share an identifier on datagram sockets) do not take each other's replies.
    sequence: u16,
}

impl PingSensor {
    pub fn new(check: PingCheck) -> Self {
        Self {
            packets: check.packets.clamp(1, MAX_PACKETS),
            sequence: rand::random(),
        }
    }
}

/// What the replies of one poll add up to; `None` when none came back.
#[derive(Debug, Clone, PartialEq)]
struct Summary {
    average: f64,
    minimum: f64,
    maximum: f64,
    /// Mean difference between consecutive replies; `None` with fewer than two.
    jitter: Option<f64>,
    /// Lost requests in percent.
    loss: f64,
}

/// `replies` in the order sent, response times in milliseconds (`None`: lost).
fn summarize(replies: &[Option<f64>]) -> Option<Summary> {
    let got: Vec<f64> = replies.iter().flatten().copied().collect();
    if got.is_empty() {
        return None;
    }
    let n = got.len() as f64;
    let jitter = (got.len() > 1)
        .then(|| got.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (n - 1.0));
    Some(Summary {
        average: got.iter().sum::<f64>() / n,
        minimum: got.iter().copied().fold(f64::INFINITY, f64::min),
        maximum: got.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        jitter,
        loss: 100.0 * (replies.len() - got.len()) as f64 / replies.len() as f64,
    })
}

impl Summary {
    fn channels(&self) -> Vec<ChannelReading> {
        let reading = |key: &str, unit, value| ChannelReading {
            key: key.to_owned(),
            unit,
            value,
        };
        vec![
            reading("rtt", Unit::Milliseconds, Some(self.average)),
            reading("rtt_min", Unit::Milliseconds, Some(self.minimum)),
            reading("rtt_max", Unit::Milliseconds, Some(self.maximum)),
            reading("jitter", Unit::Milliseconds, self.jitter),
            reading("loss", Unit::Percent, Some(self.loss)),
        ]
    }
}

impl Sensor for PingSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            let addr = resolve(ctx.host).await?;
            let client = ctx.ping.for_addr(addr).ok_or_else(|| {
                SensorError::Unreachable("no ICMP socket available (permissions?)".to_owned())
            })?;
            // The requests go out one after the other and wait for their replies side by side.
            // Dropping the set (the poller's cut-off) cancels the requests still waiting.
            let ident = PingIdentifier(rand::random());
            let mut requests = JoinSet::new();
            for i in 0..self.packets {
                let seq = PingSequence(self.sequence);
                self.sequence = self.sequence.wrapping_add(1);
                let (client, timeout) = (client.clone(), ctx.timeout);
                requests.spawn(async move {
                    tokio::time::sleep(PACKET_GAP * u32::from(i)).await;
                    let mut pinger = client.pinger(addr, ident).await;
                    pinger.timeout(timeout);
                    let reply = pinger.ping(seq, &[0; 8]).await;
                    (i, reply.map(|(_, rtt)| rtt.as_secs_f64() * 1000.0))
                });
            }
            let mut replies = vec![None; usize::from(self.packets)];
            let mut failure: Option<SurgeError> = None;
            while let Some(done) = requests.join_next().await {
                let Ok((i, reply)) = done else { continue };
                match reply {
                    Ok(ms) => replies[usize::from(i)] = Some(ms),
                    // A lost packet; another error (network unreachable …) explains more.
                    Err(e) if failure.is_none() || !matches!(e, SurgeError::Timeout { .. }) => {
                        failure = Some(e);
                    }
                    Err(_) => {}
                }
            }
            match summarize(&replies) {
                Some(summary) => Ok(Measurement {
                    note: None,
                    channels: summary.channels(),
                }),
                None => Err(SensorError::Unreachable(
                    failure.map_or_else(|| "no reply".to_owned(), |e| e.to_string()),
                )),
            }
        })
    }
}

async fn resolve(host: &str) -> Result<IpAddr, SensorError> {
    if let Ok(addr) = host.parse::<IpAddr>() {
        return Ok(addr);
    }
    let mut addrs = tokio::net::lookup_host((host, 0))
        .await
        .map_err(|e| SensorError::Unreachable(format!("cannot resolve {host}: {e}")))?;
    addrs
        .next()
        .map(|a| a.ip())
        .ok_or_else(|| SensorError::Unreachable(format!("cannot resolve {host}")))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn pings_loopback_or_reports_missing_permission() {
        let clients = PingClients::new();
        let mut ctx = PollContext {
            host: "127.0.0.1",
            now_ms: 0,
            timeout: Duration::from_secs(2),
            snmp: None,
            ping: &clients,
        };
        match PingSensor::new(PingCheck::default()).poll(&mut ctx).await {
            Ok(m) => {
                let keys: Vec<_> = m.channels.iter().map(|c| c.key.as_str()).collect();
                assert_eq!(keys, ["rtt", "rtt_min", "rtt_max", "jitter", "loss"]);
                assert!(m.channels[0].value.is_some_and(|v| v >= 0.0));
                assert_eq!(m.channels[4].value, Some(0.0), "loopback loses nothing");
            }
            // Sandboxes without ping_group_range: the error must be explicit, not a panic.
            Err(SensorError::Unreachable(_)) => {}
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn replies_add_up_to_average_extremes_jitter_and_loss() {
        let all = summarize(&[Some(1.0), Some(3.0), Some(2.0), Some(2.0), Some(2.0)]).unwrap();
        assert_eq!(
            (all.average, all.minimum, all.maximum, all.loss),
            (2.0, 1.0, 3.0, 0.0)
        );
        // |3-1| + |2-3| + 0 + 0 over four steps
        assert_eq!(all.jitter, Some(0.75));

        let some = summarize(&[Some(4.0), None, Some(2.0), None, Some(6.0)]).unwrap();
        assert_eq!((some.average, some.loss), (4.0, 40.0));
        // lost packets are skipped: |2-4| and |6-2|
        assert_eq!(some.jitter, Some(3.0));

        let one = summarize(&[None, Some(5.0)]).unwrap();
        assert_eq!((one.average, one.jitter, one.loss), (5.0, None, 50.0));
        assert_eq!(
            one.channels()[3].value,
            None,
            "no jitter from a single reply"
        );

        assert_eq!(summarize(&[None, None, None]), None);
    }

    #[test]
    fn packets_are_checked_and_default_for_old_sensors() {
        assert!(PingCheck { packets: 1 }.validate().is_ok());
        assert!(
            PingCheck {
                packets: MAX_PACKETS
            }
            .validate()
            .is_ok()
        );
        assert!(PingCheck { packets: 0 }.validate().is_err());
        assert!(
            PingCheck {
                packets: MAX_PACKETS + 1
            }
            .validate()
            .is_err()
        );
        let old: PingCheck = serde_json::from_str("{}").unwrap();
        assert_eq!(old.packets, DEFAULT_PACKETS);
    }

    #[tokio::test]
    async fn a_host_that_never_answers_fails_after_one_timeout() {
        let clients = PingClients::new();
        if !clients.ipv4_available() {
            return;
        }
        let mut ctx = PollContext {
            host: "192.0.2.1",
            now_ms: 0,
            timeout: Duration::from_millis(300),
            snmp: None,
            ping: &clients,
        };
        let started = std::time::Instant::now();
        let result = PingSensor::new(PingCheck::default()).poll(&mut ctx).await;
        assert!(
            matches!(result, Err(SensorError::Unreachable(_))),
            "{result:?}"
        );
        // timeout + 4 × gap, not 5 × timeout
        assert!(
            started.elapsed() < Duration::from_millis(1200),
            "{:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn literal_addresses_are_not_resolved() {
        assert_eq!(
            resolve("192.0.2.7").await.unwrap(),
            "192.0.2.7".parse::<IpAddr>().unwrap()
        );
    }
}
