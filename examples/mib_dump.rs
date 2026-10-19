// Lists every resolved MIB object as "name<TAB>oid" (bundled MIBs plus the files given as
// arguments) and prints module problems to stderr. Used by dev/mib-oracle.sh to compare with net-snmp.
fn main() {
    let texts: Vec<String> = std::env::args()
        .skip(1)
        .map(|p| match std::fs::read(&p) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) => {
                eprintln!("cannot read {p}: {e}");
                std::process::exit(2);
            }
        })
        .collect();
    let sources: Vec<opennetmon::mib::Source<'_>> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| opennetmon::mib::Source {
            origin: opennetmon::mib::Origin::Imported(i as i64),
            text,
        })
        .collect();
    let index = opennetmon::mib::MibIndex::build(&sources);
    for r in index.reports() {
        if !r.is_ok() || !r.warnings.is_empty() {
            eprintln!(
                "{}: shadowed={} missing={:?} unresolved={:?} warnings={:?}",
                r.name, r.shadowed, r.missing_modules, r.unresolved, r.warnings
            );
        }
    }
    for o in index.all() {
        println!("{}\t{}", o.name, opennetmon::mib::dotted(&o.oid));
    }
}
