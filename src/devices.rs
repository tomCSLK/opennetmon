//! Device and group domain types and input validation.

use std::net::IpAddr;

use crate::i18n::{Lang, Msg};

pub const MAX_NAME_LEN: usize = 64;
pub const MAX_HOST_LEN: usize = 253;
pub const MAX_GROUP_DEPTH: usize = 8;
pub const MIN_INTERVAL_SECS: u32 = 10;
pub const MAX_INTERVAL_SECS: u32 = 3600;
pub const MAX_TIMEOUT_SECS: u32 = 60;
pub const MAX_RETRIES: u32 = 5;
pub const DEFAULT_PORT: u16 = 161;
pub const DEFAULT_INTERVAL_SECS: u32 = 60;
pub const DEFAULT_TIMEOUT_SECS: u32 = 5;
pub const DEFAULT_RETRIES: u32 = 1;

/// Validates a group or device name (1 to 64 characters, no control characters).
pub fn validate_name(name: &str) -> Result<&str, Msg> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN || name.chars().any(char::is_control)
    {
        return Err(Msg::new("name-invalid"));
    }
    Ok(name)
}

/// Validates an IP address or a hostname (RFC 1123) and returns the normalized form.
/// Hostnames are lowercased; unspecified, multicast and broadcast addresses are rejected.
pub fn validate_host(host: &str) -> Result<String, Msg> {
    let invalid = || Msg::new("host-invalid");
    let host = host.trim();
    if host.is_empty() || host.len() > MAX_HOST_LEN {
        return Err(invalid());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        let bad = ip.is_unspecified()
            || ip.is_multicast()
            || matches!(ip, IpAddr::V4(v4) if v4.is_broadcast());
        return if bad {
            Err(invalid())
        } else {
            Ok(ip.to_string())
        };
    }
    let host = host.to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    // A numeric last label (e.g. "300.1.1.1") is a malformed IP address, not a hostname.
    let last_numeric = labels
        .last()
        .is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()));
    if labels.iter().all(|l| label_ok(l)) && !last_numeric {
        Ok(host)
    } else {
        Err(invalid())
    }
}

/// Polling settings of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Polling {
    pub port: u16,
    pub interval_secs: u32,
    pub timeout_secs: u32,
    pub retries: u32,
}

impl Default for Polling {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            interval_secs: DEFAULT_INTERVAL_SECS,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            retries: DEFAULT_RETRIES,
        }
    }
}

impl Polling {
    pub fn validate(&self) -> Result<(), Msg> {
        if self.port == 0 {
            return Err(Msg::new("polling-port-invalid"));
        }
        if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&self.interval_secs) {
            return Err(Msg::new("polling-interval-invalid"));
        }
        if !(1..=MAX_TIMEOUT_SECS).contains(&self.timeout_secs) {
            return Err(Msg::new("polling-timeout-invalid"));
        }
        if self.retries > MAX_RETRIES {
            return Err(Msg::new("polling-retries-invalid"));
        }
        // All attempts of one poll must fit into the interval, otherwise polls overlap.
        if self.timeout_secs.saturating_mul(self.retries + 1) > self.interval_secs {
            return Err(Msg::new("polling-attempts-too-long"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DeviceInput {
    pub name: String,
    pub host: String,
    pub group_id: Option<i64>,
    /// SNMP credentials; `None` = no SNMP, the device is only pinged.
    pub profile_id: Option<i64>,
    pub polling: Polling,
}

impl DeviceInput {
    /// Returns the input with trimmed name and normalized host, or a message.
    pub fn validated(mut self) -> Result<Self, Msg> {
        self.name = validate_name(&self.name)?.to_owned();
        self.host = validate_host(&self.host)?;
        self.polling.validate()?;
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub group_id: Option<i64>,
    /// SNMP credentials; `None` = no SNMP, the device is only pinged.
    pub profile_id: Option<i64>,
    pub profile_name: Option<String>,
    pub polling: Polling,
}

impl Device {
    pub fn has_snmp(&self) -> bool {
        self.profile_id.is_some()
    }

    /// Name of the SNMP profile for the UI ("kein SNMP" for ping-only devices).
    pub fn profile_label(&self, t: Lang) -> String {
        self.profile_name
            .clone()
            .unwrap_or_else(|| t.get("device-no-snmp"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
}

/// A group in depth-first order with its depth (1 = top level).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupNode {
    pub group: Group,
    pub depth: usize,
}

/// Orders groups depth-first; siblings keep the order of `groups` (display order from storage). Orphans and cycles cannot occur
/// (foreign key plus move checks); anything unreachable is simply left out.
pub fn flatten_groups(groups: &[Group]) -> Vec<GroupNode> {
    fn walk(groups: &[Group], parent: Option<i64>, depth: usize, out: &mut Vec<GroupNode>) {
        for group in groups.iter().filter(|g| g.parent_id == parent) {
            out.push(GroupNode {
                group: group.clone(),
                depth,
            });
            if depth < MAX_GROUP_DEPTH + 1 {
                walk(groups, Some(group.id), depth + 1, out);
            }
        }
    }
    let mut out = Vec::with_capacity(groups.len());
    walk(groups, None, 1, &mut out);
    out
}

/// The ids of `id` and all its descendants (each id once, so corrupted cyclic data terminates).
pub fn subtree_ids(groups: &[Group], id: i64) -> Vec<i64> {
    let mut ids = vec![id];
    let mut i = 0;
    while i < ids.len() {
        let current = ids[i];
        for group in groups.iter().filter(|g| g.parent_id == Some(current)) {
            if !ids.contains(&group.id) {
                ids.push(group.id);
            }
        }
        i += 1;
    }
    ids
}

/// Depth of a group (1 = top level); 0 for `None`.
pub fn depth_of(groups: &[Group], id: Option<i64>) -> usize {
    let mut depth = 0;
    let mut current = id;
    while let Some(group_id) = current {
        depth += 1;
        current = groups
            .iter()
            .find(|g| g.id == group_id)
            .and_then(|g| g.parent_id);
        if depth > groups.len() {
            break;
        }
    }
    depth
}

/// Height of the subtree below `id` (0 for a leaf).
pub fn subtree_height(groups: &[Group], id: i64) -> usize {
    fn walk(groups: &[Group], id: i64, budget: usize) -> usize {
        if budget == 0 {
            return 0; // corrupted cyclic data: stop instead of recursing forever
        }
        groups
            .iter()
            .filter(|g| g.parent_id == Some(id))
            .map(|g| 1 + walk(groups, g.id, budget - 1))
            .max()
            .unwrap_or(0)
    }
    walk(groups, id, groups.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_validated_and_normalized() {
        for ok in [
            "192.168.178.253",
            "fe80::1",
            "switch-keller.lan",
            "Router",
            "a.b-c.example.org",
            " localhost ",
        ] {
            assert!(validate_host(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            validate_host("Switch-KELLER.lan").unwrap(),
            "switch-keller.lan"
        );
        assert_eq!(validate_host("FE80:0:0:0:0:0:0:1").unwrap(), "fe80::1");
        for bad in [
            "",
            " ",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "::",
            "300.1.1.1",
            "1.2.3",
            "-a.lan",
            "a-.lan",
            "a..lan",
            "a b",
            "a_b.lan",
            "bücher.lan",
            "http://x",
            "a/b",
            &"a".repeat(64),
            &format!("{}.lan", "a".repeat(250)),
        ] {
            assert!(validate_host(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn polling_rules() {
        assert!(Polling::default().validate().is_ok());
        let p = |port, interval, timeout, retries| Polling {
            port,
            interval_secs: interval,
            timeout_secs: timeout,
            retries,
        };
        assert!(p(0, 60, 5, 1).validate().is_err());
        assert!(p(161, 9, 1, 0).validate().is_err());
        assert!(p(161, 3601, 5, 1).validate().is_err());
        assert!(p(161, 60, 0, 1).validate().is_err());
        assert!(p(161, 3600, 61, 1).validate().is_err());
        assert!(p(161, 3600, 5, 6).validate().is_err());
        // 5 s × (3 + 1) = 20 s > 10 s interval.
        assert!(p(161, 10, 5, 3).validate().is_err());
        assert!(p(161, 10, 5, 1).validate().is_ok());
    }

    #[test]
    fn device_input_is_normalized() {
        let input = DeviceInput {
            name: "  Switch Keller ".to_owned(),
            host: "SW-Keller.LAN".to_owned(),
            group_id: None,
            profile_id: Some(1),
            polling: Polling::default(),
        };
        let input = input.validated().unwrap();
        assert_eq!(
            (input.name.as_str(), input.host.as_str()),
            ("Switch Keller", "sw-keller.lan")
        );
        assert!(validate_name("a\tb").is_err());
        assert!(validate_name(&"x".repeat(65)).is_err());
    }

    fn g(id: i64, name: &str, parent: Option<i64>) -> Group {
        Group {
            id,
            name: name.to_owned(),
            parent_id: parent,
        }
    }

    #[test]
    fn groups_are_flattened_depth_first_in_the_given_order() {
        // As `storage::groups::list_groups` returns them: siblings already in display order.
        let groups = vec![
            g(4, "Anderes", None),
            g(1, "Zuhause", None),
            g(2, "büro", Some(1)),
            g(3, "Keller", Some(1)),
            g(5, "Rack", Some(3)),
        ];
        let names: Vec<(String, usize)> = flatten_groups(&groups)
            .into_iter()
            .map(|n| (n.group.name, n.depth))
            .collect();
        let expected: Vec<(String, usize)> = [
            ("Anderes", 1),
            ("Zuhause", 1),
            ("büro", 2),
            ("Keller", 2),
            ("Rack", 3),
        ]
        .into_iter()
        .map(|(n, d)| (n.to_owned(), d))
        .collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn subtree_depth_and_height() {
        let groups = vec![
            g(1, "a", None),
            g(2, "b", Some(1)),
            g(3, "c", Some(2)),
            g(4, "d", None),
        ];
        let mut ids = subtree_ids(&groups, 1);
        ids.sort_unstable();
        assert_eq!(ids, [1, 2, 3]);
        assert_eq!(subtree_ids(&groups, 4), [4]);
        assert_eq!(depth_of(&groups, None), 0);
        assert_eq!(depth_of(&groups, Some(1)), 1);
        assert_eq!(depth_of(&groups, Some(3)), 3);
        assert_eq!(subtree_height(&groups, 1), 2);
        assert_eq!(subtree_height(&groups, 3), 0);
    }

    #[test]
    fn corrupted_cycles_do_not_hang() {
        let groups = vec![g(1, "a", Some(2)), g(2, "b", Some(1))];
        assert!(depth_of(&groups, Some(1)) > 0);
        assert_eq!(subtree_ids(&groups, 1).len(), 2);
        assert!(subtree_height(&groups, 1) <= groups.len());
        assert!(flatten_groups(&groups).is_empty());
    }
}
