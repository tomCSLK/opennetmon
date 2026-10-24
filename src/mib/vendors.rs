//! Who a MIB module belongs to: the private enterprise number (IANA, `1.3.6.1.4.1.<number>`) under which it defines
//! its objects, or the standard tree. Names for the enterprises one meets in networks; the full IANA list (over 60,000
//! entries) is not bundled, other numbers are shown as such.

/// The owner of a module, from the OIDs of its objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Vendor {
    /// A private enterprise number.
    Enterprise(u64),
    /// The standard trees (`mib-2`, `snmpV2`), e.g. an IETF MIB that is not bundled.
    Standard,
}

const ENTERPRISES: &[u64] = &[1, 3, 6, 1, 4, 1];
const STANDARD: &[&[u64]] = &[&[1, 3, 6, 1, 2], &[1, 3, 6, 1, 6]];

impl Vendor {
    /// The owner of one OID; `None` outside the enterprise and standard trees.
    pub fn of(oid: &[u64]) -> Option<Self> {
        match oid.strip_prefix(ENTERPRISES) {
            Some([number, ..]) => Some(Self::Enterprise(*number)),
            Some([]) => None,
            None if STANDARD.iter().any(|tree| oid.starts_with(tree)) => Some(Self::Standard),
            None => None,
        }
    }

    /// The vendor's name, if it is a known enterprise.
    pub fn name(self) -> Option<&'static str> {
        match self {
            Self::Enterprise(number) => enterprise_name(number),
            Self::Standard => None,
        }
    }
}

/// Known enterprise numbers, sorted by number.
const NAMES: &[(u64, &str)] = &[
    (2, "IBM"),
    (9, "Cisco"),
    (11, "HP"),
    (43, "3Com"),
    (171, "D-Link"),
    (232, "Compaq"),
    (253, "Xerox"),
    (311, "Microsoft"),
    (318, "APC"),
    (367, "Ricoh"),
    (476, "Liebert"),
    (534, "Eaton (Powerware)"),
    (641, "Lexmark"),
    (674, "Dell"),
    (789, "NetApp"),
    (890, "Zyxel"),
    (1248, "Epson"),
    (1347, "Kyocera"),
    (1602, "Canon"),
    (1916, "Extreme Networks"),
    (1991, "Foundry Networks"),
    (2011, "Huawei"),
    (2021, "UC Davis (Net-SNMP)"),
    (2435, "Brother"),
    (2620, "Check Point"),
    (2636, "Juniper Networks"),
    (3375, "F5 Networks"),
    (3808, "CyberPower"),
    (3955, "Linksys"),
    (4413, "Broadcom"),
    (4526, "Netgear"),
    (5951, "Citrix NetScaler"),
    (6486, "Alcatel-Lucent Enterprise"),
    (6574, "Synology"),
    (6876, "VMware"),
    (6889, "Avaya"),
    (7779, "Infoblox"),
    (8072, "Net-SNMP"),
    (8691, "Moxa"),
    (11863, "TP-Link"),
    (12356, "Fortinet"),
    (13742, "Raritan"),
    (14823, "Aruba Networks"),
    (14988, "MikroTik"),
    (17713, "Cambium Networks"),
    (24681, "QNAP"),
    (25461, "Palo Alto Networks"),
    (25506, "H3C"),
    (29671, "Cisco Meraki"),
    (30065, "Arista Networks"),
    (41112, "Ubiquiti"),
];

pub fn enterprise_name(number: u64) -> Option<&'static str> {
    NAMES
        .binary_search_by_key(&number, |(n, _)| *n)
        .ok()
        .map(|i| NAMES[i].1)
}

/// The owner of most of the given OIDs (ties: the smaller), `None` if none belongs to a tree above.
pub fn most_common<'a>(oids: impl IntoIterator<Item = &'a [u64]>) -> Option<Vendor> {
    let mut counts: std::collections::BTreeMap<Vendor, usize> = std::collections::BTreeMap::new();
    for vendor in oids.into_iter().filter_map(Vendor::of) {
        *counts.entry(vendor).or_default() += 1;
    }
    // `max_by_key` returns the last maximum; iterate in reverse so that the smaller key wins a tie.
    counts
        .into_iter()
        .rev()
        .max_by_key(|(_, count)| *count)
        .map(|(vendor, _)| vendor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_sorted_for_the_binary_search() {
        assert!(NAMES.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(enterprise_name(11863), Some("TP-Link"));
        assert_eq!(enterprise_name(9), Some("Cisco"));
        assert_eq!(enterprise_name(99_999_999), None);
    }

    #[test]
    fn owners_come_from_the_oid_trees() {
        assert_eq!(
            Vendor::of(&[1, 3, 6, 1, 4, 1, 11863, 6, 4]),
            Some(Vendor::Enterprise(11863))
        );
        assert_eq!(Vendor::of(&[1, 3, 6, 1, 4, 1]), None);
        assert_eq!(Vendor::of(&[1, 3, 6, 1, 2, 1, 17]), Some(Vendor::Standard));
        assert_eq!(Vendor::of(&[1, 3, 6, 1, 6, 3, 1]), Some(Vendor::Standard));
        assert_eq!(Vendor::of(&[1, 3, 6, 1, 3, 5]), None);
        let tplink = [1, 3, 6, 1, 4, 1, 11863, 1];
        let bridge = [1, 3, 6, 1, 2, 1, 17, 1];
        // A vendor MIB that also places a node in mib-2 still belongs to the vendor.
        assert_eq!(
            most_common([&tplink[..], &tplink[..], &bridge[..]]),
            Some(Vendor::Enterprise(11863))
        );
        assert_eq!(
            most_common([&bridge[..], &tplink[..]]),
            Some(Vendor::Enterprise(11863)),
            "a tie goes to the smaller key"
        );
        assert_eq!(most_common([]), None);
    }
}
