//! Finds measurable values on a device: walks known tables and scalar groups from the catalog
//! (or any table/group the admin names) and turns every readable number into a proposed custom
//! OID sensor with a readable name, factor, unit and the current value.

use std::collections::{BTreeMap, HashMap};

use crate::mib::catalog::{
    CatalogEntry, RowLabel, RowRule, entity_sensor_unit, unit_from_description,
};
use crate::mib::{MibIndex, MibObject, ObjectKind};
use crate::sensors::Unit;
use crate::sensors::format::format_value;
use crate::sensors::snmp::{OidMode, numeric};
use crate::snmp::{SnmpClient, SnmpError, SnmpValue};

/// Upper bound of values read per table or group.
const MAX_ROWS: usize = 3_000;
/// Upper bound of proposals per group (huge tables are cut, with a note).
const MAX_CANDIDATES: usize = 500;
const MAX_NAME_CHARS: usize = 64;

const IF_NAME: &[u64] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 1];
const IF_DESCR: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 2];
const ENT_PHYSICAL_NAME: &[u64] = &[1, 3, 6, 1, 2, 1, 47, 1, 1, 1, 1, 7];

/// A proposed sensor.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub oid: Vec<u64>,
    pub name: String,
    pub object: String,
    pub column: String,
    pub row: String,
    /// The current value, formatted with factor, unit and enumeration label.
    pub value: String,
    pub scale: f64,
    pub unit: Option<String>,
    pub mode: OidMode,
    pub selected: bool,
}

/// The values found for one table or group.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub title: String,
    /// `MODULE::object`
    pub source: String,
    pub candidates: Vec<Candidate>,
    /// E.g. "gekürzt auf 500 Werte".
    pub note: Option<String>,
}

/// What to look at.
pub enum Target<'a> {
    Catalog(&'static CatalogEntry),
    /// A table or an OBJECT IDENTIFIER with scalars below it.
    Object(&'a MibObject),
}

/// A value column as discovery uses it.
struct Column<'a> {
    object: &'a MibObject,
    label: String,
    scale: f64,
    unit: Option<String>,
    selected: bool,
}

fn default_unit(o: &MibObject) -> (f64, Option<String>) {
    if let Some(u) = &o.units {
        return (1.0, Some(u.clone()));
    }
    match o.description.as_deref().and_then(unit_from_description) {
        Some((factor, unit)) => (factor, Some(unit)),
        None => (1.0, None),
    }
}

/// Value columns of a generic table or group: every readable number; read-only values (measurements)
/// are ticked, writable ones (usually settings) are not.
fn generic_columns<'a>(index: &'a MibIndex, root: &MibObject) -> Vec<Column<'a>> {
    index
        .under(&root.oid)
        .into_iter()
        .filter(|o| o.is_pollable())
        .filter(|o| match root.kind {
            ObjectKind::Table => o.kind == ObjectKind::Column,
            _ => o.kind == ObjectKind::Scalar,
        })
        .map(|o| {
            let (scale, unit) = default_unit(o);
            Column {
                object: o,
                label: o.name.clone(),
                scale,
                unit,
                selected: o.access.as_deref() == Some("read-only"),
            }
        })
        .collect()
}

fn catalog_columns<'a>(index: &'a MibIndex, entry: &CatalogEntry) -> Vec<Column<'a>> {
    entry
        .columns
        .iter()
        .filter_map(|spec| {
            let o = *index.by_name(spec.object, Some(entry.module)).first()?;
            Some(Column {
                object: o,
                label: spec.label.to_owned(),
                scale: spec.scale,
                unit: spec.unit.map(str::to_owned),
                selected: spec.selected,
            })
        })
        .collect()
}

fn format_candidate_value(o: &MibObject, raw: f64, scale: f64, unit: Option<&str>) -> String {
    if scale == 1.0
        && raw.fract() == 0.0
        && let Some(label) = o.enum_label(raw as i64)
    {
        return format!("{label} ({raw})");
    }
    let shown = format_value(Unit::Count, raw * scale);
    match unit {
        Some(u) if o.base.is_some_and(|b| b.is_counter()) => format!("{shown} {u} (Zählerstand)"),
        Some(u) => format!("{shown} {u}"),
        None if o.base.is_some_and(|b| b.is_counter()) => format!("{shown} (Zählerstand)"),
        None => shown,
    }
}

fn sensor_name(column: &str, row: &str) -> String {
    let name = if row.is_empty() {
        column.to_owned()
    } else {
        format!("{column} {row}")
    };
    name.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect()
}

fn text(value: &SnmpValue) -> Option<String> {
    match value {
        SnmpValue::Text(t) => {
            let t: String = t.chars().filter(|c| !c.is_control()).collect();
            let t = t.trim().to_owned();
            (!t.is_empty()).then_some(t)
        }
        _ => None,
    }
}

/// Text values of one column by row index, e.g. interface names.
async fn names_by_index(client: &mut SnmpClient, column: &[u64]) -> HashMap<Vec<u64>, String> {
    match client.walk(column, MAX_ROWS).await {
        Ok(rows) => rows
            .into_iter()
            .filter_map(|(oid, v)| Some((oid.get(column.len()..)?.to_vec(), text(&v)?)))
            .collect(),
        Err(_) => HashMap::new(),
    }
}

fn dotted(arcs: &[u64]) -> String {
    crate::mib::dotted(arcs)
}

/// Looks at one table or group and proposes sensors.
async fn discover_one(
    client: &mut SnmpClient,
    index: &MibIndex,
    root: &MibObject,
    title: String,
    columns: Vec<Column<'_>>,
    rows_label: Option<RowLabel>,
    rule: RowRule,
) -> Result<Option<Group>, SnmpError> {
    if columns.is_empty() {
        return Ok(None);
    }
    let walked = client.walk(&root.oid, MAX_ROWS).await?;
    if walked.is_empty() {
        return Ok(None);
    }
    let source = format!("{}::{}", root.module, root.name);
    let mut candidates = Vec::new();

    if root.kind != ObjectKind::Table {
        // Scalars: `object.0`.
        let values: HashMap<&[u64], &SnmpValue> =
            walked.iter().map(|(o, v)| (o.as_slice(), v)).collect();
        for c in &columns {
            let mut oid = c.object.oid.clone();
            oid.push(0);
            let Some(raw) = values.get(oid.as_slice()).and_then(|v| numeric(v)) else {
                continue;
            };
            candidates.push(Candidate {
                name: sensor_name(&c.label, ""),
                object: c.object.name.clone(),
                column: c.label.clone(),
                row: String::new(),
                value: format_candidate_value(c.object, raw, c.scale, c.unit.as_deref()),
                scale: c.scale,
                unit: c.unit.clone(),
                mode: mode_of(c.object),
                selected: c.selected,
                oid,
            });
        }
        if candidates.is_empty() {
            return Ok(None);
        }
        return Ok(Some(Group {
            title,
            source,
            candidates,
            note: None,
        }));
    }

    // Table: index → column OID → value.
    let entry_len = root.oid.len() + 1;
    let mut rows: BTreeMap<Vec<u64>, HashMap<Vec<u64>, &SnmpValue>> = BTreeMap::new();
    for (oid, value) in &walked {
        if oid.len() <= entry_len + 1 {
            continue;
        }
        let column = oid[..entry_len + 1].to_vec();
        let row = oid[entry_len + 1..].to_vec();
        rows.entry(row).or_default().insert(column, value);
    }

    // Row labels.
    let columns_by_name: HashMap<&str, &MibObject> = index
        .under(&root.oid)
        .into_iter()
        .filter(|o| o.kind == ObjectKind::Column)
        .map(|o| (o.name.as_str(), o))
        .collect();
    let index_names = columns
        .first()
        .map(|c| c.object.index.clone())
        .unwrap_or_default();
    let mut external: HashMap<Vec<u64>, String> = HashMap::new();
    let label_column: Option<&MibObject> = match rows_label {
        Some(RowLabel::Column(name)) => columns_by_name.get(name).copied(),
        Some(RowLabel::Physical) => {
            external = names_by_index(client, ENT_PHYSICAL_NAME).await;
            None
        }
        Some(RowLabel::Pattern(_)) => None,
        None => {
            if index_names.iter().any(|n| n == "ifIndex") {
                external = names_by_index(client, IF_NAME).await;
                if external.is_empty() {
                    external = names_by_index(client, IF_DESCR).await;
                }
                None
            } else {
                // A text column that names the row.
                columns_by_name
                    .values()
                    .copied()
                    .filter(|o| o.base == Some(crate::mib::BaseType::OctetString))
                    .filter(|o| {
                        let n = o.name.to_lowercase();
                        ["name", "descr", "description", "label", "port"]
                            .iter()
                            .any(|k| n.ends_with(k))
                    })
                    .min_by_key(|o| o.oid.clone())
            }
        }
    };
    let row_name =
        |position: usize, row: &[u64], cells: &HashMap<Vec<u64>, &SnmpValue>| -> String {
            let i = dotted(row);
            if let Some(col) = label_column
                && let Some(name) = cells.get(&col.oid).and_then(|v| text(v))
            {
                return name;
            }
            if let Some(name) = external.get(row) {
                return name.clone();
            }
            match rows_label {
                Some(RowLabel::Pattern(p)) => p
                    .replace("{i}", &i)
                    .replace("{n}", &(position + 1).to_string()),
                Some(RowLabel::Physical) => format!("Sensor {i}"),
                _ => format!("Index {i}"),
            }
        };
    let column_value = |cells: &HashMap<Vec<u64>, &SnmpValue>, name: &str| -> Option<f64> {
        let o = columns_by_name.get(name)?;
        cells.get(&o.oid).and_then(|v| numeric(v))
    };

    let mut truncated = false;
    'rows: for (position, (row, cells)) in rows.iter().enumerate() {
        let row_label = row_name(position, row, cells);
        for c in &columns {
            let Some(raw) = cells.get(&c.object.oid).and_then(|v| numeric(v)) else {
                continue;
            };
            let (scale, unit) = match rule {
                RowRule::EntitySensor if c.object.name == "entPhySensorValue" => {
                    let kind = column_value(cells, "entPhySensorType").unwrap_or(1.0) as i64;
                    let scale = column_value(cells, "entPhySensorScale").unwrap_or(9.0) as i64;
                    let precision =
                        column_value(cells, "entPhySensorPrecision").unwrap_or(0.0) as i64;
                    let (factor, unit) = entity_sensor_unit(kind, scale, precision);
                    (factor, unit.map(str::to_owned))
                }
                RowRule::HrStorage => {
                    let units = column_value(cells, "hrStorageAllocationUnits").unwrap_or(1.0);
                    (units / 1_000_000.0, c.unit.clone())
                }
                _ => (c.scale, c.unit.clone()),
            };
            if candidates.len() >= MAX_CANDIDATES {
                truncated = true;
                break 'rows;
            }
            let mut oid = c.object.oid.clone();
            oid.extend_from_slice(row);
            candidates.push(Candidate {
                name: sensor_name(&c.label, &row_label),
                object: c.object.name.clone(),
                column: c.label.clone(),
                row: row_label.clone(),
                value: format_candidate_value(c.object, raw, scale, unit.as_deref()),
                scale,
                unit,
                mode: mode_of(c.object),
                selected: c.selected,
                oid,
            });
        }
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    Ok(Some(Group {
        title,
        source,
        candidates,
        note: truncated.then(|| format!("Gekürzt auf {MAX_CANDIDATES} Werte.")),
    }))
}

fn mode_of(o: &MibObject) -> OidMode {
    if o.base.is_some_and(|b| b.is_counter()) {
        OidMode::Counter
    } else {
        OidMode::Gauge
    }
}

/// Walks every target and returns the groups that have values. A target the device does not
/// answer (or whose MIB is not loaded) is simply left out; device-level SNMP errors are returned.
pub async fn discover(
    client: &mut SnmpClient,
    index: &MibIndex,
    targets: &[Target<'_>],
) -> Result<Vec<Group>, SnmpError> {
    let mut groups = Vec::new();
    for target in targets {
        let found = match target {
            Target::Catalog(entry) => {
                let Some(root) = index
                    .by_name(entry.object, Some(entry.module))
                    .first()
                    .copied()
                else {
                    continue;
                };
                let columns = catalog_columns(index, entry);
                discover_one(
                    client,
                    index,
                    root,
                    entry.title.to_owned(),
                    columns,
                    Some(entry.rows),
                    entry.rule,
                )
                .await
            }
            Target::Object(root) => {
                let columns = generic_columns(index, root);
                discover_one(
                    client,
                    index,
                    root,
                    format!("{} ({})", root.name, root.module),
                    columns,
                    None,
                    RowRule::Plain,
                )
                .await
            }
        };
        match found {
            Ok(Some(group)) => groups.push(group),
            Ok(None) => {}
            // The whole device does not answer: no point in trying the other tables.
            Err(e @ (SnmpError::Timeout | SnmpError::Auth | SnmpError::Unreachable)) => {
                return Err(e);
            }
            // A single table the agent cannot deliver (protocol oddities): skip it.
            Err(e) => tracing::debug!(error = %e, "value discovery: table skipped"),
        }
    }
    Ok(groups)
}

/// The catalog targets (all of them; entries whose MIB is not loaded are skipped by `discover`).
pub fn catalog_targets() -> Vec<Target<'static>> {
    crate::mib::catalog::CATALOG
        .iter()
        .map(Target::Catalog)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{Credentials, Secret};
    use crate::mib::{Origin, Source};
    use crate::snmp::SnmpTarget;
    use crate::snmp::testagent::{FakeAgent, FakeValue};
    use std::time::Duration;

    const VENDOR: &str = r#"
TEST-POE-MIB DEFINITIONS ::= BEGIN
IMPORTS MODULE-IDENTITY, OBJECT-TYPE, Counter32, enterprises FROM SNMPv2-SMI DisplayString FROM SNMPv2-TC;
testMIB MODULE-IDENTITY LAST-UPDATED "202401010000Z" ORGANIZATION "T" CONTACT-INFO "x" DESCRIPTION "x" ::= { enterprises 64001 }
testGlobal OBJECT IDENTIFIER ::= { testMIB 1 }
testConsumption OBJECT-TYPE SYNTAX INTEGER MAX-ACCESS read-only STATUS current
    DESCRIPTION "Total consumption. The unit is 0.1W." ::= { testGlobal 1 }
testLimit OBJECT-TYPE SYNTAX INTEGER MAX-ACCESS read-write STATUS current
    DESCRIPTION "Configured budget in 0.1W." ::= { testGlobal 2 }
testPortTable OBJECT-TYPE SYNTAX SEQUENCE OF TestPortEntry MAX-ACCESS not-accessible STATUS current DESCRIPTION "x" ::= { testMIB 2 }
testPortEntry OBJECT-TYPE SYNTAX TestPortEntry MAX-ACCESS not-accessible STATUS current DESCRIPTION "x"
    INDEX { testPortIndex } ::= { testPortTable 1 }
TestPortEntry ::= SEQUENCE { testPortIndex INTEGER, testPortName DisplayString, testPortPower INTEGER, testPortState INTEGER, testPortPackets Counter32 }
testPortIndex OBJECT-TYPE SYNTAX INTEGER MAX-ACCESS read-only STATUS current DESCRIPTION "x" ::= { testPortEntry 1 }
testPortName OBJECT-TYPE SYNTAX DisplayString MAX-ACCESS read-only STATUS current DESCRIPTION "x" ::= { testPortEntry 2 }
testPortPower OBJECT-TYPE SYNTAX INTEGER MAX-ACCESS read-only STATUS current DESCRIPTION "Power in 0.1W." ::= { testPortEntry 3 }
testPortState OBJECT-TYPE SYNTAX INTEGER { off(0), on(2) } MAX-ACCESS read-only STATUS current DESCRIPTION "x" ::= { testPortEntry 4 }
testPortPackets OBJECT-TYPE SYNTAX Counter32 MAX-ACCESS read-only STATUS current DESCRIPTION "x" ::= { testPortEntry 5 }
END
"#;

    const BASE: &[u64] = &[1, 3, 6, 1, 4, 1, 64001];

    fn oid(rest: &[u64]) -> Vec<u64> {
        [BASE, rest].concat()
    }

    async fn rig() -> (FakeAgent, SnmpClient, MibIndex) {
        let agent = FakeAgent::spawn("public", "x", 1).await;
        agent.set(&oid(&[1, 1, 0]), FakeValue::Integer(62));
        agent.set(&oid(&[1, 2, 0]), FakeValue::Integer(620));
        for (port, name, power, state) in [(1u64, "Port eins", 62, 2), (2, "", 0, 0)] {
            agent.set(&oid(&[2, 1, 1, port]), FakeValue::Integer(port as i64));
            agent.set(&oid(&[2, 1, 2, port]), FakeValue::Text(name.to_owned()));
            agent.set(&oid(&[2, 1, 3, port]), FakeValue::Integer(power));
            agent.set(&oid(&[2, 1, 4, port]), FakeValue::Integer(state));
            agent.set(&oid(&[2, 1, 5, port]), FakeValue::Counter32(1_000));
        }
        let target = SnmpTarget {
            host: "127.0.0.1".to_owned(),
            port: agent.port,
            timeout: Duration::from_secs(2),
            retries: 0,
        };
        let client = SnmpClient::connect(
            &target,
            &Credentials::V2c {
                community: Secret::new("public"),
            },
        )
        .await
        .unwrap();
        let index = MibIndex::build(&[Source {
            origin: Origin::Imported(1),
            text: VENDOR,
        }]);
        (agent, client, index)
    }

    #[tokio::test]
    async fn a_generic_table_becomes_named_sensors_with_units_from_descriptions() {
        let (_agent, mut client, index) = rig().await;
        let table = index.by_name("testPortTable", None)[0];
        let groups = discover(&mut client, &index, &[Target::Object(table)])
            .await
            .unwrap();
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        assert_eq!(g.source, "TEST-POE-MIB::testPortTable");
        let find = |name: &str| {
            g.candidates
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| {
                    panic!(
                        "{name}: {:?}",
                        g.candidates.iter().map(|c| &c.name).collect::<Vec<_>>()
                    )
                })
        };
        let power = find("testPortPower Port eins");
        assert_eq!(crate::mib::dotted(&power.oid), "1.3.6.1.4.1.64001.2.1.3.1");
        assert_eq!(
            (power.scale, power.unit.as_deref(), power.value.as_str()),
            (0.1, Some("W"), "6,2 W")
        );
        assert!(power.selected, "read-only measurements are ticked");
        assert_eq!(find("testPortState Port eins").value, "on (2)");
        assert_eq!(
            find("testPortState Index 2").value,
            "off (0)",
            "rows without a name fall back to the index"
        );
        let packets = find("testPortPackets Port eins");
        assert_eq!(packets.mode, OidMode::Counter);
        assert!(packets.value.contains("Zählerstand"));
        assert!(
            g.candidates.iter().all(|c| c.object != "testPortName"),
            "text columns are no sensors"
        );
    }

    #[tokio::test]
    async fn a_scalar_group_ticks_measurements_but_not_settings() {
        let (_agent, mut client, index) = rig().await;
        let group = index.by_name("testGlobal", None)[0];
        let groups = discover(&mut client, &index, &[Target::Object(group)])
            .await
            .unwrap();
        let c = &groups[0].candidates;
        assert_eq!(c.len(), 2);
        assert_eq!(
            (c[0].name.as_str(), c[0].value.as_str(), c[0].selected),
            ("testConsumption", "6,2 W", true)
        );
        assert_eq!(crate::mib::dotted(&c[0].oid), "1.3.6.1.4.1.64001.1.1.0");
        assert!(!c[1].selected, "read-write values are settings");
    }

    #[tokio::test]
    async fn catalog_entries_without_loaded_mibs_or_data_are_skipped() {
        let (_agent, mut client, index) = rig().await;
        // None of the catalog tables exist on the fake agent; TP-Link MIBs are not loaded here.
        let groups = discover(&mut client, &index, &catalog_targets())
            .await
            .unwrap();
        assert!(groups.is_empty());
    }

    #[tokio::test]
    async fn an_unreachable_device_is_an_error() {
        let silent = FakeAgent::spawn("other", "x", 1).await;
        let index = MibIndex::build(&[Source {
            origin: Origin::Imported(1),
            text: VENDOR,
        }]);
        let target = SnmpTarget {
            host: "127.0.0.1".to_owned(),
            port: silent.port,
            timeout: Duration::from_millis(300),
            retries: 0,
        };
        let mut client = SnmpClient::connect(
            &target,
            &Credentials::V2c {
                community: Secret::new("public"),
            },
        )
        .await
        .unwrap();
        let table = index.by_name("testPortTable", None)[0];
        assert!(matches!(
            discover(&mut client, &index, &[Target::Object(table)]).await,
            Err(SnmpError::Timeout)
        ));
    }

    #[test]
    fn names_are_trimmed_to_the_limit() {
        assert_eq!(sensor_name("PoE-Leistung", "Port 1"), "PoE-Leistung Port 1");
        assert_eq!(sensor_name("Wert", ""), "Wert");
        assert_eq!(
            sensor_name(&"x".repeat(80), "y").chars().count(),
            MAX_NAME_CHARS
        );
    }
}
