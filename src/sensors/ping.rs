//! ICMP echo sensor. Uses unprivileged datagram sockets where available
//! (`net.ipv4.ping_group_range`), otherwise raw sockets (needs `CAP_NET_RAW`).

use std::net::IpAddr;

use surge_ping::{Client, Config, ICMP, PingIdentifier, PingSequence};

use super::{ChannelReading, Measurement, PollContext, PollFuture, Sensor, SensorError, Unit};

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
    sequence: u16,
}

impl PingSensor {
    pub fn new() -> Self {
        Self { sequence: 0 }
    }
}

impl Default for PingSensor {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensor for PingSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            let addr = resolve(ctx.host).await?;
            let client = ctx.ping.for_addr(addr).ok_or_else(|| {
                SensorError::Unreachable("no ICMP socket available (permissions?)".to_owned())
            })?;
            self.sequence = self.sequence.wrapping_add(1);
            let mut pinger = client.pinger(addr, PingIdentifier(rand::random())).await;
            pinger.timeout(ctx.timeout);
            let (_, rtt) = pinger
                .ping(PingSequence(self.sequence), &[0; 8])
                .await
                .map_err(|e| SensorError::Unreachable(e.to_string()))?;
            Ok(Measurement {
                channels: vec![ChannelReading {
                    key: "rtt".to_owned(),
                    unit: Unit::Milliseconds,
                    value: Some(rtt.as_secs_f64() * 1000.0),
                }],
            })
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
        match PingSensor::new().poll(&mut ctx).await {
            Ok(m) => {
                assert_eq!(m.channels.len(), 1);
                assert_eq!(m.channels[0].key, "rtt");
                assert!(m.channels[0].value.is_some_and(|v| v >= 0.0));
            }
            // Sandboxes without ping_group_range: the error must be explicit, not a panic.
            Err(SensorError::Unreachable(_)) => {}
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn literal_addresses_are_not_resolved() {
        assert_eq!(
            resolve("192.0.2.7").await.unwrap(),
            "192.0.2.7".parse::<IpAddr>().unwrap()
        );
    }
}
