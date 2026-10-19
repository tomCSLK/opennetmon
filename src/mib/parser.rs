//! Parser for SMIv1/SMIv2 MIB modules. Extracts what monitoring needs: imports, OID assignments,
//! OBJECT-TYPE details (syntax, access, units, description, index) and type definitions (textual
//! conventions). Everything else (compliance statements, macro definitions, …) is skipped.
//!
//! The parser is deliberately tolerant: vendor MIBs are often not quite standard-conformant, so
//! anything unexpected is skipped with a warning instead of aborting the module.

use super::lexer::{Spanned, Token, tokenize};

/// One component of an OID value: `{ iso org(3) 6 enterprises }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OidPart {
    Name(String),
    Number(u64),
    /// `org(3)`: a name with its number; the number decides.
    Named(String, u64),
}

/// A syntax as written: a base type or a reference to a named type, with optional enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxKind {
    /// A named type: `Integer32`, `DisplayString`, `INTEGER`, `OCTET STRING` (as `OCTET STRING`), …
    Named(String),
    /// `SEQUENCE OF Entry` (a table).
    SequenceOf(String),
    /// `SEQUENCE { … }` or `CHOICE { … }` (row type definitions; not needed further).
    Structure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Syntax {
    pub kind: SyntaxKind,
    /// Named numbers of `INTEGER { up(1), down(2) }` or `BITS { … }`.
    pub enums: Vec<(String, i64)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObjectType {
    pub syntax: Option<Syntax>,
    pub units: Option<String>,
    pub access: Option<String>,
    pub status: Option<String>,
    pub description: Option<String>,
    pub index: Vec<String>,
    pub augments: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefKind {
    /// `name OBJECT IDENTIFIER ::= { … }`
    Identifier,
    ObjectType(ObjectType),
    /// MODULE-IDENTITY, OBJECT-IDENTITY, NOTIFICATION-TYPE, groups, compliance statements, …
    Other(String),
}

/// A value assignment with an OID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidDef {
    pub name: String,
    pub value: Vec<OidPart>,
    pub kind: DefKind,
    pub line: usize,
}

/// `Name ::= …` or `Name ::= TEXTUAL-CONVENTION …`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDef {
    pub name: String,
    pub syntax: Syntax,
    pub display_hint: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Module {
    pub name: String,
    /// `(module, names)` per `FROM` clause.
    pub imports: Vec<(String, Vec<String>)>,
    pub defs: Vec<OidDef>,
    pub types: Vec<TypeDef>,
    /// Things the parser skipped, with line numbers (for diagnostics, never fatal).
    pub warnings: Vec<String>,
}

/// Macro names that introduce a value assignment ending in `::= value`.
const VALUE_MACROS: [&str; 10] = [
    "OBJECT-TYPE",
    "MODULE-IDENTITY",
    "OBJECT-IDENTITY",
    "NOTIFICATION-TYPE",
    "OBJECT-GROUP",
    "NOTIFICATION-GROUP",
    "MODULE-COMPLIANCE",
    "AGENT-CAPABILITIES",
    "TRAP-TYPE",
    "OBJECT-IDENTITY",
];

struct Parser {
    tokens: Vec<Spanned>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|s| &s.token)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset).map(|s| &s.token)
    }

    fn line(&self) -> usize {
        self.tokens
            .get(self.pos)
            .or_else(|| self.tokens.last())
            .map_or(0, |s| s.line)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).map(|s| s.token.clone());
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn is_ident(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Ident(w)) if w == word)
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_ident(&mut self, word: &str) -> bool {
        if self.is_ident(word) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Skips a balanced group starting at the current opening token (`{`, `(` or `[`).
    fn skip_group(&mut self) {
        let (open, close) = match self.peek() {
            Some(Token::LBrace) => (Token::LBrace, Token::RBrace),
            Some(Token::LParen) => (Token::LParen, Token::RParen),
            Some(Token::LBracket) => (Token::LBracket, Token::RBracket),
            _ => return,
        };
        let mut depth = 0usize;
        while let Some(t) = self.next() {
            if t == open {
                depth += 1;
            } else if t == close {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return;
                }
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        match self.peek() {
            Some(Token::Str(s)) => {
                let s = s.clone();
                self.pos += 1;
                Some(s)
            }
            _ => None,
        }
    }

    fn ident(&mut self) -> Option<String> {
        match self.peek() {
            Some(Token::Ident(s)) => {
                let s = s.clone();
                self.pos += 1;
                Some(s)
            }
            _ => None,
        }
    }

    /// `{ a(1), b(2) }` → named numbers. Expects the opening brace.
    fn named_numbers(&mut self) -> Vec<(String, i64)> {
        let mut out = Vec::new();
        if !self.eat(&Token::LBrace) {
            return out;
        }
        loop {
            match self.peek() {
                None => break,
                Some(Token::RBrace) => {
                    self.pos += 1;
                    break;
                }
                Some(Token::Ident(_)) => {
                    // Tolerate vendor names like `n/a(0)`: glue identifier parts and stray characters.
                    let mut name = String::new();
                    loop {
                        match self.peek() {
                            Some(Token::Ident(part)) => name.push_str(part),
                            Some(Token::Other(c)) => name.push(*c),
                            _ => break,
                        }
                        self.pos += 1;
                    }
                    if self.eat(&Token::LParen) {
                        if let Some(Token::Number(n)) = self.peek().cloned() {
                            self.pos += 1;
                            if let Ok(n) = n.parse::<i64>() {
                                out.push((name, n));
                            }
                        }
                        // Skip to the closing parenthesis.
                        while let Some(t) = self.next() {
                            if t == Token::RParen {
                                break;
                            }
                        }
                    }
                }
                Some(_) => self.pos += 1,
            }
        }
        out
    }

    /// Skips size/range constraints like `(0..100)` or `(SIZE (0..255))`.
    fn skip_constraints(&mut self) {
        while self.peek() == Some(&Token::LParen) {
            self.skip_group();
        }
    }

    fn syntax(&mut self) -> Syntax {
        let plain = |kind| Syntax {
            kind,
            enums: Vec::new(),
        };
        // `[APPLICATION 1] IMPLICIT INTEGER …` (base type definitions in the SMI modules).
        if self.peek() == Some(&Token::LBracket) {
            self.skip_group();
            self.eat_ident("IMPLICIT");
        }
        let Some(first) = self.ident() else {
            return plain(SyntaxKind::Named(String::new()));
        };
        match first.as_str() {
            "SEQUENCE" if self.eat_ident("OF") => {
                let entry = self.ident().unwrap_or_default();
                plain(SyntaxKind::SequenceOf(entry))
            }
            "SEQUENCE" | "CHOICE" => {
                self.skip_group();
                plain(SyntaxKind::Structure)
            }
            "OCTET" => {
                self.eat_ident("STRING");
                self.skip_constraints();
                plain(SyntaxKind::Named("OCTET STRING".to_owned()))
            }
            "OBJECT" => {
                self.eat_ident("IDENTIFIER");
                plain(SyntaxKind::Named("OBJECT IDENTIFIER".to_owned()))
            }
            _ => {
                let enums = if self.peek() == Some(&Token::LBrace) {
                    self.named_numbers()
                } else {
                    Vec::new()
                };
                self.skip_constraints();
                Syntax {
                    kind: SyntaxKind::Named(first),
                    enums,
                }
            }
        }
    }

    /// `{ parent 1 }`, `{ iso org(3) dod(6) 1 }`. Expects the opening brace.
    fn oid_value(&mut self) -> Option<Vec<OidPart>> {
        if !self.eat(&Token::LBrace) {
            return None;
        }
        let mut parts = Vec::new();
        loop {
            match self.next()? {
                Token::RBrace => return Some(parts),
                Token::Number(n) => parts.push(OidPart::Number(n.parse().ok()?)),
                Token::Ident(name) => {
                    if self.eat(&Token::LParen) {
                        let number = match self.next() {
                            Some(Token::Number(n)) => n.parse().ok(),
                            _ => None,
                        };
                        self.eat(&Token::RParen);
                        match number {
                            Some(n) => parts.push(OidPart::Named(name, n)),
                            None => parts.push(OidPart::Name(name)),
                        }
                    } else {
                        parts.push(OidPart::Name(name));
                    }
                }
                _ => {}
            }
        }
    }

    /// Clauses of an OBJECT-TYPE up to (not including) `::=`.
    fn object_type_clauses(&mut self) -> ObjectType {
        let mut ot = ObjectType::default();
        while let Some(token) = self.peek().cloned() {
            match token {
                Token::Assign => break,
                Token::Ident(word) => {
                    self.pos += 1;
                    match word.as_str() {
                        "SYNTAX" => ot.syntax = Some(self.syntax()),
                        "UNITS" => ot.units = self.string(),
                        "MAX-ACCESS" | "ACCESS" | "MIN-ACCESS" => ot.access = self.ident(),
                        "STATUS" => ot.status = self.ident(),
                        "DESCRIPTION" => ot.description = self.string(),
                        "REFERENCE" => {
                            self.string();
                        }
                        "INDEX" => {
                            if self.eat(&Token::LBrace) {
                                while let Some(t) = self.next() {
                                    match t {
                                        Token::RBrace => break,
                                        Token::Ident(name) if name != "IMPLIED" => {
                                            ot.index.push(name)
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        "AUGMENTS" => {
                            if self.eat(&Token::LBrace) {
                                ot.augments = self.ident();
                                while let Some(t) = self.next() {
                                    if t == Token::RBrace {
                                        break;
                                    }
                                }
                            }
                        }
                        "DEFVAL" => self.skip_group(),
                        _ => {}
                    }
                }
                Token::LBrace | Token::LParen | Token::LBracket => self.skip_group(),
                _ => self.pos += 1,
            }
        }
        ot
    }

    /// Skips the clauses of any other macro up to `::=` (balanced groups as a whole).
    fn skip_to_assign(&mut self) {
        while let Some(token) = self.peek() {
            match token {
                Token::Assign => return,
                Token::LBrace | Token::LParen | Token::LBracket => self.skip_group(),
                _ => self.pos += 1,
            }
        }
    }

    /// Skips a `MACRO ::= BEGIN … END` definition.
    fn skip_macro(&mut self) {
        while let Some(t) = self.next() {
            if t == Token::Ident("END".to_owned()) {
                return;
            }
        }
    }

    fn imports(&mut self, module: &mut Module) {
        let mut names = Vec::new();
        while let Some(t) = self.next() {
            match t {
                Token::Semicolon => break,
                Token::Ident(word) if word == "FROM" => {
                    if let Some(from) = self.ident() {
                        module.imports.push((from, std::mem::take(&mut names)));
                    }
                }
                Token::Ident(word) => names.push(word),
                _ => {}
            }
        }
    }

    /// Parses a type assignment after `Name ::=`.
    fn type_assignment(&mut self, name: String, module: &mut Module) {
        if self.eat_ident("TEXTUAL-CONVENTION") {
            let mut display_hint = None;
            let mut description = None;
            while let Some(token) = self.peek().cloned() {
                match token {
                    Token::Ident(word) => {
                        self.pos += 1;
                        match word.as_str() {
                            "DISPLAY-HINT" => display_hint = self.string(),
                            "DESCRIPTION" => description = self.string(),
                            "STATUS" => {
                                self.ident();
                            }
                            "REFERENCE" => {
                                self.string();
                            }
                            "SYNTAX" => {
                                let syntax = self.syntax();
                                module.types.push(TypeDef {
                                    name,
                                    syntax,
                                    display_hint,
                                    description,
                                });
                                return;
                            }
                            _ => {}
                        }
                    }
                    _ => self.pos += 1,
                }
            }
            return;
        }
        let syntax = self.syntax();
        module.types.push(TypeDef {
            name,
            syntax,
            display_hint: None,
            description: None,
        });
    }

    /// The body of one module after `BEGIN`, up to its `END`.
    fn body(&mut self, module: &mut Module) {
        while let Some(token) = self.peek().cloned() {
            let line = self.line();
            match token {
                Token::Ident(word) if word == "END" => {
                    self.pos += 1;
                    return;
                }
                Token::Ident(word) if word == "IMPORTS" => {
                    self.pos += 1;
                    self.imports(module);
                }
                Token::Ident(word) if word == "EXPORTS" => {
                    while let Some(t) = self.next() {
                        if t == Token::Semicolon {
                            break;
                        }
                    }
                }
                Token::Ident(name) => {
                    self.pos += 1;
                    self.assignment(name, line, module);
                }
                other => {
                    module.warnings.push(format!(
                        "line {line}: unexpected character {other:?} skipped"
                    ));
                    self.pos += 1;
                }
            }
        }
    }

    fn assignment(&mut self, name: String, line: usize, module: &mut Module) {
        // `name OBJECT IDENTIFIER ::= { … }`
        if self.is_ident("OBJECT")
            && matches!(self.peek_at(1), Some(Token::Ident(w)) if w == "IDENTIFIER")
        {
            self.pos += 2;
            if self.eat(&Token::Assign)
                && let Some(value) = self.oid_value()
            {
                module.defs.push(OidDef {
                    name,
                    value,
                    kind: DefKind::Identifier,
                    line,
                });
            }
            return;
        }
        if self.eat_ident("MACRO") {
            self.skip_macro();
            return;
        }
        if let Some(Token::Ident(macro_name)) = self.peek().cloned()
            && VALUE_MACROS.contains(&macro_name.as_str())
        {
            self.pos += 1;
            let kind = if macro_name == "OBJECT-TYPE" {
                DefKind::ObjectType(self.object_type_clauses())
            } else {
                self.skip_to_assign();
                DefKind::Other(macro_name.clone())
            };
            if !self.eat(&Token::Assign) {
                module
                    .warnings
                    .push(format!("line {line}: {name}: no \"::=\" found"));
                return;
            }
            match self.peek() {
                Some(Token::LBrace) => {
                    if let Some(value) = self.oid_value() {
                        module.defs.push(OidDef {
                            name,
                            value,
                            kind,
                            line,
                        });
                    }
                }
                // TRAP-TYPE ::= 3 (a trap number, not an OID).
                _ => {
                    self.next();
                }
            }
            return;
        }
        if self.eat(&Token::Assign) {
            self.type_assignment(name, module);
            return;
        }
        module
            .warnings
            .push(format!("line {line}: \"{name}\" not understood, skipped"));
    }
}

/// Parses all modules in `text` (a file usually holds one).
pub fn parse(text: &str) -> Vec<Module> {
    let mut p = Parser {
        tokens: tokenize(text),
        pos: 0,
    };
    let mut modules = Vec::new();
    while p.pos < p.tokens.len() {
        // Find `Name DEFINITIONS ::= BEGIN`.
        let found = matches!(
            (p.peek(), p.peek_at(1), p.peek_at(2), p.peek_at(3)),
            (Some(Token::Ident(_)), Some(Token::Ident(d)), Some(Token::Assign), Some(Token::Ident(b)))
                if d == "DEFINITIONS" && b == "BEGIN"
        );
        if !found {
            p.pos += 1;
            continue;
        }
        let mut module = Module {
            name: p.ident().unwrap_or_default(),
            ..Module::default()
        };
        p.pos += 3;
        p.body(&mut module);
        modules.push(module);
    }
    modules
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
TEST-MIB DEFINITIONS ::= BEGIN
IMPORTS
    MODULE-IDENTITY, OBJECT-TYPE, Integer32, Counter64, enterprises FROM SNMPv2-SMI
    DisplayString, TEXTUAL-CONVENTION FROM SNMPv2-TC;

testMIB MODULE-IDENTITY
    LAST-UPDATED "202401010000Z"
    ORGANIZATION "Test"
    CONTACT-INFO "none"
    DESCRIPTION "A test module."
    REVISION "202401010000Z"
    DESCRIPTION "First."
    ::= { enterprises 99999 }

FanState ::= TEXTUAL-CONVENTION
    DISPLAY-HINT "d"
    STATUS current
    DESCRIPTION "State of a fan."
    SYNTAX INTEGER { ok(1), failed(2) }

testObjects OBJECT IDENTIFIER ::= { testMIB 1 }

testTemperature OBJECT-TYPE
    SYNTAX Integer32 (-50..150)
    UNITS "degrees Celsius"
    MAX-ACCESS read-only
    STATUS current
    DESCRIPTION "Board temperature."
    ::= { testObjects 1 }

testFanTable OBJECT-TYPE
    SYNTAX SEQUENCE OF TestFanEntry
    MAX-ACCESS not-accessible
    STATUS current
    DESCRIPTION "Fans."
    ::= { testObjects 2 }

testFanEntry OBJECT-TYPE
    SYNTAX TestFanEntry
    MAX-ACCESS not-accessible
    STATUS current
    DESCRIPTION "A fan."
    INDEX { testFanIndex }
    ::= { testFanTable 1 }

TestFanEntry ::= SEQUENCE { testFanIndex Integer32, testFanState FanState }

testFanState OBJECT-TYPE
    SYNTAX FanState
    MAX-ACCESS read-only
    STATUS current
    DESCRIPTION "Fan state."
    DEFVAL { ok }
    ::= { testFanEntry 2 }

testOctets OBJECT-TYPE
    SYNTAX Counter64
    MAX-ACCESS read-only
    STATUS current
    DESCRIPTION "Bytes."
    ::= { testObjects 3 }

testTrap TRAP-TYPE
    ENTERPRISE testMIB
    VARIABLES { testTemperature }
    ::= 7
END
"#;

    fn def<'a>(m: &'a Module, name: &str) -> &'a OidDef {
        m.defs.iter().find(|d| d.name == name).unwrap()
    }

    #[test]
    fn a_module_is_parsed_with_imports_types_and_objects() {
        let modules = parse(SAMPLE);
        assert_eq!(modules.len(), 1);
        let m = &modules[0];
        assert_eq!(m.name, "TEST-MIB");
        assert_eq!(m.imports.len(), 2);
        assert_eq!(m.imports[0].0, "SNMPv2-SMI");
        assert!(m.imports[0].1.contains(&"enterprises".to_owned()));
        assert_eq!(
            m.imports[1],
            (
                "SNMPv2-TC".to_owned(),
                vec!["DisplayString".to_owned(), "TEXTUAL-CONVENTION".to_owned()]
            )
        );
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);

        assert_eq!(
            def(m, "testMIB").value,
            [
                OidPart::Name("enterprises".to_owned()),
                OidPart::Number(99999)
            ]
        );
        assert_eq!(
            def(m, "testMIB").kind,
            DefKind::Other("MODULE-IDENTITY".to_owned())
        );
        assert_eq!(def(m, "testObjects").kind, DefKind::Identifier);

        let DefKind::ObjectType(temp) = &def(m, "testTemperature").kind else {
            panic!()
        };
        assert_eq!(
            temp.syntax.as_ref().unwrap().kind,
            SyntaxKind::Named("Integer32".to_owned())
        );
        assert_eq!(temp.units.as_deref(), Some("degrees Celsius"));
        assert_eq!(temp.access.as_deref(), Some("read-only"));
        assert_eq!(temp.description.as_deref(), Some("Board temperature."));

        let DefKind::ObjectType(table) = &def(m, "testFanTable").kind else {
            panic!()
        };
        assert_eq!(
            table.syntax.as_ref().unwrap().kind,
            SyntaxKind::SequenceOf("TestFanEntry".to_owned())
        );
        let DefKind::ObjectType(entry) = &def(m, "testFanEntry").kind else {
            panic!()
        };
        assert_eq!(entry.index, ["testFanIndex"]);

        let tc = m.types.iter().find(|t| t.name == "FanState").unwrap();
        assert_eq!(
            tc.syntax.enums,
            [("ok".to_owned(), 1), ("failed".to_owned(), 2)]
        );
        assert_eq!(tc.display_hint.as_deref(), Some("d"));
        assert!(
            m.types
                .iter()
                .any(|t| t.name == "TestFanEntry" && t.syntax.kind == SyntaxKind::Structure)
        );
        assert!(
            m.defs.iter().all(|d| d.name != "testTrap"),
            "trap numbers are not OIDs"
        );
    }

    #[test]
    fn oid_values_with_named_numbers_and_smi_base_types() {
        let text = "X DEFINITIONS ::= BEGIN
            internet OBJECT IDENTIFIER ::= { iso org(3) dod(6) 1 }
            Counter32 ::= [APPLICATION 1] IMPLICIT INTEGER (0..4294967295)
            OBJECT-TYPE MACRO ::= BEGIN TYPE NOTATION ::= \"SYNTAX\" Syntax VALUE NOTATION ::= value(VALUE ObjectName) END
            ifType OBJECT-TYPE SYNTAX INTEGER { other(1), ethernetCsmacd(6) } ACCESS read-only STATUS mandatory ::= { ifEntry 3 }
        END";
        let m = &parse(text)[0];
        assert_eq!(
            def(m, "internet").value,
            [
                OidPart::Name("iso".to_owned()),
                OidPart::Named("org".to_owned(), 3),
                OidPart::Named("dod".to_owned(), 6),
                OidPart::Number(1)
            ]
        );
        assert!(
            m.types.iter().any(|t| t.name == "Counter32"
                && t.syntax.kind == SyntaxKind::Named("INTEGER".to_owned()))
        );
        let DefKind::ObjectType(ot) = &def(m, "ifType").kind else {
            panic!()
        };
        assert_eq!(
            ot.syntax.as_ref().unwrap().enums,
            [("other".to_owned(), 1), ("ethernetCsmacd".to_owned(), 6)]
        );
        assert_eq!(ot.access.as_deref(), Some("read-only"));
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    }

    #[test]
    fn vendor_enumeration_names_with_odd_characters_are_kept() {
        let text = "V DEFINITIONS ::= BEGIN
            x OBJECT-TYPE SYNTAX INTEGER { n/a(0), power_user(1), stp(2) } MAX-ACCESS read-only STATUS current ::= { iso 9 }
        END";
        let m = &parse(text)[0];
        let DefKind::ObjectType(ot) = &def(m, "x").kind else {
            panic!()
        };
        assert_eq!(
            ot.syntax.as_ref().unwrap().enums,
            [
                ("n/a".to_owned(), 0),
                ("power_user".to_owned(), 1),
                ("stp".to_owned(), 2)
            ]
        );
    }

    #[test]
    fn several_modules_garbage_and_truncation_never_panic() {
        let two = "A DEFINITIONS ::= BEGIN a OBJECT IDENTIFIER ::= { iso 1 } END B DEFINITIONS ::= BEGIN END";
        let modules = parse(two);
        assert_eq!(
            modules.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            ["A", "B"]
        );
        assert!(parse("no module here at all").is_empty());
        for cut in 0..SAMPLE.len() {
            if SAMPLE.is_char_boundary(cut) {
                let _ = parse(&SAMPLE[..cut]);
            }
        }
        let weird = parse("W DEFINITIONS ::= BEGIN @@@ x OBJECT-TYPE SYNTAX ::= { } y z END");
        assert_eq!(weird.len(), 1);
        assert!(!weird[0].warnings.is_empty());
    }
}
