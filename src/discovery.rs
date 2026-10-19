//! Interface discovery: walks the IF-MIB tables of a device and merges the columns per interface.
//! Names come from `ifName` (switches), falling back to `ifDescr` (access points have no `ifXTable`).

use std::collections::BTreeMap;

use crate::snmp::{SnmpClient, SnmpError, SnmpValue};

const IF_DESCR: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 2];
const IF_TYPE: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 3];
const IF_SPEED: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 5];
const IF_OPER_STATUS: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 8];
const IF_NAME: &[u64] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 1];
const IF_HIGH_SPEED: &[u64] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 15];
const IF_ALIAS: &[u64] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 18];

/// Upper bound per table; a device with more interfaces than this is not a normal switch or AP.
const MAX_ROWS: usize = 2_000;
const MAX_TEXT_CHARS: usize = 64;

/// `ifType` values worth monitoring by default: ethernet, gigabit ethernet, fast ethernet, Wi-Fi.
const MONITORED_TYPES: [i64; 5] = [6, 62, 69, 71, 117];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredInterface {
    pub index: u32,
    /// `ifName`, else `ifDescr`, else `Interface <index>`; cleaned and at most 64 characters.
    pub name: String,
    pub descr: String,
    pub alias: String,
    pub if_type: Option<i64>,
    /// `ifOperStatus` code (1 = up).
    pub oper_status: Option<i64>,
    pub speed_mbps: Option<u64>,
}

impl DiscoveredInterface {
    pub fn is_up(&self) -> bool {
        self.oper_status == Some(1)
    }

    /// A physical or Wi-Fi port (not a VLAN interface, loopback, tunnel, …).
    pub fn is_port(&self) -> bool {
        self.if_type.is_some_and(|t| MONITORED_TYPES.contains(&t))
    }

    /// Whether the interface is a good default choice (an up port of a physical or Wi-Fi type).
    pub fn suggested(&self) -> bool {
        self.is_up() && self.is_port()
    }

    pub fn oper_status_label(&self) -> &'static str {
        match self.oper_status {
            Some(1) => "up",
            Some(2) => "down",
            Some(3) => "testing",
            Some(5) => "dormant",
            Some(6) => "notPresent",
            Some(7) => "lowerLayerDown",
            _ => "unknown",
        }
    }
}

type Column = Vec<(Vec<u64>, SnmpValue)>;

/// The walked columns, as returned by [`SnmpClient::walk`].
#[derive(Default)]
pub struct Columns {
    pub descr: Column,
    pub if_type: Column,
    pub speed: Column,
    pub oper_status: Column,
    pub name: Column,
    pub high_speed: Column,
    pub alias: Column,
}

fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Rows of a column keyed by their `ifIndex` (the last OID arc).
fn by_index(column: &Column) -> BTreeMap<u32, &SnmpValue> {
    column
        .iter()
        .filter_map(|(oid, value)| {
            let index = u32::try_from(*oid.last()?).ok()?;
            Some((index, value))
        })
        .collect()
}

fn text(value: Option<&&SnmpValue>) -> String {
    match value {
        Some(SnmpValue::Text(t)) => clean(t),
        _ => String::new(),
    }
}

fn integer(value: Option<&&SnmpValue>) -> Option<i64> {
    match value? {
        SnmpValue::Integer(n) => Some(*n),
        SnmpValue::Gauge32(n) => Some(i64::from(*n)),
        _ => None,
    }
}

/// Merges the columns into one entry per interface, ordered by `ifIndex`.
pub fn merge(columns: &Columns) -> Vec<DiscoveredInterface> {
    let (descr, if_type, speed, oper) = (
        by_index(&columns.descr),
        by_index(&columns.if_type),
        by_index(&columns.speed),
        by_index(&columns.oper_status),
    );
    let (name, high_speed, alias) = (
        by_index(&columns.name),
        by_index(&columns.high_speed),
        by_index(&columns.alias),
    );
    let mut indexes: Vec<u32> = descr.keys().chain(name.keys()).copied().collect();
    indexes.sort_unstable();
    indexes.dedup();
    indexes
        .into_iter()
        .map(|index| {
            let if_name = text(name.get(&index));
            let if_descr = text(descr.get(&index));
            let display = [&if_name, &if_descr]
                .into_iter()
                .find(|t| !t.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Interface {index}"));
            let speed_mbps = integer(high_speed.get(&index))
                .filter(|s| *s > 0)
                .or_else(|| integer(speed.get(&index)).map(|bps| bps / 1_000_000))
                .and_then(|s| u64::try_from(s).ok())
                .filter(|s| *s > 0);
            DiscoveredInterface {
                index,
                name: display,
                descr: if_descr,
                alias: text(alias.get(&index)),
                if_type: integer(if_type.get(&index)),
                oper_status: integer(oper.get(&index)),
                speed_mbps,
            }
        })
        .collect()
}

/// Walks all needed IF-MIB columns. Columns an agent does not have (no `ifXTable`) are simply empty.
pub async fn discover_interfaces(
    client: &mut SnmpClient,
) -> Result<Vec<DiscoveredInterface>, SnmpError> {
    let columns = Columns {
        descr: client.walk(IF_DESCR, MAX_ROWS).await?,
        if_type: client.walk(IF_TYPE, MAX_ROWS).await?,
        speed: client.walk(IF_SPEED, MAX_ROWS).await?,
        oper_status: client.walk(IF_OPER_STATUS, MAX_ROWS).await?,
        name: client.walk(IF_NAME, MAX_ROWS).await?,
        high_speed: client.walk(IF_HIGH_SPEED, MAX_ROWS).await?,
        alias: client.walk(IF_ALIAS, MAX_ROWS).await?,
    };
    Ok(merge(&columns))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{Credentials, Secret};
    use crate::snmp::SnmpTarget;
    use crate::snmp::testagent::{FakeAgent, FakeValue};
    use std::time::Duration;

    fn row(column: &[u64], index: u64, value: SnmpValue) -> (Vec<u64>, SnmpValue) {
        let mut oid = column.to_vec();
        oid.push(index);
        (oid, value)
    }

    fn text_value(t: &str) -> SnmpValue {
        SnmpValue::Text(t.to_owned())
    }

    #[test]
    fn switch_interfaces_use_if_name_and_high_speed_with_sparse_indexes() {
        let columns = Columns {
            descr: vec![
                row(IF_DESCR, 1, text_value("gigabitEthernet 1/0/1")),
                row(IF_DESCR, 49153, text_value("vlan1")),
            ],
            if_type: vec![
                row(IF_TYPE, 1, SnmpValue::Integer(6)),
                row(IF_TYPE, 49153, SnmpValue::Integer(53)),
            ],
            oper_status: vec![
                row(IF_OPER_STATUS, 1, SnmpValue::Integer(1)),
                row(IF_OPER_STATUS, 49153, SnmpValue::Integer(1)),
            ],
            name: vec![
                row(IF_NAME, 1, text_value("Gi1/0/1")),
                row(IF_NAME, 49153, text_value("Vl1")),
            ],
            high_speed: vec![row(IF_HIGH_SPEED, 1, SnmpValue::Gauge32(1000))],
            alias: vec![row(IF_ALIAS, 1, text_value("Uplink"))],
            ..Columns::default()
        };
        let found = merge(&columns);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].index, 1);
        assert_eq!(found[0].name, "Gi1/0/1");
        assert_eq!(found[0].descr, "gigabitEthernet 1/0/1");
        assert_eq!(found[0].alias, "Uplink");
        assert_eq!(found[0].speed_mbps, Some(1000));
        assert!(found[0].suggested());
        assert_eq!(found[1].index, 49153);
        assert!(found[1].is_up());
        assert!(!found[1].suggested(), "VLAN interfaces are not suggested");
    }

    #[test]
    fn access_points_fall_back_to_if_descr_and_if_speed() {
        let columns = Columns {
            descr: vec![
                row(IF_DESCR, 1, text_value("eth0")),
                row(IF_DESCR, 2, text_value("ath0")),
            ],
            if_type: vec![
                row(IF_TYPE, 1, SnmpValue::Integer(6)),
                row(IF_TYPE, 2, SnmpValue::Integer(71)),
            ],
            speed: vec![
                row(IF_SPEED, 1, SnmpValue::Gauge32(1_000_000_000)),
                row(IF_SPEED, 2, SnmpValue::Gauge32(0)),
            ],
            oper_status: vec![
                row(IF_OPER_STATUS, 1, SnmpValue::Integer(1)),
                row(IF_OPER_STATUS, 2, SnmpValue::Integer(2)),
            ],
            ..Columns::default()
        };
        let found = merge(&columns);
        assert_eq!(found[0].name, "eth0");
        assert_eq!(found[0].speed_mbps, Some(1000));
        assert_eq!(found[1].speed_mbps, None);
        assert_eq!(found[1].oper_status_label(), "down");
        assert!(!found[1].suggested(), "down ports are not suggested");
        assert_eq!(found[0].alias, "");
    }

    #[test]
    fn missing_or_blank_names_get_a_placeholder_and_text_is_cleaned() {
        let columns = Columns {
            descr: vec![
                row(IF_DESCR, 7, text_value("  ")),
                row(
                    IF_DESCR,
                    8,
                    text_value(&format!("a\u{0007}b{}", "x".repeat(100))),
                ),
            ],
            ..Columns::default()
        };
        let found = merge(&columns);
        assert_eq!(found[0].name, "Interface 7");
        assert!(!found[1].name.contains('\u{0007}'));
        assert_eq!(found[1].name.chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn nothing_discovered_gives_an_empty_list() {
        assert!(merge(&Columns::default()).is_empty());
    }

    #[tokio::test]
    async fn discovers_interfaces_over_the_wire() {
        let agent = FakeAgent::spawn("public", "x", 1).await;
        for (index, name, status) in [(1u64, "Port 1", 1), (2, "Port 2", 2), (49153, "Vlan", 1)] {
            agent.set(
                &[IF_DESCR, &[index]].concat(),
                FakeValue::Text(name.to_owned()),
            );
            agent.set(&[IF_TYPE, &[index]].concat(), FakeValue::Integer(6));
            agent.set(
                &[IF_OPER_STATUS, &[index]].concat(),
                FakeValue::Integer(status),
            );
        }
        let creds = Credentials::V2c {
            community: Secret::new("public"),
        };
        let target = SnmpTarget {
            host: "127.0.0.1".to_owned(),
            port: agent.port,
            timeout: Duration::from_secs(2),
            retries: 0,
        };
        let mut client = SnmpClient::connect(&target, &creds).await.unwrap();
        let found = discover_interfaces(&mut client).await.unwrap();
        let names: Vec<_> = found
            .iter()
            .map(|i| (i.index, i.name.as_str(), i.is_up()))
            .collect();
        assert_eq!(
            names,
            [
                (1, "Port 1", true),
                (2, "Port 2", false),
                (49153, "Vlan", true)
            ]
        );
    }
}
