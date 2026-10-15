//! A minimal SNMPv2c agent for tests: answers GET requests from a mutable OID table (preloaded with
//! `sysDescr.0` and `sysUpTime.0`) and ignores requests with a wrong community (like real agents do).
//! Unknown OIDs yield `noSuchInstance`. Not a general SNMP implementation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use super::{SYS_DESCR, SYS_UPTIME};

/// A value the fake agent can serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeValue {
    Text(String),
    Integer(i64),
    Counter32(u32),
    Gauge32(u32),
    Timeticks(u32),
    Counter64(u64),
}

type Table = Arc<Mutex<HashMap<Vec<u64>, FakeValue>>>;

pub struct FakeAgent {
    pub port: u16,
    table: Table,
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
        let community = community.as_bytes().to_vec();
        let table: Table = Arc::new(Mutex::new(HashMap::from([
            (SYS_DESCR.to_vec(), FakeValue::Text(sys_descr.to_owned())),
            (SYS_UPTIME.to_vec(), FakeValue::Timeticks(uptime_ticks)),
        ])));
        let served = Arc::clone(&table);
        let task = tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            while let Ok((len, from)) = socket.recv_from(&mut buf).await {
                if let Some(reply) = respond(&buf[..len], &community, &served) {
                    let _ = socket.send_to(&reply, from).await;
                }
            }
        });
        Self { port, table, task }
    }

    /// Sets (or replaces) the value served for `oid`.
    pub fn set(&self, oid: &[u64], value: FakeValue) {
        if let Ok(mut table) = self.table.lock() {
            table.insert(oid.to_vec(), value);
        }
    }

    pub fn set_uptime(&self, ticks: u32) {
        self.set(SYS_UPTIME, FakeValue::Timeticks(ticks));
    }
}

fn decode_oid(bytes: &[u8]) -> Vec<u64> {
    let Some((&first, rest)) = bytes.split_first() else {
        return Vec::new();
    };
    let mut oid = vec![u64::from(first / 40), u64::from(first % 40)];
    let mut acc = 0u64;
    for b in rest {
        acc = (acc << 7) | u64::from(b & 0x7f);
        if b & 0x80 == 0 {
            oid.push(acc);
            acc = 0;
        }
    }
    oid
}

fn signed(n: i64) -> Vec<u8> {
    let bytes = n.to_be_bytes();
    let mut start = 0;
    while start < 7
        && ((bytes[start] == 0 && bytes[start + 1] & 0x80 == 0)
            || (bytes[start] == 0xff && bytes[start + 1] & 0x80 != 0))
    {
        start += 1;
    }
    bytes[start..].to_vec()
}

fn encode_value(value: &FakeValue) -> Vec<u8> {
    match value {
        FakeValue::Text(t) => tlv(0x04, t.as_bytes()),
        FakeValue::Integer(n) => tlv(0x02, &signed(*n)),
        FakeValue::Counter32(n) => tlv(0x41, &unsigned(u64::from(*n))),
        FakeValue::Gauge32(n) => tlv(0x42, &unsigned(u64::from(*n))),
        FakeValue::Timeticks(n) => tlv(0x43, &unsigned(u64::from(*n))),
        FakeValue::Counter64(n) => tlv(0x46, &unsigned(*n)),
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

fn unsigned(n: u64) -> Vec<u8> {
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

fn respond(request: &[u8], community: &[u8], table: &Table) -> Option<Vec<u8>> {
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
        let looked_up = table.lock().ok()?.get(&decode_oid(oid)).cloned();
        let value = match looked_up {
            Some(v) => encode_value(&v),
            None => vec![0x81, 0x00], // noSuchInstance
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
