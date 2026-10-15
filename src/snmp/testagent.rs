//! A minimal SNMPv2c agent for tests: answers GET requests for `sysDescr.0` and `sysUpTime.0` and ignores
//! requests with a wrong community (like real agents do). Not a general SNMP implementation.

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

const OID_SYS_DESCR: &[u8] = &[0x2b, 6, 1, 2, 1, 1, 1, 0];
const OID_SYS_UPTIME: &[u8] = &[0x2b, 6, 1, 2, 1, 1, 3, 0];

pub struct FakeAgent {
    pub port: u16,
    task: JoinHandle<()>,
}

impl Drop for FakeAgent {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeAgent {
    pub async fn spawn(community: &str, sys_descr: &str, uptime_ticks: u32) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind fake agent");
        let port = socket.local_addr().expect("local addr").port();
        let (community, sys_descr) = (community.as_bytes().to_vec(), sys_descr.as_bytes().to_vec());
        let task = tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            while let Ok((len, from)) = socket.recv_from(&mut buf).await {
                if let Some(reply) = respond(&buf[..len], &community, &sys_descr, uptime_ticks) {
                    let _ = socket.send_to(&reply, from).await;
                }
            }
        });
        Self { port, task }
    }
}

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match content.len() {
        n if n < 128 => out.push(u8::try_from(n).unwrap_or(0)),
        n if n < 256 => out.extend([0x81, u8::try_from(n).unwrap_or(0)]),
        n => out.extend([
            0x82,
            u8::try_from(n >> 8).unwrap_or(0),
            u8::try_from(n & 0xff).unwrap_or(0),
        ]),
    }
    out.extend_from_slice(content);
    out
}

/// Reads one TLV: `(tag, value, rest)`.
fn read(data: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = data.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let n = usize::from(first & 0x7f);
        let (bytes, rest) = rest.split_at_checked(n)?;
        (
            bytes
                .iter()
                .fold(0usize, |acc, b| (acc << 8) | usize::from(*b)),
            rest,
        )
    };
    let (value, rest) = rest.split_at_checked(len)?;
    Some((tag, value, rest))
}

fn unsigned(n: u32) -> Vec<u8> {
    let mut bytes: Vec<u8> = n
        .to_be_bytes()
        .into_iter()
        .skip_while(|b| *b == 0)
        .collect();
    if bytes.first().is_none_or(|b| b & 0x80 != 0) {
        bytes.insert(0, 0);
    }
    bytes
}

fn respond(
    request: &[u8],
    community: &[u8],
    sys_descr: &[u8],
    uptime_ticks: u32,
) -> Option<Vec<u8>> {
    let (_, message, _) = read(request)?;
    let (_, version, rest) = read(message)?;
    let (_, their_community, rest) = read(rest)?;
    let (pdu_tag, pdu, _) = read(rest)?;
    if their_community != community || pdu_tag != 0xa0 {
        return None;
    }
    let (_, request_id, rest) = read(pdu)?;
    let (_, _, rest) = read(rest)?; // error status
    let (_, _, rest) = read(rest)?; // error index
    let (_, mut varbinds, _) = read(rest)?;

    let mut answers = Vec::new();
    while !varbinds.is_empty() {
        let (_, varbind, remaining) = read(varbinds)?;
        varbinds = remaining;
        let (_, oid, _) = read(varbind)?;
        let oid_tlv = tlv(0x06, oid);
        let value = if oid == OID_SYS_DESCR {
            tlv(0x04, sys_descr)
        } else if oid == OID_SYS_UPTIME {
            tlv(0x43, &unsigned(uptime_ticks))
        } else {
            vec![0x80, 0x00] // noSuchObject
        };
        answers.extend(tlv(0x30, &[oid_tlv, value].concat()));
    }

    let pdu = [
        tlv(0x02, request_id),
        tlv(0x02, &[0]),
        tlv(0x02, &[0]),
        tlv(0x30, &answers),
    ]
    .concat();
    let message = [tlv(0x02, version), tlv(0x04, community), tlv(0xa2, &pdu)].concat();
    Some(tlv(0x30, &message))
}
