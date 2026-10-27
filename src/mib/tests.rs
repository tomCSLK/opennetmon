use super::*;

fn bundled() -> MibIndex {
    MibIndex::build(&[])
}

fn object<'a>(index: &'a MibIndex, name: &str) -> &'a MibObject {
    let found = index.by_name(name, None);
    assert!(!found.is_empty(), "{name} not found");
    found[0]
}

#[test]
fn all_bundled_modules_load_without_problems() {
    let index = bundled();
    let reports = index.reports();
    let names: Vec<&str> = reports.iter().map(|r| r.name.as_str()).collect();
    for expected in [
        "SNMPv2-SMI",
        "SNMPv2-TC",
        "SNMPv2-CONF",
        "SNMPv2-MIB",
        "RFC1155-SMI",
        "RFC1213-MIB",
        "IF-MIB",
        "IANAifType-MIB",
        "SNMP-FRAMEWORK-MIB",
        "HOST-RESOURCES-MIB",
        "INET-ADDRESS-MIB",
        "ENTITY-MIB",
        "ENTITY-SENSOR-MIB",
    ] {
        assert!(names.contains(&expected), "{expected} missing in {names:?}");
    }
    for r in reports {
        assert!(
            r.is_ok(),
            "{}: missing {:?}, unresolved {:?}",
            r.name,
            r.missing_modules,
            r.unresolved
        );
        assert!(r.warnings.is_empty(), "{}: {:?}", r.name, r.warnings);
    }
    assert!(index.len() > 500, "{}", index.len());
}

#[test]
fn well_known_objects_resolve_to_their_oids_and_types() {
    let index = bundled();
    let cases: [(&str, &str, ObjectKind, Option<BaseType>); 9] = [
        (
            "sysUpTime",
            "1.3.6.1.2.1.1.3",
            ObjectKind::Scalar,
            Some(BaseType::TimeTicks),
        ),
        (
            "sysDescr",
            "1.3.6.1.2.1.1.1",
            ObjectKind::Scalar,
            Some(BaseType::OctetString),
        ),
        ("ifTable", "1.3.6.1.2.1.2.2", ObjectKind::Table, None),
        ("ifEntry", "1.3.6.1.2.1.2.2.1", ObjectKind::Row, None),
        (
            "ifHCInOctets",
            "1.3.6.1.2.1.31.1.1.1.6",
            ObjectKind::Column,
            Some(BaseType::Counter64),
        ),
        (
            "ifOperStatus",
            "1.3.6.1.2.1.2.2.1.8",
            ObjectKind::Column,
            Some(BaseType::Integer),
        ),
        (
            "hrProcessorLoad",
            "1.3.6.1.2.1.25.3.3.1.2",
            ObjectKind::Column,
            Some(BaseType::Integer),
        ),
        (
            "entPhySensorValue",
            "1.3.6.1.2.1.99.1.1.1.4",
            ObjectKind::Column,
            Some(BaseType::Integer),
        ),
        ("enterprises", "1.3.6.1.4.1", ObjectKind::Node, None),
    ];
    for (name, oid, kind, base) in cases {
        let o = object(&index, name);
        assert_eq!(dotted(&o.oid), oid, "{name}");
        assert_eq!(o.kind, kind, "{name}");
        assert_eq!(o.base, base, "{name}");
    }
    assert_eq!(
        object(&index, "sysDescr").type_name.as_deref(),
        Some("DisplayString")
    );
    assert_eq!(object(&index, "ifEntry").index, ["ifIndex"]);
    assert_eq!(
        object(&index, "ifHCInOctets").index,
        ["ifIndex"],
        "AUGMENTS rows share the index of the base row"
    );
    assert_eq!(
        object(&index, "ifInOctets").index,
        ["ifIndex"],
        "columns know their row's index"
    );
    assert_eq!(object(&index, "ifXEntry").index, ["ifIndex"]);
    assert!(object(&index, "ifHCInOctets").is_pollable());
    assert!(
        !object(&index, "sysDescr").is_pollable(),
        "text is not a sensor value"
    );
    assert!(!object(&index, "ifTable").is_pollable());
}

#[test]
fn enumerations_come_from_the_object_or_its_textual_convention() {
    let index = bundled();
    let oper = object(&index, "ifOperStatus");
    assert_eq!(oper.enum_label(1), Some("up"));
    assert_eq!(oper.enum_label(7), Some("lowerLayerDown"));
    assert_eq!(oper.enum_label(99), None);
    // EntitySensorDataType is a textual convention in ENTITY-SENSOR-MIB.
    let sensor_type = object(&index, "entPhySensorType");
    assert_eq!(
        sensor_type.type_name.as_deref(),
        Some("EntitySensorDataType")
    );
    assert_eq!(sensor_type.enum_label(8), Some("celsius"));
    // TruthValue from SNMPv2-TC.
    let promiscuous = object(&index, "ifPromiscuousMode");
    assert_eq!(promiscuous.enum_label(1), Some("true"));
    assert_eq!(
        object(&index, "hrStorageAllocationUnits").units.as_deref(),
        Some("Bytes")
    );
    assert_eq!(
        object(&index, "ifHighSpeed").units,
        None,
        "IF-MIB names no units"
    );
}

#[test]
fn user_input_with_names_resolves_to_numeric_oids() {
    let index = bundled();
    let (oid, o) = index.resolve_input("sysUpTime.0").unwrap();
    assert_eq!(dotted(&oid), "1.3.6.1.2.1.1.3.0");
    assert_eq!(o.name, "sysUpTime");
    let (oid, _) = index.resolve_input(" IF-MIB::ifInOctets.3 ").unwrap();
    assert_eq!(dotted(&oid), "1.3.6.1.2.1.2.2.1.10.3");
    let (oid, _) = index.resolve_input("ifinoctets.3").unwrap();
    assert_eq!(
        dotted(&oid),
        "1.3.6.1.2.1.2.2.1.10.3",
        "names are case-insensitive; RFC1213-MIB and IF-MIB agree"
    );
    assert!(
        index
            .resolve_input("noSuchThing.0")
            .unwrap_err()
            .to_string()
            .contains("in keiner geladenen MIB")
    );
    assert!(
        index
            .resolve_input("sysUpTime.x")
            .unwrap_err()
            .to_string()
            .contains("keine Zahl")
    );
    assert!(
        index
            .resolve_input("HOST-RESOURCES-MIB::sysUpTime.0")
            .is_err(),
        "wrong module"
    );
}

#[test]
fn oids_are_described_by_their_most_specific_object() {
    let index = bundled();
    assert_eq!(
        index
            .name_of(&[1, 3, 6, 1, 2, 1, 2, 2, 1, 10, 3])
            .as_deref(),
        Some("ifInOctets.3")
    );
    assert_eq!(
        index.name_of(&[1, 3, 6, 1, 2, 1, 1, 3, 0]).as_deref(),
        Some("sysUpTime.0")
    );
    let (o, rest) = index.describe(&[1, 3, 6, 1, 4, 1, 11863, 6, 4]).unwrap();
    assert_eq!(
        (o.name.as_str(), rest),
        ("enterprises", &[11863u64, 6, 4][..])
    );
    assert!(index.describe(&[2, 99]).is_none() || index.describe(&[2, 99]).unwrap().0.oid == [2]);
}

#[test]
fn objects_below_an_oid_are_listed_in_order() {
    let index = bundled();
    let entry = object(&index, "ifEntry").oid.clone();
    let names: Vec<&str> = index
        .under(&entry)
        .iter()
        .map(|o| o.name.as_str())
        .collect();
    assert_eq!(names.first(), Some(&"ifIndex"));
    assert!(names.contains(&"ifOperStatus") && names.contains(&"ifOutQLen"));
    assert!(
        !names.contains(&"ifEntry"),
        "the prefix itself is not included"
    );
    assert!(index.under(&[1, 3, 6, 1, 2, 1, 2, 2, 1, 999]).is_empty());
}

#[test]
fn search_finds_pollable_objects_first() {
    let index = bundled();
    let hits = index.search("hrProcessor", 10);
    assert_eq!(
        hits[0].name,
        "hrProcessorLoad",
        "{:?}",
        hits.iter().map(|o| &o.name).collect::<Vec<_>>()
    );
    let by_description = index.search("bandwidth", 50);
    assert!(
        by_description.iter().any(|o| o.name == "ifSpeed"),
        "descriptions are searched"
    );
    assert!(index.search("   ", 10).is_empty());
    assert_eq!(index.search("if", 5).len(), 5);
}

#[test]
fn sensor_targets_from_names_and_numbers() {
    let index = bundled();
    let t = index.oid_target("sysUpTime").unwrap();
    assert_eq!(dotted(&t.oid), "1.3.6.1.2.1.1.3.0", "scalars get .0");
    assert_eq!(t.object.unwrap().name, "sysUpTime");
    let t = index.oid_target("hrProcessorLoad.196608").unwrap();
    assert_eq!(dotted(&t.oid), "1.3.6.1.2.1.25.3.3.1.2.196608");
    let t = index.oid_target("1.3.6.1.2.1.2.2.1.10.3").unwrap();
    assert_eq!(
        t.object.unwrap().name,
        "ifInOctets",
        "numeric OIDs are recognised too"
    );
    let t = index
        .oid_target(".1.3.6.1.4.1.11863.6.4.1.1.1.1.2.1")
        .unwrap();
    assert!(
        t.object.is_none(),
        "a vendor OID without its MIB is fine, just unnamed"
    );

    assert!(
        index
            .oid_target("ifTable")
            .unwrap_err()
            .to_string()
            .contains("Tabelle")
    );
    assert!(
        index
            .oid_target("ifEntry")
            .unwrap_err()
            .to_string()
            .contains("Tabellenzeile")
    );
    assert!(
        index
            .oid_target("system")
            .unwrap_err()
            .to_string()
            .contains("Knoten")
    );
    let missing_index = index.oid_target("ifInOctets").unwrap_err().to_string();
    assert!(
        missing_index.contains("Index anhängen") && missing_index.contains("ifIndex"),
        "{missing_index}"
    );
    assert!(index.oid_target("ifStackStatus.1.2").is_ok());
    assert!(index.oid_target("ifIndex.1").is_ok());
    assert!(
        index
            .oid_target("")
            .unwrap_err()
            .to_string()
            .contains("Bitte")
    );
    assert!(index.oid_target("1.3.x").is_err());
}

#[test]
fn standard_catalog_entries_resolve_with_the_bundled_mibs() {
    let index = bundled();
    for entry in catalog::CATALOG
        .iter()
        .filter(|e| !e.module.starts_with("TPLINK"))
    {
        assert!(
            !index.by_name(entry.object, Some(entry.module)).is_empty(),
            "{}",
            entry.object
        );
        for c in entry.columns {
            let o = index.by_name(c.object, Some(entry.module));
            assert!(!o.is_empty(), "{}", c.object);
            assert!(o[0].is_pollable(), "{} must be a readable number", c.object);
        }
    }
}

const VENDOR: &str = r#"
VENDOR-MIB DEFINITIONS ::= BEGIN
IMPORTS MODULE-IDENTITY, OBJECT-TYPE, enterprises FROM SNMPv2-SMI
        DisplayString FROM SNMPv2-TC
        vendorBase FROM VENDOR-BASE-MIB;
vendorMIB MODULE-IDENTITY LAST-UPDATED "202401010000Z" ORGANIZATION "V" CONTACT-INFO "x" DESCRIPTION "x"
    ::= { enterprises 64000 }
vendorCpu OBJECT-TYPE SYNTAX INTEGER (0..100) MAX-ACCESS read-only STATUS current
    DESCRIPTION "CPU load in percent." ::= { vendorMIB 1 }
vendorOther OBJECT-TYPE SYNTAX INTEGER MAX-ACCESS read-only STATUS current
    DESCRIPTION "Below a missing module." ::= { vendorBase 1 }
END
"#;

#[test]
fn uploaded_modules_resolve_against_bundled_ones_and_report_what_is_missing() {
    let index = MibIndex::build(&[Source {
        origin: Origin::Imported(7),
        text: VENDOR,
    }]);
    let (oid, cpu) = index.resolve_input("vendorCpu.0").unwrap();
    assert_eq!(dotted(&oid), "1.3.6.1.4.1.64000.1.0");
    assert_eq!(cpu.description.as_deref(), Some("CPU load in percent."));
    let report = index
        .reports()
        .iter()
        .find(|r| r.name == "VENDOR-MIB")
        .unwrap();
    assert_eq!(report.origin, Origin::Imported(7));
    assert_eq!(report.missing_modules, ["VENDOR-BASE-MIB"]);
    assert_eq!(report.unresolved, ["vendorOther"]);
    assert!(!report.is_ok());
}

#[test]
fn bundled_modules_win_over_uploaded_copies() {
    let fake = "RFC1213-MIB DEFINITIONS ::= BEGIN sysDescr OBJECT IDENTIFIER ::= { iso 9 } END";
    let index = MibIndex::build(&[Source {
        origin: Origin::Imported(1),
        text: fake,
    }]);
    assert_eq!(dotted(&object(&index, "sysDescr").oid), "1.3.6.1.2.1.1.1");
    let shadowed = index
        .reports()
        .iter()
        .find(|r| r.name == "RFC1213-MIB" && r.origin == Origin::Imported(1))
        .unwrap();
    assert!(shadowed.shadowed && !shadowed.is_ok());
}

/// The TP-Link MIBs are not in the repository (vendor files); this runs when they are present
/// locally in dev/data/mibs/tplink.
#[test]
fn tplink_mibs_resolve_when_available() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("dev/data/mibs/tplink");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("skipped: {} not present", dir.display());
        return;
    };
    let texts: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| String::from_utf8_lossy(&std::fs::read(e.path()).unwrap()).into_owned())
        .collect();
    let sources: Vec<Source<'_>> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| Source {
            origin: Origin::Imported(i as i64),
            text,
        })
        .collect();
    let index = MibIndex::build(&sources);
    let (oid, cpu) = index.resolve_input("tpSysMonitorCpu5Seconds.1").unwrap();
    assert_eq!(dotted(&oid), "1.3.6.1.4.1.11863.6.4.1.1.1.1.2.1");
    assert_eq!(cpu.kind, ObjectKind::Column);
    assert_eq!(cpu.module, "TPLINK-SYSMONITOR-MIB");
    let (oid, _) = index
        .resolve_input("tpSysMonitorMemoryUtilization.1")
        .unwrap();
    assert_eq!(dotted(&oid), "1.3.6.1.4.1.11863.6.4.1.2.1.1.2.1");
    for entry in catalog::CATALOG
        .iter()
        .filter(|e| e.module.starts_with("TPLINK"))
    {
        assert!(
            !index.by_name(entry.object, Some(entry.module)).is_empty(),
            "{}",
            entry.object
        );
        for c in entry.columns {
            let o = index.by_name(c.object, Some(entry.module));
            assert!(!o.is_empty() && o[0].is_pollable(), "{}", c.object);
        }
    }
    let problems: Vec<String> = index
        .reports()
        .iter()
        .filter(|r| matches!(r.origin, Origin::Imported(_)) && !r.shadowed && !r.is_ok())
        .map(|r| {
            format!(
                "{}: missing {:?} unresolved {:?}",
                r.name, r.missing_modules, r.unresolved
            )
        })
        .collect();
    eprintln!("TP-Link modules with problems: {problems:#?}");
}

#[test]
fn modules_belong_to_the_enterprise_of_their_objects_or_of_their_importers() {
    use super::vendors::Vendor;
    let tc = "ACME-TC DEFINITIONS ::= BEGIN IMPORTS TEXTUAL-CONVENTION FROM SNMPv2-TC;
        AcmeState ::= TEXTUAL-CONVENTION STATUS current DESCRIPTION \"x\" SYNTAX INTEGER { on(1), off(2) } END";
    let acme = "ACME-MIB DEFINITIONS ::= BEGIN IMPORTS OBJECT-TYPE, enterprises FROM SNMPv2-SMI AcmeState FROM ACME-TC;
        acme OBJECT IDENTIFIER ::= { enterprises 64000 }
        acmeState OBJECT-TYPE SYNTAX AcmeState MAX-ACCESS read-only STATUS current DESCRIPTION \"x\" ::= { acme 1 } END";
    let bridge = "MY-BRIDGE-MIB DEFINITIONS ::= BEGIN IMPORTS mib-2 FROM SNMPv2-SMI;
        myBridge OBJECT IDENTIFIER ::= { mib-2 9999 } END";
    let index = MibIndex::build(&[
        Source {
            origin: Origin::Imported(1),
            text: tc,
        },
        Source {
            origin: Origin::Imported(2),
            text: acme,
        },
        Source {
            origin: Origin::Imported(3),
            text: bridge,
        },
    ]);
    let vendor = |name: &str| {
        index
            .reports()
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .vendor
    };
    assert_eq!(vendor("ACME-MIB"), Some(Vendor::Enterprise(64000)));
    assert_eq!(
        vendor("ACME-TC"),
        Some(Vendor::Enterprise(64000)),
        "adopted by its importer"
    );
    assert_eq!(vendor("MY-BRIDGE-MIB"), Some(Vendor::Standard));
    assert_eq!(vendor("IF-MIB"), Some(Vendor::Standard));
}

/// `a0` = enterprises.99999 (seven arcs), each `a<i>` one arc below `a<i-1>`; `reversed` lists the
/// deepest definition first, so resolving it recurses all the way down.
fn chain(n: usize, reversed: bool) -> String {
    let mut defs: Vec<String> = (1..n)
        .map(|i| format!("a{i} OBJECT IDENTIFIER ::= {{ a{} 1 }}", i - 1))
        .collect();
    defs.insert(
        0,
        "a0 OBJECT IDENTIFIER ::= { enterprises 99999 }".to_owned(),
    );
    if reversed {
        defs.reverse();
    }
    format!(
        "CHAIN-MIB DEFINITIONS ::= BEGIN\nIMPORTS enterprises FROM SNMPv2-SMI;\n{}\nEND\n",
        defs.join("\n")
    )
}

#[test]
fn long_chains_of_definitions_stop_at_the_longest_oid() {
    // A file like this once took gigabytes (each OID as long as the chain) or overflowed the stack.
    for reversed in [false, true] {
        let text = chain(5000, reversed);
        let index = MibIndex::build(&[Source {
            origin: Origin::Imported(1),
            text: &text,
        }]);
        let report = index
            .reports()
            .iter()
            .find(|r| r.name == "CHAIN-MIB")
            .unwrap();
        // a0 … a121 have 7 to 128 arcs; deeper ones are no valid OIDs.
        assert_eq!(report.objects, 122, "reversed: {reversed}");
        assert_eq!(report.unresolved.len(), 5000 - 122);
        assert_eq!(object(&index, "a121").oid.len(), MAX_OID_ARCS);
        assert!(index.by_name("a122", None).is_empty());
    }
}
