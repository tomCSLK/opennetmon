//! MIB support: parse MIB modules (bundled standard MIBs plus uploaded vendor MIBs), resolve every
//! object to its numeric OID and offer lookups in both directions (name → OID, OID → name) and a
//! search. The index is rebuilt in memory whenever the set of modules changes; nothing here talks
//! to devices.

pub mod catalog;
pub mod lexer;
pub mod parser;

use std::collections::{BTreeMap, HashMap, HashSet};

use parser::{DefKind, Module, OidDef, OidPart, Syntax, SyntaxKind, TypeDef};
use rust_embed::RustEmbed;

use crate::i18n::{Lang, Msg};

#[derive(RustEmbed)]
#[folder = "mibs/"]
struct Bundled;

/// Modules that only define SMI macros; importing from them never fails.
const VIRTUAL_MODULES: [&str; 1] = ["RFC-1212"];

/// Where a module comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Bundled,
    /// Uploaded file (id in `mib_files`).
    Imported(i64),
}

/// The SNMP base type of an object after resolving textual conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseType {
    Integer,
    Unsigned32,
    Gauge32,
    Counter32,
    Counter64,
    TimeTicks,
    OctetString,
    ObjectIdentifier,
    IpAddress,
    Bits,
    Opaque,
}

/// Whether `name` is an SMI base type (`INTEGER`, `Counter64`, …) rather than a textual convention.
pub fn is_builtin_type(name: &str) -> bool {
    BaseType::from_builtin(name).is_some()
}

impl BaseType {
    fn from_builtin(name: &str) -> Option<Self> {
        Some(match name {
            "INTEGER" | "Integer32" => Self::Integer,
            "Unsigned32" | "UInteger32" => Self::Unsigned32,
            "Gauge32" | "Gauge" => Self::Gauge32,
            "Counter32" | "Counter" => Self::Counter32,
            "Counter64" => Self::Counter64,
            "TimeTicks" => Self::TimeTicks,
            "OCTET STRING" => Self::OctetString,
            "OBJECT IDENTIFIER" => Self::ObjectIdentifier,
            "IpAddress" | "NetworkAddress" => Self::IpAddress,
            "BITS" => Self::Bits,
            "Opaque" => Self::Opaque,
            _ => return None,
        })
    }

    /// Name of the type; the technical names are the same in every language.
    pub fn label(self, t: Lang) -> String {
        let name = match self {
            Self::OctetString => return t.get("mib-type-octet-string"),
            Self::IpAddress => return t.get("mib-type-ip-address"),
            Self::Integer => "Integer",
            Self::Unsigned32 => "Unsigned32",
            Self::Gauge32 => "Gauge32",
            Self::Counter32 => "Counter32",
            Self::Counter64 => "Counter64",
            Self::TimeTicks => "TimeTicks",
            Self::ObjectIdentifier => "OID",
            Self::Bits => "Bits",
            Self::Opaque => "Opaque",
        };
        name.to_owned()
    }

    /// Counters are monitored as a rate per second, everything numeric else as a value.
    pub fn is_counter(self) -> bool {
        matches!(self, Self::Counter32 | Self::Counter64)
    }

    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            Self::Integer
                | Self::Unsigned32
                | Self::Gauge32
                | Self::Counter32
                | Self::Counter64
                | Self::TimeTicks
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// OBJECT IDENTIFIER, identities, groups, notifications, …
    Node,
    Scalar,
    Table,
    Row,
    Column,
}

impl ObjectKind {
    pub fn label_id(self) -> &'static str {
        match self {
            Self::Node => "mib-kind-node",
            Self::Scalar => "mib-kind-scalar",
            Self::Table => "mib-kind-table",
            Self::Row => "mib-kind-row",
            Self::Column => "mib-kind-column",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MibObject {
    pub module: String,
    pub name: String,
    pub oid: Vec<u64>,
    pub kind: ObjectKind,
    pub base: Option<BaseType>,
    /// The type as written, e.g. `DisplayString`, `Integer32`, `TruthValue`.
    pub type_name: Option<String>,
    pub enums: Vec<(String, i64)>,
    pub units: Option<String>,
    pub access: Option<String>,
    pub description: Option<String>,
    /// Index columns of a row (for columns: of their row).
    pub index: Vec<String>,
}

impl MibObject {
    /// Name of an enumeration value, e.g. `up` for 1.
    pub fn enum_label(&self, value: i64) -> Option<&str> {
        self.enums
            .iter()
            .find(|(_, n)| *n == value)
            .map(|(name, _)| name.as_str())
    }

    /// Can be polled as a sensor value (readable and numeric).
    pub fn is_pollable(&self) -> bool {
        matches!(self.kind, ObjectKind::Scalar | ObjectKind::Column)
            && self.base.is_some_and(BaseType::is_numeric)
            && self
                .access
                .as_deref()
                .is_some_and(|a| a.starts_with("read"))
    }
}

/// Result of loading one module.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleReport {
    pub name: String,
    pub origin: Origin,
    pub objects: usize,
    /// Imported modules that are not loaded.
    pub missing_modules: Vec<String>,
    /// Definitions whose OID could not be resolved.
    pub unresolved: Vec<String>,
    pub warnings: Vec<String>,
    /// Skipped because a module of the same name is already loaded (bundled modules win).
    pub shadowed: bool,
}

impl ModuleReport {
    pub fn is_ok(&self) -> bool {
        self.missing_modules.is_empty() && self.unresolved.is_empty() && !self.shadowed
    }
}

/// A source text to load.
pub struct Source<'a> {
    pub origin: Origin,
    pub text: &'a str,
}

#[derive(Debug, Default)]
pub struct MibIndex {
    objects: Vec<MibObject>,
    by_oid: BTreeMap<Vec<u64>, usize>,
    by_name: HashMap<String, Vec<usize>>,
    reports: Vec<ModuleReport>,
}

/// The index of the bundled MIBs alone, built once per process.
pub fn bundled_index() -> std::sync::Arc<MibIndex> {
    static BUNDLED: std::sync::OnceLock<std::sync::Arc<MibIndex>> = std::sync::OnceLock::new();
    BUNDLED
        .get_or_init(|| std::sync::Arc::new(MibIndex::build(&[])))
        .clone()
}

/// The texts of the bundled standard MIBs.
pub fn bundled_texts() -> Vec<String> {
    let mut names: Vec<_> = Bundled::iter().filter(|n| n.ends_with(".txt")).collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| Bundled::get(&name))
        .map(|file| String::from_utf8_lossy(&file.data).into_owned())
        .collect()
}

struct Loaded {
    module: Module,
    origin: Origin,
}

struct Resolver<'a> {
    modules: HashMap<&'a str, &'a Loaded>,
    /// Definitions per module by name.
    defs: HashMap<&'a str, HashMap<&'a str, &'a OidDef>>,
    types: HashMap<&'a str, HashMap<&'a str, &'a TypeDef>>,
    /// Global fallback when an import cannot be followed: name → (module, def).
    global_defs: HashMap<&'a str, Vec<(&'a str, &'a OidDef)>>,
    memo: HashMap<(String, String), Option<Vec<u64>>>,
    in_progress: HashSet<(String, String)>,
}

impl<'a> Resolver<'a> {
    fn new(loaded: &'a [Loaded]) -> Self {
        let mut r = Resolver {
            modules: HashMap::new(),
            defs: HashMap::new(),
            types: HashMap::new(),
            global_defs: HashMap::new(),
            memo: HashMap::new(),
            in_progress: HashSet::new(),
        };
        for l in loaded {
            let name = l.module.name.as_str();
            r.modules.insert(name, l);
            let defs = r.defs.entry(name).or_default();
            for d in &l.module.defs {
                defs.entry(d.name.as_str()).or_insert(d);
                r.global_defs
                    .entry(d.name.as_str())
                    .or_default()
                    .push((name, d));
            }
            let types = r.types.entry(name).or_default();
            for t in &l.module.types {
                types.entry(t.name.as_str()).or_insert(t);
            }
        }
        r
    }

    /// The module a name visible in `module` comes from (itself or an import).
    fn source_module(&self, module: &'a str, name: &str) -> Option<&'a str> {
        let l = self.modules.get(module)?;
        if self.defs.get(module).is_some_and(|d| d.contains_key(name))
            || self.types.get(module).is_some_and(|t| t.contains_key(name))
        {
            return Some(module);
        }
        l.module
            .imports
            .iter()
            .find(|(_, names)| names.iter().any(|n| n == name))
            .and_then(|(from, _)| {
                self.modules
                    .get(from.as_str())
                    .map(|m| m.module.name.as_str())
            })
    }

    fn find_def(&self, module: &'a str, name: &str, depth: usize) -> Option<(&'a str, &'a OidDef)> {
        if depth > 8 {
            return None;
        }
        if let Some(d) = self.defs.get(module).and_then(|d| d.get(name)) {
            return Some((module, d));
        }
        if let Some(from) = self.source_module(module, name)
            && from != module
            && let Some(found) = self.find_def(from, name, depth + 1)
        {
            return Some(found);
        }
        // Tolerance: an import from a module that is not loaded (or a missing import) still
        // resolves if exactly one loaded module defines the name.
        match self.global_defs.get(name).map(Vec::as_slice) {
            Some([only]) => Some(*only),
            Some(many) if !many.is_empty() => many
                .iter()
                .find(|(m, _)| m.starts_with("SNMPv2-") || *m == "RFC1155-SMI")
                .or(many.first())
                .copied(),
            _ => None,
        }
    }

    fn resolve_name(&mut self, module: &'a str, name: &str) -> Option<Vec<u64>> {
        match name {
            "ccitt" => return Some(vec![0]),
            "iso" => return Some(vec![1]),
            "joint-iso-ccitt" => return Some(vec![2]),
            _ => {}
        }
        let (def_module, def) = self.find_def(module, name, 0)?;
        self.resolve_def(def_module, def)
    }

    fn resolve_def(&mut self, module: &'a str, def: &'a OidDef) -> Option<Vec<u64>> {
        let key = (module.to_owned(), def.name.clone());
        if let Some(done) = self.memo.get(&key) {
            return done.clone();
        }
        if !self.in_progress.insert(key.clone()) {
            return None; // cycle
        }
        let mut oid: Option<Vec<u64>> = Some(Vec::new());
        for (i, part) in def.value.iter().enumerate() {
            let Some(current) = oid.as_mut() else { break };
            match part {
                OidPart::Number(n) => current.push(*n),
                OidPart::Named(_, n) => current.push(*n),
                OidPart::Name(name) if i == 0 => {
                    oid = self.resolve_name(module, name);
                }
                // A bare name later in the value is not valid SMI; give up on this definition.
                OidPart::Name(_) => oid = None,
            }
        }
        let oid = oid.filter(|o| !o.is_empty());
        self.in_progress.remove(&key);
        self.memo.insert(key, oid.clone());
        oid
    }

    /// Base type, type name, enums and display hint of a syntax, following textual conventions.
    fn syntax_info(
        &self,
        module: &'a str,
        syntax: &Syntax,
        depth: usize,
    ) -> (Option<BaseType>, Vec<(String, i64)>) {
        let SyntaxKind::Named(name) = &syntax.kind else {
            return (None, Vec::new());
        };
        if let Some(base) = BaseType::from_builtin(name) {
            return (Some(base), syntax.enums.clone());
        }
        if depth > 8 {
            return (None, syntax.enums.clone());
        }
        let type_def = self
            .source_module(module, name)
            .and_then(|m| {
                self.types
                    .get(m)
                    .and_then(|t| t.get(name.as_str()))
                    .map(|t| (m, *t))
            })
            .or_else(|| {
                // Not imported (or from a missing module): any loaded module defining it.
                self.types
                    .iter()
                    .find_map(|(m, t)| t.get(name.as_str()).map(|t| (*m, *t)))
            });
        match type_def {
            Some((m, t)) => {
                let (base, enums) = self.syntax_info(m, &t.syntax, depth + 1);
                // A refinement on the object (`TC { a(1) }`) overrides the TC's enumeration.
                let enums = if syntax.enums.is_empty() {
                    enums
                } else {
                    syntax.enums.clone()
                };
                (base, enums)
            }
            None => (None, syntax.enums.clone()),
        }
    }
}

/// Index columns of a row; a row that `AUGMENTS` another one shares that row's index.
fn row_index(resolver: &Resolver<'_>, module: &str, row: &OidDef) -> Vec<String> {
    let mut current = (module.to_owned(), row);
    for _ in 0..8 {
        let DefKind::ObjectType(ot) = &current.1.kind else {
            return Vec::new();
        };
        if !ot.index.is_empty() {
            return ot.index.clone();
        }
        let Some(base) = &ot.augments else {
            return Vec::new();
        };
        match resolver.find_def(module_static(resolver, &current.0), base, 0) {
            Some((m, d)) => current = (m.to_owned(), d),
            None => return Vec::new(),
        }
    }
    Vec::new()
}

fn is_row(def: &OidDef) -> bool {
    matches!(&def.kind, DefKind::ObjectType(ot) if !ot.index.is_empty() || ot.augments.is_some())
}

impl MibIndex {
    /// Builds the index from the bundled MIBs plus `extra` (uploaded files, oldest first).
    pub fn build(extra: &[Source<'_>]) -> Self {
        let bundled = bundled_texts();
        let mut sources: Vec<Source<'_>> = bundled
            .iter()
            .map(|text| Source {
                origin: Origin::Bundled,
                text,
            })
            .collect();
        sources.extend(extra.iter().map(|s| Source {
            origin: s.origin.clone(),
            text: s.text,
        }));
        Self::from_sources(&sources)
    }

    fn from_sources(sources: &[Source<'_>]) -> Self {
        let mut index = MibIndex::default();
        let mut loaded: Vec<Loaded> = Vec::new();
        for source in sources {
            for module in parser::parse(source.text) {
                match loaded.iter().position(|l| l.module.name == module.name) {
                    // Bundled modules win over uploaded copies (e.g. a vendor's RFC1213-MIB).
                    Some(_) if source.origin != Origin::Bundled => {
                        index.reports.push(ModuleReport {
                            name: module.name.clone(),
                            origin: source.origin.clone(),
                            objects: 0,
                            missing_modules: Vec::new(),
                            unresolved: Vec::new(),
                            warnings: Vec::new(),
                            shadowed: true,
                        });
                    }
                    Some(pos) => {
                        loaded[pos] = Loaded {
                            module,
                            origin: source.origin.clone(),
                        }
                    }
                    None => loaded.push(Loaded {
                        module,
                        origin: source.origin.clone(),
                    }),
                }
            }
        }

        let mut resolver = Resolver::new(&loaded);
        let names: HashSet<&str> = loaded.iter().map(|l| l.module.name.as_str()).collect();
        for l in &loaded {
            let module = l.module.name.as_str();
            let mut report = ModuleReport {
                name: module.to_owned(),
                origin: l.origin.clone(),
                objects: 0,
                missing_modules: l
                    .module
                    .imports
                    .iter()
                    .map(|(from, _)| from.clone())
                    .filter(|from| {
                        !names.contains(from.as_str()) && !VIRTUAL_MODULES.contains(&from.as_str())
                    })
                    .collect(),
                unresolved: Vec::new(),
                warnings: l.module.warnings.clone(),
                shadowed: false,
            };
            report.missing_modules.dedup();
            for def in &l.module.defs {
                let Some(oid) = resolver.resolve_def(module, def) else {
                    report.unresolved.push(def.name.clone());
                    continue;
                };
                let object = Self::object(&resolver, l, module, def, oid);
                report.objects += 1;
                index.insert(object);
            }
            index.reports.push(report);
        }
        index
    }

    fn object(
        resolver: &Resolver<'_>,
        l: &Loaded,
        module: &str,
        def: &OidDef,
        oid: Vec<u64>,
    ) -> MibObject {
        let mut object = MibObject {
            module: module.to_owned(),
            name: def.name.clone(),
            oid,
            kind: ObjectKind::Node,
            base: None,
            type_name: None,
            enums: Vec::new(),
            units: None,
            access: None,
            description: None,
            index: Vec::new(),
        };
        if let DefKind::ObjectType(ot) = &def.kind {
            object.units = ot.units.clone();
            object.access = ot.access.clone();
            object.description = ot.description.clone();
            if let Some(syntax) = &ot.syntax {
                if let SyntaxKind::Named(name) = &syntax.kind {
                    object.type_name = Some(name.clone());
                }
                let module_ref: &str = l.module.name.as_str();
                let (base, enums) = resolver.syntax_info(
                    resolver
                        .modules
                        .get(module_ref)
                        .map_or(module_ref, |m| m.module.name.as_str()),
                    syntax,
                    0,
                );
                object.base = base;
                object.enums = enums;
            }
            object.kind = if matches!(
                ot.syntax.as_ref().map(|s| &s.kind),
                Some(SyntaxKind::SequenceOf(_))
            ) {
                ObjectKind::Table
            } else if is_row(def) {
                object.index = row_index(resolver, module_static(resolver, module), def);
                ObjectKind::Row
            } else {
                // A column if its parent (`{ xEntry n }`) is a row.
                let parent_row = match def.value.first() {
                    Some(OidPart::Name(parent)) => resolver
                        .find_def(module_static(resolver, module), parent, 0)
                        .filter(|(_, d)| is_row(d)),
                    _ => None,
                };
                match parent_row {
                    Some((row_module, row)) => {
                        object.index = row_index(resolver, row_module, row);
                        ObjectKind::Column
                    }
                    None => ObjectKind::Scalar,
                }
            };
        }
        object
    }

    fn insert(&mut self, object: MibObject) {
        let id = self.objects.len();
        // The first definition of an OID wins (bundled before uploaded, SMIv2 before SMIv1 by file order).
        self.by_oid.entry(object.oid.clone()).or_insert(id);
        self.by_name
            .entry(object.name.to_lowercase())
            .or_default()
            .push(id);
        self.objects.push(object);
    }

    pub fn all(&self) -> &[MibObject] {
        &self.objects
    }

    pub fn reports(&self) -> &[ModuleReport] {
        &self.reports
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// All objects strictly below `prefix`, in OID order.
    pub fn under(&self, prefix: &[u64]) -> Vec<&MibObject> {
        self.by_oid
            .range(prefix.to_vec()..)
            .skip_while(|(oid, _)| oid.as_slice() == prefix)
            .take_while(|(oid, _)| oid.starts_with(prefix))
            .map(|(_, &i)| &self.objects[i])
            .collect()
    }

    pub fn get_oid(&self, oid: &[u64]) -> Option<&MibObject> {
        self.by_oid.get(oid).map(|&i| &self.objects[i])
    }

    /// The most specific known object for `oid` and the remaining arcs (the instance, e.g. `.0`).
    pub fn describe<'s, 'o>(&'s self, oid: &'o [u64]) -> Option<(&'s MibObject, &'o [u64])> {
        (1..=oid.len())
            .rev()
            .find_map(|n| self.get_oid(&oid[..n]).map(|o| (o, &oid[n..])))
    }

    /// `name.1.2` with the object name for `oid`, if known.
    pub fn name_of(&self, oid: &[u64]) -> Option<String> {
        let (object, rest) = self.describe(oid)?;
        let mut s = object.name.clone();
        for arc in rest {
            s.push('.');
            s.push_str(&arc.to_string());
        }
        Some(s)
    }

    /// Objects named `name` (case-insensitive), optionally only in `module`.
    pub fn by_name(&self, name: &str, module: Option<&str>) -> Vec<&MibObject> {
        self.by_name
            .get(&name.to_lowercase())
            .map(|ids| {
                ids.iter()
                    .map(|&i| &self.objects[i])
                    .filter(|o| module.is_none_or(|m| o.module.eq_ignore_ascii_case(m)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Resolves user input like `ifInOctets.3`, `IF-MIB::ifInOctets.3` or `sysUpTime.0` to a numeric
    /// OID and the object it refers to. Numeric input is left to the caller.
    pub fn resolve_input(&self, input: &str) -> Result<(Vec<u64>, &MibObject), Msg> {
        let input = input.trim().trim_start_matches('.');
        let (module, rest) = match input.split_once("::") {
            Some((m, r)) => (Some(m.trim()), r.trim()),
            None => (None, input),
        };
        let mut parts = rest.split('.');
        let name = parts.next().unwrap_or_default();
        let suffix: Vec<u64> = parts
            .map(|p| {
                p.parse::<u64>()
                    .map_err(|_| Msg::new("mib-instance-not-number").with("part", p))
            })
            .collect::<Result<_, _>>()?;
        let candidates = self.by_name(name, module);
        let mut distinct: Vec<&MibObject> = Vec::new();
        for c in candidates {
            if !distinct.iter().any(|d| d.oid == c.oid) {
                distinct.push(c);
            }
        }
        match distinct.as_slice() {
            [] => Err(Msg::new("mib-name-unknown").with("name", name)),
            [object] => {
                let mut oid = object.oid.clone();
                oid.extend(suffix);
                Ok((oid, object))
            }
            many => Err(Msg::new("mib-name-ambiguous")
                .with("name", name)
                .with(
                    "modules",
                    many.iter()
                        .map(|o| o.module.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                )
                .with("module", many[0].module.as_str())),
        }
    }

    /// Case-insensitive search over names (exact, prefix, substring) and descriptions;
    /// pollable objects first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&MibObject> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(u8, &MibObject)> = self
            .objects
            .iter()
            .filter_map(|o| {
                let name = o.name.to_lowercase();
                let rank = if name == q {
                    0
                } else if name.starts_with(&q) {
                    1
                } else if name.contains(&q) {
                    2
                } else if o
                    .description
                    .as_deref()
                    .is_some_and(|d| d.to_lowercase().contains(&q))
                {
                    3
                } else {
                    return None;
                };
                Some((rank * 2 + u8::from(!o.is_pollable()), o))
            })
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
        let mut seen = HashSet::new();
        scored
            .into_iter()
            .map(|(_, o)| o)
            .filter(|o| seen.insert(o.oid.clone()))
            .take(limit)
            .collect()
    }
}

/// What a custom OID sensor polls: the numeric OID and, if known, the MIB object behind it.
#[derive(Debug, Clone, PartialEq)]
pub struct OidTarget {
    pub oid: Vec<u64>,
    pub object: Option<MibObject>,
}

impl MibIndex {
    /// Interprets the OID field of the sensor form: numeric (`1.3.6.1.2.1.1.3.0`) or by name
    /// (`sysUpTime.0`, `IF-MIB::ifInOctets.3`). A scalar given without instance gets `.0`;
    /// tables, rows, plain nodes, columns without an index and unreadable objects are refused
    /// with an explanation. Messages are user-facing (German).
    pub fn oid_target(&self, input: &str) -> Result<OidTarget, Msg> {
        let input = input.trim();
        if input.is_empty() {
            return Err(Msg::new("mib-oid-missing"));
        }
        // Anything starting with a digit is meant as a numeric OID (and gets that error message).
        let numeric = input
            .trim_start_matches('.')
            .starts_with(|c: char| c.is_ascii_digit());
        let (mut oid, object) = if numeric {
            let oid = crate::sensors::spec::parse_oid(input)?;
            // Only an object that the OID points into as a value counts (not e.g. `enterprises`).
            let object = self
                .describe(&oid)
                .map(|(o, _)| o)
                .filter(|o| matches!(o.kind, ObjectKind::Scalar | ObjectKind::Column))
                .cloned();
            (oid, object)
        } else {
            let (oid, object) = self.resolve_input(input)?;
            match object.kind {
                ObjectKind::Scalar | ObjectKind::Column => {}
                other => {
                    return Err(Msg::new("mib-not-pollable")
                        .with("name", object.name.as_str())
                        .with("kind", Msg::new(other.label_id())));
                }
            }
            (oid, Some(object.clone()))
        };
        if let Some(o) = &object {
            if o.access.as_deref().is_some_and(|a| !a.starts_with("read")) {
                return Err(Msg::new("mib-not-readable")
                    .with("name", o.name.as_str())
                    .with("access", o.access.as_deref().unwrap_or("")));
            }
            if oid == o.oid {
                match o.kind {
                    ObjectKind::Scalar => oid.push(0),
                    _ => {
                        let message = if o.index.is_empty() {
                            Msg::new("mib-column-needs-index-unknown")
                        } else {
                            Msg::new("mib-column-needs-index").with("index", o.index.join(", "))
                        };
                        return Err(message.with("name", o.name.as_str()));
                    }
                }
            }
        }
        Ok(OidTarget { oid, object })
    }
}

/// Helper to get a `&'a str` for a module name from the resolver's map.
fn module_static<'a>(resolver: &Resolver<'a>, module: &str) -> &'a str {
    resolver
        .modules
        .get(module)
        .map_or("", |l| l.module.name.as_str())
}

/// Formats an OID as dotted numbers.
pub fn dotted(oid: &[u64]) -> String {
    oid.iter().map(u64::to_string).collect::<Vec<_>>().join(".")
}

#[cfg(test)]
mod tests;
