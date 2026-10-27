//! Just enough DNS for the DNS sensor: one A or AAAA question over UDP to a chosen server, and the
//! addresses of the answer. The server is asked directly (not the system resolver), so a failing
//! DNS server shows up even when another one answers for the system.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;

/// Longest DNS name (RFC 1035) without the final dot.
pub const MAX_NAME_LEN: usize = 253;
const MAX_LABEL_LEN: usize = 63;
/// Large enough for any UDP answer without EDNS.
const MAX_PACKET: usize = 1_232;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordType {
    A,
    Aaaa,
}

impl RecordType {
    fn code(self) -> u16 {
        match self {
            Self::A => 1,
            Self::Aaaa => 28,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DnsError {
    #[error("no answer in time")]
    Timeout,
    #[error("{0}")]
    Network(String),
    /// The server answered with an error code (3 = the name does not exist).
    #[error("answer code {0}")]
    Code(u8),
    #[error("answer is not valid")]
    Malformed,
}

/// Whether `name` is a valid host name to ask for (letters, digits, `-` and `_`; a final dot is allowed).
pub fn valid_name(name: &str) -> bool {
    let name = name.strip_suffix('.').unwrap_or(name);
    !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= MAX_LABEL_LEN
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

/// A query packet with recursion desired. `name` must be [`valid_name`].
pub fn build_query(id: u16, name: &str, record: RecordType) -> Vec<u8> {
    let mut packet = Vec::with_capacity(18 + name.len());
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00]); // RD
    packet.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // one question
    for label in name.strip_suffix('.').unwrap_or(name).split('.') {
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&record.code().to_be_bytes());
    packet.extend_from_slice(&[0, 1]); // class IN
    packet
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], DnsError> {
        let end = self.pos.checked_add(n).ok_or(DnsError::Malformed)?;
        let slice = self.data.get(self.pos..end).ok_or(DnsError::Malformed)?;
        self.pos = end;
        Ok(slice)
    }

    fn u16(&mut self) -> Result<u16, DnsError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    /// Skips a (possibly compressed) name; a pointer ends it.
    fn skip_name(&mut self) -> Result<(), DnsError> {
        loop {
            let len = self.take(1)?[0];
            match len {
                0 => return Ok(()),
                l if l & 0xC0 == 0xC0 => {
                    self.take(1)?;
                    return Ok(());
                }
                l if l & 0xC0 == 0 => {
                    self.take(usize::from(l))?;
                }
                _ => return Err(DnsError::Malformed),
            }
        }
    }
}

/// The addresses of type `record` in the answer to query `id`.
pub fn parse_answer(id: u16, record: RecordType, data: &[u8]) -> Result<Vec<IpAddr>, DnsError> {
    let mut r = Reader { data, pos: 0 };
    if r.u16()? != id {
        return Err(DnsError::Malformed);
    }
    let flags = r.u16()?;
    if flags & 0x8000 == 0 {
        return Err(DnsError::Malformed);
    }
    let code = (flags & 0x000F) as u8;
    if code != 0 {
        return Err(DnsError::Code(code));
    }
    let questions = r.u16()?;
    let answers = r.u16()?;
    r.take(4)?; // authority and additional counts
    for _ in 0..questions {
        r.skip_name()?;
        r.take(4)?;
    }
    let mut found = Vec::new();
    for _ in 0..answers {
        r.skip_name()?;
        let kind = r.u16()?;
        let class = r.u16()?;
        r.take(4)?; // TTL
        let len = usize::from(r.u16()?);
        let rdata = r.take(len)?;
        if class != 1 || kind != record.code() {
            continue; // e.g. the CNAME in front of the address
        }
        match (record, rdata.len()) {
            (RecordType::A, 4) => {
                found.push(IpAddr::V4(Ipv4Addr::new(
                    rdata[0], rdata[1], rdata[2], rdata[3],
                )));
            }
            (RecordType::Aaaa, 16) => {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(rdata);
                found.push(IpAddr::V6(Ipv6Addr::from(octets)));
            }
            _ => return Err(DnsError::Malformed),
        }
    }
    Ok(found)
}

/// Asks `server` for the `record` addresses of `name`, waiting at most `timeout`.
pub async fn query(
    server: SocketAddr,
    name: &str,
    record: RecordType,
    timeout: Duration,
) -> Result<Vec<IpAddr>, DnsError> {
    let network = |e: std::io::Error| DnsError::Network(e.to_string());
    let local: SocketAddr = if server.is_ipv4() {
        (Ipv4Addr::UNSPECIFIED, 0).into()
    } else {
        (Ipv6Addr::UNSPECIFIED, 0).into()
    };
    let socket = UdpSocket::bind(local).await.map_err(network)?;
    socket.connect(server).await.map_err(network)?;
    let id: u16 = rand::random();
    socket
        .send(&build_query(id, name, record))
        .await
        .map_err(network)?;
    let mut buf = vec![0u8; MAX_PACKET];
    let wait = async {
        loop {
            let n = socket.recv(&mut buf).await.map_err(network)?;
            // Answers to an earlier, timed-out query (other id) are ignored.
            match parse_answer(id, record, &buf[..n]) {
                Err(DnsError::Malformed) if n >= 2 && buf[..2] != id.to_be_bytes() => continue,
                other => return other,
            }
        }
    };
    tokio::time::timeout(timeout, wait)
        .await
        .unwrap_or(Err(DnsError::Timeout))
}

#[cfg(test)]
pub(crate) mod testserver {
    use std::net::SocketAddr;

    use tokio::net::UdpSocket;

    /// Answers every query with `addresses` (A records; empty list and `code` > 0 for errors),
    /// with a CNAME in front to exercise compressed names.
    pub async fn serve(addresses: Vec<[u8; 4]>, code: u8) -> SocketAddr {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            while let Ok((n, peer)) = socket.recv_from(&mut buf).await {
                let query = &buf[..n];
                let mut answer = Vec::new();
                answer.extend_from_slice(&query[..2]);
                answer.extend_from_slice(&[0x81, 0x80 | code]);
                answer.extend_from_slice(&[0, 1]);
                let count = if addresses.is_empty() {
                    0
                } else {
                    addresses.len() as u16 + 1
                };
                answer.extend_from_slice(&count.to_be_bytes());
                answer.extend_from_slice(&[0, 0, 0, 0]);
                answer.extend_from_slice(&query[12..]); // the question
                if !addresses.is_empty() {
                    // CNAME pointing to "x" + the question name.
                    answer.extend_from_slice(&[0xC0, 12, 0, 5, 0, 1, 0, 0, 0, 60, 0, 4]);
                    answer.extend_from_slice(&[1, b'x', 0xC0, 12]);
                    for a in &addresses {
                        answer.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
                        answer.extend_from_slice(a);
                    }
                }
                let _ = socket.send_to(&answer, peer).await;
            }
        });
        addr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked() {
        for good in ["example.org", "example.org.", "a-b_c.lan", "localhost"] {
            assert!(valid_name(good), "{good}");
        }
        let long_label = format!("{}.org", "a".repeat(64));
        let long_name = vec!["abcdefgh"; 32].join(".");
        for bad in [
            "",
            ".",
            "a..b",
            "exa mple.org",
            "ex/ample",
            "bücher.de",
            long_label.as_str(),
            long_name.as_str(),
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn queries_have_the_question_in_wire_format() {
        let q = build_query(0xBEEF, "ab.c.", RecordType::Aaaa);
        assert_eq!(
            q,
            [
                0xBE, 0xEF, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 2, b'a', b'b', 1, b'c', 0, 0, 28, 0, 1
            ]
        );
    }

    #[test]
    fn broken_answers_are_rejected_without_panicking() {
        let q = build_query(7, "example.org", RecordType::A);
        assert_eq!(parse_answer(7, RecordType::A, &q), Err(DnsError::Malformed));
        for cut in 0..q.len() {
            let mut answer = q[..cut].to_vec();
            if answer.len() > 2 {
                answer[2] |= 0x80;
            }
            let _ = parse_answer(7, RecordType::A, &answer);
        }
        let mut wrong_id = q.clone();
        wrong_id[2] |= 0x80;
        assert_eq!(
            parse_answer(8, RecordType::A, &wrong_id),
            Err(DnsError::Malformed)
        );
    }

    #[tokio::test]
    async fn asks_the_server_and_reads_the_addresses() {
        let server = testserver::serve(vec![[192, 0, 2, 7], [192, 0, 2, 8]], 0).await;
        let found = query(server, "example.org", RecordType::A, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(
            found,
            [IpAddr::from([192, 0, 2, 7]), IpAddr::from([192, 0, 2, 8])]
        );

        let missing = testserver::serve(Vec::new(), 3).await;
        let r = query(
            missing,
            "nope.example",
            RecordType::A,
            Duration::from_secs(2),
        )
        .await;
        assert_eq!(r, Err(DnsError::Code(3)));
    }

    #[tokio::test]
    async fn a_silent_server_times_out() {
        let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let r = query(
            silent.local_addr().unwrap(),
            "example.org",
            RecordType::A,
            Duration::from_millis(200),
        )
        .await;
        assert_eq!(r, Err(DnsError::Timeout));
    }
}
