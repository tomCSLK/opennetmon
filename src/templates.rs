//! Device templates: made from a configured device, applied to others.
//!
//! A template stores rules rather than sensor ids: ping/uptime, interface sensors (the same ports by
//! name, all up ports or all ports) and custom OID sensors with name, factor, unit, state and
//! thresholds. Applying a template only adds what a device does not monitor yet and skips values the
//! device does not deliver; it never changes or deletes existing sensors.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::devices::MAX_NAME_LEN;
use crate::discovery::{DiscoveredInterface, discover_interfaces};
use crate::error::Error;
use crate::i18n::{Lang, Msg};
use crate::sensors::snmp::{OidMode, numeric};
use crate::sensors::spec::{SensorSpec, format_oid, parse_oid};
use crate::snmp::{SYS_OBJECT_ID, SnmpClient, SnmpError, SnmpValue};
use crate::status::{Status, Thresholds, ValueState};
use crate::storage::sensors::{self as store, ChannelRecord, SensorRecord};

/// Upper bound of rules in one template.
pub const MAX_ITEMS: usize = 2_000;
const FORMAT_VERSION: u32 = 1;
/// OIDs per GET request when checking which values a device delivers.
const GET_BATCH: usize = 10;

/// Thresholds of one channel, by channel key (`in`, `out`, `value`, …).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn_above: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn_below: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down_above: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down_below: Option<f64>,
    /// Ratings of status values (`up`, `warning`, `down`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<StateRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateRule {
    pub value: i64,
    pub label: String,
    pub status: String,
}

impl StateRule {
    fn state(&self) -> Option<ValueState> {
        let status = Status::parse(&self.status)
            .filter(|s| matches!(s, Status::Up | Status::Warning | Status::Down))?;
        Some(ValueState {
            value: self.value,
            label: self.label.clone(),
            status,
        })
    }
}

impl Limits {
    pub fn thresholds(&self) -> Thresholds {
        Thresholds {
            warn_above: self.warn_above,
            warn_below: self.warn_below,
            down_above: self.down_above,
            down_below: self.down_below,
        }
    }

    pub fn value_states(&self) -> Vec<ValueState> {
        self.states.iter().filter_map(StateRule::state).collect()
    }
}

/// The thresholds of the channels that have any.
fn limits_of(channels: &[ChannelRecord]) -> Vec<Limits> {
    channels
        .iter()
        .filter(|c| c.thresholds != Thresholds::default() || !c.states.is_empty())
        .map(|c| Limits {
            key: c.key.clone(),
            warn_above: c.thresholds.warn_above,
            warn_below: c.thresholds.warn_below,
            down_above: c.thresholds.down_above,
            down_below: c.thresholds.down_below,
            states: c
                .states
                .iter()
                .map(|s| StateRule {
                    value: s.value,
                    label: s.label.clone(),
                    status: s.status.as_str().to_owned(),
                })
                .collect(),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BasicKind {
    Ping,
    Uptime,
}

impl BasicKind {
    fn spec(self) -> SensorSpec {
        match self {
            Self::Ping => SensorSpec::Ping,
            Self::Uptime => SensorSpec::Uptime,
        }
    }
}

/// Ping or uptime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Basic {
    pub kind: BasicKind,
    pub name: String,
    pub enabled: bool,
    #[serde(default)]
    pub limits: Vec<Limits>,
}

/// Which interfaces get a traffic or status sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortSelection {
    /// The ports listed in the rule, found by interface name.
    Same,
    /// Ports (physical or Wi-Fi) that are up when the template is applied.
    Up,
    /// All ports (physical or Wi-Fi), up or not.
    All,
}

/// One port of a [`PortSelection::Same`] rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Port {
    /// `ifName` (or `ifDescr`) as discovery reports it.
    pub interface: String,
    /// Sensor name, e.g. "Traffic gigabitEthernet 1/0/1" or a name the admin gave it.
    pub name: String,
    pub enabled: bool,
    #[serde(default)]
    pub limits: Vec<Limits>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortRule {
    pub selection: PortSelection,
    /// Only for [`PortSelection::Same`].
    #[serde(default)]
    pub ports: Vec<Port>,
    /// State and thresholds of new sensors for `Up` and `All`.
    pub enabled: bool,
    #[serde(default)]
    pub limits: Vec<Limits>,
}

/// A custom OID sensor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OidItem {
    pub oid: String,
    pub name: String,
    pub counter: bool,
    pub scale: f64,
    #[serde(default)]
    pub unit: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub limits: Vec<Limits>,
}

impl OidItem {
    fn spec(&self) -> Result<SensorSpec, Msg> {
        Ok(SensorSpec::Oid {
            oid: parse_oid(&self.oid)?,
            mode: if self.counter {
                OidMode::Counter
            } else {
                OidMode::Gauge
            },
            scale: self.scale,
            unit: self.unit.clone(),
        })
    }
}

/// What a template contains (stored as JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Body {
    pub version: u32,
    #[serde(default)]
    pub basics: Vec<Basic>,
    #[serde(default)]
    pub traffic: Option<PortRule>,
    #[serde(default)]
    pub status: Option<PortRule>,
    #[serde(default)]
    pub oids: Vec<OidItem>,
}

/// `„name“: message`
fn named(name: &str, message: Msg) -> Msg {
    Msg::new("template-named-error")
        .with("name", name)
        .with("message", message)
}

fn check_name(name: &str) -> Result<(), Msg> {
    crate::devices::validate_name(name)
        .map(|_| ())
        .map_err(|m| named(name, m))
}

fn check_limits(name: &str, limits: &[Limits]) -> Result<(), Msg> {
    limits.iter().try_for_each(|l| {
        if l.key.is_empty() || l.key.len() > 32 {
            return Err(named(name, Msg::new("template-channel-invalid")));
        }
        if l.value_states().len() != l.states.len()
            || l.states.iter().any(|s| s.label.chars().count() > 64)
        {
            return Err(named(name, Msg::new("template-states-invalid")));
        }
        l.thresholds().validate().map_err(|m| named(name, m))
    })
}

impl Body {
    pub fn is_empty(&self) -> bool {
        self.basics.is_empty()
            && self.traffic.is_none()
            && self.status.is_none()
            && self.oids.is_empty()
    }

    fn rules(&self) -> impl Iterator<Item = &PortRule> {
        self.traffic.iter().chain(self.status.iter())
    }

    pub fn validate(&self) -> Result<(), Msg> {
        if self.version != FORMAT_VERSION {
            return Err(Msg::new("template-version-unknown").with("version", self.version));
        }
        let ports: usize = self.rules().map(|r| r.ports.len()).sum();
        if self.basics.len() + ports + self.oids.len() > MAX_ITEMS {
            return Err(Msg::new("template-too-many").with("count", MAX_ITEMS));
        }
        for b in &self.basics {
            check_name(&b.name)?;
            check_limits(&b.name, &b.limits)?;
        }
        for rule in self.rules() {
            check_limits("Interfaces", &rule.limits)?;
            for p in &rule.ports {
                check_name(&p.name)?;
                if p.interface.trim().is_empty() || p.interface.chars().count() > MAX_NAME_LEN {
                    return Err(named(&p.name, Msg::new("template-interface-invalid")));
                }
                check_limits(&p.name, &p.limits)?;
            }
        }
        for o in &self.oids {
            check_name(&o.name)?;
            let spec = o.spec().map_err(|m| named(&o.name, m))?;
            spec.validate().map_err(|m| named(&o.name, m))?;
            check_limits(&o.name, &o.limits)?;
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, Error> {
        serde_json::to_string(self).map_err(|e| Error::Internal(format!("template to JSON: {e}")))
    }

    pub fn from_json(text: &str) -> Result<Self, Error> {
        let body: Self = serde_json::from_str(text)
            .map_err(|e| Error::Internal(format!("stored template is not valid: {e}")))?;
        body.validate()
            .map_err(|m| Error::Internal(format!("stored template is not valid: {m}")))?;
        Ok(body)
    }

    /// Short description, e.g. "Ping, Uptime, Traffic: aktive Ports, 4 eigene Werte".
    pub fn summary(&self, t: Lang) -> String {
        let mut parts: Vec<String> = Vec::new();
        for b in &self.basics {
            let label = match b.kind {
                BasicKind::Ping => "Ping",
                BasicKind::Uptime => "Uptime",
            };
            if !parts.iter().any(|p| p == label) {
                parts.push(label.to_owned());
            }
        }
        for (label, rule) in [("Traffic", &self.traffic), ("Status", &self.status)] {
            if let Some(rule) = rule {
                parts.push(format!("{label}: {}", selection_label(t, rule)));
            }
        }
        if !self.oids.is_empty() {
            parts.push(t.get1("template-custom-values", "count", self.oids.len()));
        }
        if parts.is_empty() {
            t.get("template-empty")
        } else {
            parts.join(", ")
        }
    }
}

pub fn selection_label(t: Lang, rule: &PortRule) -> String {
    match rule.selection {
        PortSelection::Same => t.get1("template-ports-same", "count", rule.ports.len()),
        PortSelection::Up => t.get("template-ports-up"),
        PortSelection::All => t.get("template-ports-all"),
    }
}

/// How the interface sensors of the source device go into a template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortChoice {
    Skip,
    Same,
    Up,
    All,
}

impl PortChoice {
    pub fn parse(text: &str) -> Self {
        match text {
            "same" => Self::Same,
            "up" => Self::Up,
            "all" => Self::All,
            _ => Self::Skip,
        }
    }
}

pub struct BuildOptions {
    /// Ping, uptime and OID sensors (by id) to take over.
    pub include: HashSet<i64>,
    pub traffic: PortChoice,
    pub status: PortChoice,
}

fn port_rule(
    sensors: &[&SensorRecord],
    index_of: fn(&SensorSpec) -> Option<u32>,
    choice: PortChoice,
    interfaces: Option<&[DiscoveredInterface]>,
) -> Result<Option<PortRule>, Msg> {
    let selection = match choice {
        PortChoice::Skip => return Ok(None),
        PortChoice::Same => PortSelection::Same,
        PortChoice::Up => PortSelection::Up,
        PortChoice::All => PortSelection::All,
    };
    let of_kind: Vec<&SensorRecord> = sensors
        .iter()
        .copied()
        .filter(|s| index_of(&s.spec).is_some())
        .collect();
    let mut ports = Vec::new();
    if selection == PortSelection::Same {
        let interfaces = interfaces.ok_or(Msg::new("template-interfaces-unread"))?;
        for s in &of_kind {
            let index = index_of(&s.spec);
            if let Some(found) = interfaces.iter().find(|i| Some(i.index) == index) {
                ports.push(Port {
                    interface: found.name.clone(),
                    name: s.name.clone(),
                    enabled: s.enabled,
                    limits: limits_of(&s.channels),
                });
            }
        }
        if ports.is_empty() {
            return Ok(None);
        }
    }
    // For up/all ports: active if any source sensor is, thresholds only if all sensors share them.
    let enabled = of_kind.is_empty() || of_kind.iter().any(|s| s.enabled);
    let limits = match of_kind.split_first() {
        Some((first, rest)) => {
            let shared = limits_of(&first.channels);
            if rest.iter().all(|s| limits_of(&s.channels) == shared) {
                shared
            } else {
                Vec::new()
            }
        }
        None => Vec::new(),
    };
    Ok(Some(PortRule {
        selection,
        ports,
        enabled,
        limits,
    }))
}

fn traffic_index(spec: &SensorSpec) -> Option<u32> {
    match spec {
        SensorSpec::InterfaceTraffic { if_index } => Some(*if_index),
        _ => None,
    }
}

fn status_index(spec: &SensorSpec) -> Option<u32> {
    match spec {
        SensorSpec::InterfaceStatus { if_index } => Some(*if_index),
        _ => None,
    }
}

/// A template from the sensors of a device. `interfaces` (from discovery) is needed for
/// [`PortChoice::Same`] to translate interface indexes into names.
pub fn build(
    sensors: &[SensorRecord],
    interfaces: Option<&[DiscoveredInterface]>,
    options: &BuildOptions,
) -> Result<Body, Msg> {
    let all: Vec<&SensorRecord> = sensors.iter().collect();
    let mut body = Body {
        version: FORMAT_VERSION,
        basics: Vec::new(),
        traffic: port_rule(&all, traffic_index, options.traffic, interfaces)?,
        status: port_rule(&all, status_index, options.status, interfaces)?,
        oids: Vec::new(),
    };
    for s in sensors.iter().filter(|s| options.include.contains(&s.id)) {
        let limits = limits_of(&s.channels);
        match &s.spec {
            SensorSpec::Ping | SensorSpec::Uptime => body.basics.push(Basic {
                kind: if s.spec == SensorSpec::Ping {
                    BasicKind::Ping
                } else {
                    BasicKind::Uptime
                },
                name: s.name.clone(),
                enabled: s.enabled,
                limits,
            }),
            SensorSpec::Oid {
                oid,
                mode,
                scale,
                unit,
            } => body.oids.push(OidItem {
                oid: format_oid(oid),
                name: s.name.clone(),
                counter: *mode == OidMode::Counter,
                scale: *scale,
                unit: unit.clone(),
                enabled: s.enabled,
                limits,
            }),
            // Interfaces follow the port rules; local sensors belong to one machine.
            SensorSpec::InterfaceTraffic { .. }
            | SensorSpec::InterfaceStatus { .. }
            | SensorSpec::Local(_) => {}
        }
    }
    if body.is_empty() {
        return Err(Msg::new("template-would-be-empty"));
    }
    body.validate()?;
    Ok(body)
}

/// A sensor to create when applying a template.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSensor {
    pub name: String,
    pub spec: SensorSpec,
    pub enabled: bool,
    pub limits: Vec<Limits>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub create: Vec<NewSensor>,
    /// Rules the device already fulfils.
    pub present: usize,
    /// Why something was left out (for the result page).
    pub skipped: Vec<Msg>,
}

/// What is known about the target device.
pub struct Facts<'a> {
    pub existing: &'a [SensorRecord],
    /// `None` when the interfaces were not read (no interface rules).
    pub interfaces: Option<&'a [DiscoveredInterface]>,
    /// OIDs the device answered with a number.
    pub answering: &'a HashSet<Vec<u64>>,
}

/// Sensor names in use on a device (case-insensitive like the database) and numbering of duplicates.
struct Names(HashSet<String>);

impl Names {
    fn take(&mut self, wanted: &str) -> String {
        let mut name = wanted.to_owned();
        let mut n = 2;
        while self.0.contains(&name.to_lowercase()) && n < 1000 {
            let suffix = format!(" ({n})");
            let base: String = wanted
                .chars()
                .take(MAX_NAME_LEN - suffix.chars().count())
                .collect();
            name = format!("{}{suffix}", base.trim_end());
            n += 1;
        }
        self.0.insert(name.to_lowercase());
        name
    }
}

fn sensor_name(prefix: &str, interface: &str) -> String {
    format!("{prefix} {interface}")
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_LEN)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// OIDs of the template that the device does not monitor yet (to be checked with GET).
pub fn oids_to_check(body: &Body, existing: &[SensorRecord]) -> Vec<Vec<u64>> {
    let present = existing_oids(existing);
    let mut seen = HashSet::new();
    body.oids
        .iter()
        .filter_map(|o| parse_oid(&o.oid).ok())
        .filter(|oid| !present.contains(oid) && seen.insert(oid.clone()))
        .collect()
}

fn existing_oids(existing: &[SensorRecord]) -> HashSet<Vec<u64>> {
    existing
        .iter()
        .filter_map(|s| match &s.spec {
            SensorSpec::Oid { oid, .. } => Some(oid.clone()),
            _ => None,
        })
        .collect()
}

/// Decides what applying `body` adds to a device. Pure, so every rule is testable without SNMP.
pub fn plan(body: &Body, facts: &Facts<'_>) -> Plan {
    let mut out = Plan::default();
    let mut names = Names(
        facts
            .existing
            .iter()
            .map(|s| s.name.to_lowercase())
            .collect(),
    );
    let mut kinds: HashSet<&'static str> = facts.existing.iter().map(|s| s.spec.kind()).collect();
    let mut interface_keys: HashSet<(bool, u32)> = facts
        .existing
        .iter()
        .filter_map(|s| {
            traffic_index(&s.spec)
                .map(|i| (true, i))
                .or_else(|| status_index(&s.spec).map(|i| (false, i)))
        })
        .collect();
    let mut oids = existing_oids(facts.existing);

    for b in &body.basics {
        let spec = b.kind.spec();
        if !kinds.insert(spec.kind()) {
            out.present += 1;
            continue;
        }
        out.create.push(NewSensor {
            name: names.take(&b.name),
            spec,
            enabled: b.enabled,
            limits: b.limits.clone(),
        });
    }

    for (traffic, rule) in [(true, &body.traffic), (false, &body.status)] {
        let Some(rule) = rule else { continue };
        let what = if traffic { "Traffic" } else { "Status" };
        let Some(interfaces) = facts.interfaces else {
            out.skipped
                .push(Msg::new("template-skipped-unread").with("what", what));
            continue;
        };
        let spec_of = |index| {
            if traffic {
                SensorSpec::InterfaceTraffic { if_index: index }
            } else {
                SensorSpec::InterfaceStatus { if_index: index }
            }
        };
        let mut wanted: Vec<(u32, String, bool, Vec<Limits>)> = Vec::new();
        match rule.selection {
            PortSelection::Same => {
                for p in &rule.ports {
                    match interfaces.iter().find(|i| i.name == p.interface) {
                        Some(i) => {
                            wanted.push((i.index, p.name.clone(), p.enabled, p.limits.clone()))
                        }
                        None => out.skipped.push(
                            Msg::new("template-skipped-interface")
                                .with("name", p.name.as_str())
                                .with("interface", p.interface.as_str()),
                        ),
                    }
                }
            }
            PortSelection::Up | PortSelection::All => {
                for i in interfaces
                    .iter()
                    .filter(|i| i.is_port() && (rule.selection == PortSelection::All || i.is_up()))
                {
                    wanted.push((
                        i.index,
                        sensor_name(what, &i.name),
                        rule.enabled,
                        rule.limits.clone(),
                    ));
                }
            }
        }
        for (index, name, enabled, limits) in wanted {
            if !interface_keys.insert((traffic, index)) {
                out.present += 1;
                continue;
            }
            out.create.push(NewSensor {
                name: names.take(&name),
                spec: spec_of(index),
                enabled,
                limits,
            });
        }
    }

    for o in &body.oids {
        let Ok(spec) = o.spec() else { continue };
        let SensorSpec::Oid { oid, .. } = &spec else {
            continue;
        };
        if oids.contains(oid) {
            out.present += 1;
            continue;
        }
        if !facts.answering.contains(oid) {
            out.skipped.push(
                Msg::new("template-skipped-oid")
                    .with("name", o.name.as_str())
                    .with("oid", o.oid.as_str()),
            );
            continue;
        }
        oids.insert(oid.clone());
        out.create.push(NewSensor {
            name: names.take(&o.name),
            spec,
            enabled: o.enabled,
            limits: o.limits.clone(),
        });
    }
    out
}

/// The template with this `sysObjectID`, if any.
pub fn detect<'a, T>(
    templates: &'a [T],
    object_id: &str,
    key: impl Fn(&T) -> Option<&str>,
) -> Option<&'a T> {
    templates.iter().find(|t| key(t) == Some(object_id))
}

/// The device's `sysObjectID` in dotted form, if it reports one.
pub async fn read_object_id(client: &mut SnmpClient) -> Result<Option<String>, SnmpError> {
    let values = client.get(&[SYS_OBJECT_ID]).await?;
    Ok(values.into_iter().find_map(|(_, v)| match v {
        SnmpValue::ObjectId(oid) if !oid.is_empty() => Some(oid),
        _ => None,
    }))
}

/// Which of `oids` the device answers with a number. Batches that fail as a whole (e.g. an SNMPv1
/// agent's noSuchName) are retried one OID at a time; only device-level errors abort.
async fn answering(
    client: &mut SnmpClient,
    oids: &[Vec<u64>],
) -> Result<HashSet<Vec<u64>>, SnmpError> {
    let device_level = |e: &SnmpError| {
        matches!(
            e,
            SnmpError::Timeout | SnmpError::Auth | SnmpError::Unreachable
        )
    };
    let mut found = HashSet::new();
    for batch in oids.chunks(GET_BATCH) {
        let refs: Vec<&[u64]> = batch.iter().map(Vec::as_slice).collect();
        let results = match client.get(&refs).await {
            Ok(values) => values,
            Err(e) if device_level(&e) => return Err(e),
            Err(_) => {
                let mut values = Vec::new();
                for oid in batch {
                    match client.get(&[oid.as_slice()]).await {
                        Ok(v) => values.extend(v),
                        Err(e) if device_level(&e) => return Err(e),
                        Err(_) => {}
                    }
                }
                values
            }
        };
        for (oid, value) in results {
            if numeric(&value).is_some()
                && let Ok(oid) = parse_oid(&oid)
            {
                found.insert(oid);
            }
        }
    }
    Ok(found)
}

#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub created: Vec<String>,
    pub present: usize,
    pub skipped: Vec<Msg>,
}

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("device: {0}")]
    Device(#[from] SnmpError),
    #[error(transparent)]
    Store(#[from] Error),
}

/// Applies a template to a device: reads what the rules need from the device, then creates the
/// missing sensors with their state and thresholds.
pub async fn apply(
    pool: &SqlitePool,
    client: &mut SnmpClient,
    device_id: i64,
    body: &Body,
) -> Result<Outcome, ApplyError> {
    let existing = store::list_sensors(pool, device_id).await?;
    let interfaces = if body.traffic.is_some() || body.status.is_some() {
        Some(discover_interfaces(client).await?)
    } else {
        None
    };
    let answering = answering(client, &oids_to_check(body, &existing)).await?;
    let plan = plan(
        body,
        &Facts {
            existing: &existing,
            interfaces: interfaces.as_deref(),
            answering: &answering,
        },
    );
    let mut created = Vec::with_capacity(plan.create.len());
    for new in plan.create {
        let id =
            store::create_sensor_with_state(pool, device_id, &new.name, &new.spec, new.enabled)
                .await?;
        if !new.limits.is_empty() {
            let channels = store::get_sensor(pool, id)
                .await?
                .map(|s| s.channels)
                .unwrap_or_default();
            let by_key: HashMap<&str, i64> =
                channels.iter().map(|c| (c.key.as_str(), c.id)).collect();
            let limits: Vec<(i64, Thresholds)> = new
                .limits
                .iter()
                .filter_map(|l| by_key.get(l.key.as_str()).map(|id| (*id, l.thresholds())))
                .collect();
            store::set_thresholds(pool, id, &limits).await?;
            for l in &new.limits {
                if let Some(channel) = by_key.get(l.key.as_str())
                    && !l.states.is_empty()
                {
                    store::set_value_states(pool, id, *channel, &l.value_states()).await?;
                }
            }
        }
        created.push(new.name);
    }
    Ok(Outcome {
        created,
        present: plan.present,
        skipped: plan.skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::Unit;
    use crate::snmp::SnmpTarget;
    use crate::snmp::testagent::{FakeAgent, FakeValue};
    use std::time::Duration;

    fn sensor(id: i64, name: &str, spec: SensorSpec, enabled: bool) -> SensorRecord {
        let channels = spec
            .channels()
            .iter()
            .enumerate()
            .map(|(i, c)| ChannelRecord {
                id: id * 10 + i64::try_from(i).unwrap(),
                key: c.key.to_owned(),
                unit: c.unit,
                thresholds: Thresholds::default(),
                states: Vec::new(),
            })
            .collect();
        SensorRecord {
            id,
            device_id: 1,
            name: name.to_owned(),
            spec,
            enabled,
            channels,
        }
    }

    fn iface(index: u32, name: &str, if_type: i64, up: bool) -> DiscoveredInterface {
        DiscoveredInterface {
            index,
            name: name.to_owned(),
            descr: name.to_owned(),
            alias: String::new(),
            if_type: Some(if_type),
            oper_status: Some(if up { 1 } else { 2 }),
            speed_mbps: Some(1000),
        }
    }

    const POE_1: &[u64] = &[1, 3, 6, 1, 4, 1, 11863, 6, 56, 1, 1, 2, 1, 7, 1];
    const POE_2: &[u64] = &[1, 3, 6, 1, 4, 1, 11863, 6, 56, 1, 1, 2, 1, 7, 2];

    fn poe(id: i64, oid: &[u64], name: &str) -> SensorRecord {
        sensor(
            id,
            name,
            SensorSpec::Oid {
                oid: oid.to_vec(),
                mode: OidMode::Gauge,
                scale: 0.1,
                unit: Some("W".to_owned()),
            },
            false,
        )
    }

    /// A configured switch: ping (with a limit), uptime, traffic on two ports, status on one, two PoE values.
    fn source() -> (Vec<SensorRecord>, Vec<DiscoveredInterface>) {
        let mut ping = sensor(1, "Ping", SensorSpec::Ping, true);
        ping.channels[0].thresholds.warn_above = Some(50.0);
        let sensors = vec![
            ping,
            sensor(2, "Uptime", SensorSpec::Uptime, true),
            sensor(
                3,
                "Uplink Keller",
                SensorSpec::InterfaceTraffic { if_index: 49153 },
                true,
            ),
            sensor(
                4,
                "Traffic gigabitEthernet 1/0/2",
                SensorSpec::InterfaceTraffic { if_index: 49154 },
                true,
            ),
            sensor(
                5,
                "Status gigabitEthernet 1/0/1",
                SensorSpec::InterfaceStatus { if_index: 49153 },
                true,
            ),
            poe(6, POE_1, "PoE-Leistung Port 1"),
            poe(7, POE_2, "PoE-Leistung Port 2"),
        ];
        let interfaces = vec![
            iface(49153, "gigabitEthernet 1/0/1", 6, true),
            iface(49154, "gigabitEthernet 1/0/2", 6, true),
            iface(49155, "gigabitEthernet 1/0/3", 6, false),
            iface(1, "Vlan-interface1", 136, true),
        ];
        (sensors, interfaces)
    }

    fn options(traffic: PortChoice, status: PortChoice) -> BuildOptions {
        BuildOptions {
            include: [1, 2, 6, 7].into(),
            traffic,
            status,
        }
    }

    #[test]
    fn a_template_keeps_names_states_thresholds_and_port_names() {
        let (sensors, interfaces) = source();
        let body = build(
            &sensors,
            Some(&interfaces),
            &options(PortChoice::Same, PortChoice::Up),
        )
        .unwrap();
        assert_eq!(body.basics.len(), 2);
        assert_eq!(body.basics[0].limits[0].warn_above, Some(50.0));
        let traffic = body.traffic.as_ref().unwrap();
        assert_eq!(traffic.selection, PortSelection::Same);
        assert_eq!(
            traffic
                .ports
                .iter()
                .map(|p| (p.interface.as_str(), p.name.as_str()))
                .collect::<Vec<_>>(),
            [
                ("gigabitEthernet 1/0/1", "Uplink Keller"),
                ("gigabitEthernet 1/0/2", "Traffic gigabitEthernet 1/0/2")
            ]
        );
        let status = body.status.as_ref().unwrap();
        assert_eq!(
            (status.selection, status.ports.len(), status.enabled),
            (PortSelection::Up, 0, true)
        );
        assert_eq!(body.oids.len(), 2);
        assert_eq!(body.oids[0].oid, "1.3.6.1.4.1.11863.6.56.1.1.2.1.7.1");
        assert!(!body.oids[0].enabled && !body.oids[0].counter);
        assert_eq!(
            body.summary(Lang::De),
            "Ping, Uptime, Traffic: 2 bestimmte Ports, Status: aktive Ports, 2 eigene Werte"
        );
        assert_eq!(
            Body::from_json(&body.to_json().unwrap()).unwrap(),
            body,
            "JSON round trip"
        );

        // Ratings of status values travel with the template.
        let (mut sensors, interfaces) = source();
        sensors[5].channels[0].states = vec![ValueState {
            value: 3,
            label: "overload".to_owned(),
            status: Status::Down,
        }];
        let body = build(
            &sensors,
            Some(&interfaces),
            &options(PortChoice::Skip, PortChoice::Skip),
        )
        .unwrap();
        assert_eq!(body.oids[0].limits[0].states[0].status, "down");
        assert_eq!(Body::from_json(&body.to_json().unwrap()).unwrap(), body);
        let answering: HashSet<Vec<u64>> = [POE_1.to_vec()].into();
        let planned = plan(&body, &facts(&[], None, &answering));
        let poe = planned
            .create
            .iter()
            .find(|n| n.name == "PoE-Leistung Port 1")
            .unwrap();
        assert_eq!(poe.limits[0].value_states()[0].status, Status::Down);
    }

    #[test]
    fn building_needs_content_and_interface_names_for_same_ports() {
        let (sensors, _) = source();
        let none = BuildOptions {
            include: HashSet::new(),
            traffic: PortChoice::Skip,
            status: PortChoice::Skip,
        };
        assert!(
            build(&sensors, None, &none)
                .unwrap_err()
                .to_string()
                .contains("leer")
        );
        assert!(
            build(&sensors, None, &options(PortChoice::Same, PortChoice::Skip))
                .unwrap_err()
                .to_string()
                .contains("nicht gelesen")
        );
        // Up/all ports work without reading the interfaces.
        assert!(build(&sensors, None, &options(PortChoice::All, PortChoice::Skip)).is_ok());
        assert!(Body::from_json("{\"version\":9}").is_err());
    }

    fn facts<'a>(
        existing: &'a [SensorRecord],
        interfaces: Option<&'a [DiscoveredInterface]>,
        answering: &'a HashSet<Vec<u64>>,
    ) -> Facts<'a> {
        Facts {
            existing,
            interfaces,
            answering,
        }
    }

    #[test]
    fn applying_adds_what_is_missing_and_explains_what_is_skipped() {
        let (sensors, interfaces) = source();
        let body = build(
            &sensors,
            Some(&interfaces),
            &options(PortChoice::Same, PortChoice::Up),
        )
        .unwrap();
        // The target: same model, port 2 renamed away, already has ping and a sensor called "Uptime".
        let target_ifaces = vec![
            iface(49153, "gigabitEthernet 1/0/1", 6, true),
            iface(49155, "gigabitEthernet 1/0/3", 6, true),
            iface(49156, "gigabitEthernet 1/0/4", 6, false),
        ];
        let existing = vec![
            sensor(20, "Erreichbarkeit", SensorSpec::Ping, true),
            poe(21, &[1, 3, 6, 1, 9], "uptime"),
        ];
        let answering: HashSet<Vec<u64>> = [POE_1.to_vec()].into();
        let plan = plan(&body, &facts(&existing, Some(&target_ifaces), &answering));
        let created: Vec<(&str, bool)> = plan
            .create
            .iter()
            .map(|n| (n.name.as_str(), n.enabled))
            .collect();
        assert_eq!(
            created,
            [
                ("Uptime (2)", true),
                ("Uplink Keller", true),
                ("Status gigabitEthernet 1/0/1", true),
                ("Status gigabitEthernet 1/0/3", true),
                ("PoE-Leistung Port 1", false),
            ]
        );
        assert_eq!(
            plan.create[1].spec,
            SensorSpec::InterfaceTraffic { if_index: 49153 }
        );
        assert_eq!(plan.present, 1, "ping exists");
        assert_eq!(plan.skipped.len(), 2, "{:?}", plan.skipped);
        assert!(
            plan.skipped[0]
                .to_string()
                .contains("kein Interface „gigabitEthernet 1/0/2“")
        );
        assert!(
            plan.skipped[1].to_string().contains("PoE-Leistung Port 2")
                && plan.skipped[1].to_string().contains("keinen Wert")
        );

        // Applying again changes nothing.
        let mut after = existing.clone();
        for (i, n) in plan.create.iter().enumerate() {
            after.push(sensor(
                100 + i64::try_from(i).unwrap(),
                &n.name,
                n.spec.clone(),
                n.enabled,
            ));
        }
        let again = super::plan(&body, &facts(&after, Some(&target_ifaces), &answering));
        assert!(again.create.is_empty());
        assert_eq!(again.present, 6);
        assert!(oids_to_check(&body, &after) == vec![POE_2.to_vec()]);
    }

    #[test]
    fn all_ports_include_down_ports_but_no_vlan_interfaces() {
        let (sensors, interfaces) = source();
        let body = build(
            &sensors,
            Some(&interfaces),
            &options(PortChoice::All, PortChoice::Skip),
        )
        .unwrap();
        let none = HashSet::new();
        let plan = plan(&body, &facts(&[], Some(&interfaces), &none));
        let traffic: Vec<&str> = plan
            .create
            .iter()
            .filter(|n| matches!(n.spec, SensorSpec::InterfaceTraffic { .. }))
            .map(|n| n.name.as_str())
            .collect();
        assert_eq!(
            traffic,
            [
                "Traffic gigabitEthernet 1/0/1",
                "Traffic gigabitEthernet 1/0/2",
                "Traffic gigabitEthernet 1/0/3"
            ]
        );
        let no_interfaces = super::plan(&body, &facts(&[], None, &none));
        assert!(no_interfaces.skipped[0].to_string().starts_with("Traffic:"));
    }

    #[test]
    fn thresholds_of_up_ports_are_kept_only_when_all_ports_share_them() {
        let (mut sensors, interfaces) = source();
        sensors[2].channels[0].thresholds.warn_above = Some(9e8);
        let body = build(
            &sensors,
            Some(&interfaces),
            &options(PortChoice::Up, PortChoice::Skip),
        )
        .unwrap();
        assert!(body.traffic.unwrap().limits.is_empty());
        sensors[3].channels[0].thresholds.warn_above = Some(9e8);
        let body = build(
            &sensors,
            Some(&interfaces),
            &options(PortChoice::Up, PortChoice::Skip),
        )
        .unwrap();
        assert_eq!(body.traffic.unwrap().limits[0].warn_above, Some(9e8));
        assert_eq!(Unit::BitsPerSecond, sensors[3].channels[0].unit);
    }

    #[test]
    fn detection_matches_the_exact_object_id() {
        let list = [(1, Some("1.3.6.1.4.1.11863.1.1.11")), (2, None)];
        assert_eq!(
            detect(&list, "1.3.6.1.4.1.11863.1.1.11", |t| t.1).map(|t| t.0),
            Some(1)
        );
        assert!(detect(&list, "1.3.6.1.4.1.11863.1.1", |t| t.1).is_none());
    }

    #[tokio::test]
    async fn the_device_is_asked_for_its_object_id_and_values() {
        let agent = FakeAgent::spawn("public", "x", 1).await;
        agent.set(
            SYS_OBJECT_ID,
            FakeValue::ObjectId(vec![1, 3, 6, 1, 4, 1, 11863, 1, 1, 11]),
        );
        agent.set(POE_1, FakeValue::Integer(62));
        agent.set(POE_2, FakeValue::Text("n/a".to_owned()));
        let target = SnmpTarget {
            host: "127.0.0.1".to_owned(),
            port: agent.port,
            timeout: Duration::from_millis(500),
            retries: 0,
        };
        let creds = crate::credentials::Credentials::V2c {
            community: crate::credentials::Secret::new("public"),
        };
        let mut client = SnmpClient::connect(&target, &creds).await.unwrap();
        assert_eq!(
            read_object_id(&mut client).await.unwrap().as_deref(),
            Some("1.3.6.1.4.1.11863.1.1.11")
        );
        let missing = vec![1, 3, 6, 1, 4, 1, 11863, 99, 0];
        let found = answering(&mut client, &[POE_1.to_vec(), POE_2.to_vec(), missing])
            .await
            .unwrap();
        assert_eq!(found, [POE_1.to_vec()].into());
    }
}
