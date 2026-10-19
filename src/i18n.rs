//! Translations: the supported languages, the Fluent catalogs in `locales/<code>/*.ftl` (embedded) and the
//! language of the current request.
//!
//! German is the source language; every catalog must contain the same messages with the same variables
//! (checked by the tests). Pages get the language as a template field `t` and call `t.get("nav-overview")`; Rust code
//! that has no request at hand returns a [`Msg`] that is translated where it is shown.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::LazyLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "locales/"]
struct Catalogs;

/// A language of the user interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    #[default]
    De,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::De, Lang::En];

    /// ISO 639-1 code, as stored in the database and used in `<html lang>`.
    pub fn code(self) -> &'static str {
        match self {
            Self::De => "de",
            Self::En => "en",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.code() == code)
    }

    /// The language's own name, shown in language selections.
    pub fn native_name(self) -> &'static str {
        match self {
            Self::De => "Deutsch",
            Self::En => "English",
        }
    }

    /// The preferred supported language of an `Accept-Language` header; German if none matches.
    pub fn from_accept_language(header: &str) -> Self {
        let mut best: Option<(f32, Lang)> = None;
        for part in header.split(',') {
            let mut fields = part.split(';');
            let tag = fields.next().unwrap_or_default().trim();
            let primary = tag
                .split('-')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let Some(lang) = Self::parse(&primary) else {
                continue;
            };
            let quality = fields
                .find_map(|f| f.trim().strip_prefix("q="))
                .map_or(Some(1.0), |q| q.trim().parse::<f32>().ok())
                .unwrap_or(0.0);
            if quality > 0.0 && best.is_none_or(|(q, _)| quality > q) {
                best = Some((quality, lang));
            }
        }
        best.map_or_else(Self::default, |(_, lang)| lang)
    }

    /// The text of a message without variables. Unknown ids come back unchanged (and are logged).
    pub fn get(self, id: &str) -> String {
        self.fmt(id, &[])
    }

    /// A message with one variable, e.g. `t.get1("user-delete-confirm", "name", name)` in a template.
    pub fn get1(self, id: &str, name: &str, value: impl Into<Arg>) -> String {
        self.fmt(id, &[(name, value.into())])
    }

    /// A message with two variables.
    pub fn get2(
        self,
        id: &str,
        name1: &str,
        value1: impl Into<Arg>,
        name2: &str,
        value2: impl Into<Arg>,
    ) -> String {
        self.fmt(id, &[(name1, value1.into()), (name2, value2.into())])
    }

    /// A message with any number of variables.
    pub fn fmt(self, id: &str, args: &[(&str, Arg)]) -> String {
        let bundle = bundle(self);
        let Some(pattern) = bundle.get_message(id).and_then(|m| m.value()) else {
            tracing::warn!(lang = self.code(), id, "missing translation");
            return id.to_owned();
        };
        let mut fluent_args = FluentArgs::new();
        for (name, value) in args {
            fluent_args.set(*name, value.to_fluent());
        }
        let mut errors = Vec::new();
        let text = bundle.format_pattern(pattern, Some(&fluent_args), &mut errors);
        if !errors.is_empty() {
            tracing::warn!(
                lang = self.code(),
                id,
                ?errors,
                "translation could not be formatted"
            );
        }
        text.into_owned()
    }

    /// The text of a [`Msg`] in this language.
    pub fn msg(self, msg: &Msg) -> String {
        match msg {
            Msg::Key { id, args } => self.fmt(id, args),
            Msg::Text(text) => text.clone(),
        }
    }
}

/// A value for a message variable.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Text(String),
    Number(f64),
}

impl Arg {
    fn to_fluent(&self) -> FluentValue<'_> {
        match self {
            Self::Text(text) => FluentValue::from(text.as_str()),
            Self::Number(n) => FluentValue::from(*n),
        }
    }
}

impl From<&str> for Arg {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

impl From<String> for Arg {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<&String> for Arg {
    fn from(text: &String) -> Self {
        Self::Text(text.clone())
    }
}

macro_rules! number_arg {
    ($($t:ty)*) => {$(
        impl From<$t> for Arg {
            fn from(n: $t) -> Self {
                Self::Number(n as f64)
            }
        }
        impl From<&$t> for Arg {
            fn from(n: &$t) -> Self {
                Self::Number(*n as f64)
            }
        }
    )*};
}
number_arg!(i32 i64 u32 u64 usize f64);

/// A user-facing message that is translated where it is shown: a catalog id with variables, or a text that
/// is shown as it is (e.g. an error text reported by a device).
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Key {
        id: &'static str,
        args: Vec<(&'static str, Arg)>,
    },
    Text(String),
}

impl Msg {
    pub fn new(id: &'static str) -> Self {
        Self::Key {
            id,
            args: Vec::new(),
        }
    }

    /// Adds a variable.
    pub fn with(mut self, name: &'static str, value: impl Into<Arg>) -> Self {
        if let Self::Key { args, .. } = &mut self {
            args.push((name, value.into()));
        }
        self
    }
}

/// A text shown as it is (not yet translated, or reported by a device).
impl From<String> for Msg {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<&str> for Msg {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

/// German text of a message, for logs and `Display` of error types.
impl std::fmt::Display for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&Lang::De.msg(self))
    }
}

type Bundle = FluentBundle<FluentResource>;

/// All catalogs, one bundle per language in the order of [`Lang::ALL`].
static BUNDLES: LazyLock<Vec<Bundle>> = LazyLock::new(|| Lang::ALL.map(load).into());

fn bundle(lang: Lang) -> &'static Bundle {
    let index = Lang::ALL.iter().position(|l| *l == lang).unwrap_or(0);
    &BUNDLES[index]
}

/// The catalog files of a language, by file name.
fn sources(lang: Lang) -> BTreeMap<String, String> {
    let prefix = format!("{}/", lang.code());
    Catalogs::iter()
        .filter_map(|path| {
            let name = path.strip_prefix(&prefix)?.to_owned();
            let file = Catalogs::get(&path)?;
            Some((name, String::from_utf8_lossy(&file.data).into_owned()))
        })
        .collect()
}

/// Builds the bundle of a language. Syntax errors and duplicates are logged and skipped (the tests reject
/// them), so a broken entry never stops the server.
fn load(lang: Lang) -> Bundle {
    let mut bundle = FluentBundle::new_concurrent(vec![lang.code().parse().unwrap_or_default()]);
    bundle.set_use_isolating(false);
    if lang == Lang::De {
        bundle.set_formatter(Some(decimal_comma));
    }
    for (name, source) in sources(lang) {
        let resource = FluentResource::try_new(source).unwrap_or_else(|(resource, errors)| {
            tracing::error!(lang = lang.code(), file = %name, ?errors, "translation catalog has syntax errors");
            resource
        });
        if let Err(errors) = bundle.add_resource(resource) {
            tracing::error!(lang = lang.code(), file = %name, ?errors, "translation catalog has duplicate messages");
        }
    }
    bundle
}

/// Numbers in German texts use a decimal comma.
fn decimal_comma<M>(value: &FluentValue<'_>, _: &M) -> Option<String> {
    match value {
        FluentValue::Number(n) => Some(n.as_string().replace('.', ",")),
        _ => None,
    }
}

tokio::task_local! {
    static REQUEST_LANG: Cell<Lang>;
}

/// Runs a request with its language (from `Accept-Language`); [`set_current`] changes it once the
/// signed-in user is known.
pub async fn scope<F: Future>(lang: Lang, request: F) -> F::Output {
    REQUEST_LANG.scope(Cell::new(lang), request).await
}

/// The language of the current request; German outside of a request.
pub fn current() -> Lang {
    REQUEST_LANG.try_with(Cell::get).unwrap_or_default()
}

pub fn set_current(lang: Lang) {
    // Outside of a request there is nothing to set.
    let _ = REQUEST_LANG.try_with(|cell| cell.set(lang));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Message ids and their variables, from a catalog source (one message per unindented `id =` line).
    fn messages(source: &str) -> BTreeMap<String, BTreeSet<String>> {
        let mut result: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut current: Option<String> = None;
        for line in source.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            if !line.starts_with(' ')
                && let Some((id, _)) = line.split_once('=')
            {
                let id = id.trim().to_owned();
                assert!(!result.contains_key(&id), "message {id} is defined twice");
                result.insert(id.clone(), BTreeSet::new());
                current = Some(id);
            }
            let Some(id) = &current else { continue };
            let mut rest = line;
            while let Some(pos) = rest.find('$') {
                rest = &rest[pos + 1..];
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                    .collect();
                if let Some(vars) = result.get_mut(id) {
                    vars.insert(name);
                }
            }
        }
        result
    }

    fn all_messages(lang: Lang) -> BTreeMap<String, BTreeSet<String>> {
        let mut all = BTreeMap::new();
        for (file, source) in sources(lang) {
            for (id, vars) in messages(&source) {
                assert!(
                    all.insert(id.clone(), vars).is_none(),
                    "{}: message {id} is defined in more than one file ({file})",
                    lang.code()
                );
            }
        }
        all
    }

    #[test]
    fn catalogs_parse_without_errors() {
        for lang in Lang::ALL {
            let files = sources(lang);
            assert!(!files.is_empty(), "no catalog for {}", lang.code());
            for (name, source) in files {
                if let Err((_, errors)) = FluentResource::try_new(source) {
                    panic!("{}/{name}: {errors:?}", lang.code());
                }
            }
        }
    }

    #[test]
    fn every_language_has_the_same_files_messages_and_variables() {
        let files = |lang| sources(lang).into_keys().collect::<Vec<_>>();
        let reference = all_messages(Lang::De);
        for lang in Lang::ALL {
            assert_eq!(files(lang), files(Lang::De), "files of {}", lang.code());
            let other = all_messages(lang);
            for (id, vars) in &reference {
                match other.get(id) {
                    None => panic!("{}: message {id} is missing", lang.code()),
                    Some(v) => assert_eq!(v, vars, "{}: variables of {id}", lang.code()),
                }
            }
            for id in other.keys() {
                assert!(
                    reference.contains_key(id),
                    "{}: message {id} is not in the German catalog",
                    lang.code()
                );
            }
        }
    }

    /// Source files of the program (templates and Rust code) with their contents.
    fn program_sources() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs" || e == "html") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    out.push((path.display().to_string(), text));
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut out = Vec::new();
        walk(&root.join("src"), &mut out);
        walk(&root.join("templates"), &mut out);
        out
    }

    /// Whether the text before a `.get(` call ends with a language: `t`, `lang`, `user.lang`, `Lang::En` …
    fn is_language_receiver(before: &str) -> bool {
        let ident: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == ':')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        ident == "t" || ident.ends_with("lang") || ident.starts_with("Lang::")
    }

    /// Every id passed literally to a translation call exists in the catalogs.
    #[test]
    fn every_used_message_exists() {
        let known = all_messages(Lang::De);
        let mut checked = 0;
        for (file, text) in program_sources() {
            for call in [".get(\"", ".get1(\"", ".get2(\"", ".fmt(\"", "Msg::new(\""] {
                for (pos, _) in text.match_indices(call) {
                    if call.starts_with('.') && !is_language_receiver(&text[..pos]) {
                        continue;
                    }
                    let rest = &text[pos + call.len()..];
                    let id = &rest[..rest.find('"').unwrap_or(0)];
                    if file.ends_with("i18n.rs") && id == "no-such-message" {
                        continue;
                    }
                    assert!(known.contains_key(id), "{file}: unknown message {id}");
                    checked += 1;
                }
            }
        }
        assert!(checked > 10, "the scan found almost no translation calls");
    }

    /// Every message is used somewhere (as a string literal), so the catalogs do not collect dead texts.
    #[test]
    fn every_message_is_used() {
        let mut sources = program_sources();
        sources.retain(|(file, _)| !file.ends_with("i18n.rs"));
        for id in all_messages(Lang::De).keys() {
            let quoted = format!("\"{id}\"");
            assert!(
                sources.iter().any(|(_, text)| text.contains(&quoted)),
                "message {id} is not used"
            );
        }
    }

    #[test]
    fn accept_language_picks_the_best_supported_language() {
        assert_eq!(Lang::from_accept_language(""), Lang::De);
        assert_eq!(Lang::from_accept_language("en-US,en;q=0.9"), Lang::En);
        assert_eq!(
            Lang::from_accept_language("fr-FR, en;q=0.5, de;q=0.8"),
            Lang::De
        );
        assert_eq!(Lang::from_accept_language("fr, it"), Lang::De);
        assert_eq!(Lang::from_accept_language("EN-gb"), Lang::En);
        assert_eq!(Lang::from_accept_language("en;q=0, de;q=0.1"), Lang::De);
        assert_eq!(Lang::from_accept_language("en;q=0"), Lang::De);
        assert_eq!(Lang::from_accept_language("*;q=0.5, en"), Lang::En);
    }

    #[test]
    fn messages_are_formatted_with_variables_and_plurals() {
        assert_eq!(Lang::De.get("role-admin"), "Administrator");
        assert_eq!(Lang::En.get("role-viewer"), "Viewer");
        assert_eq!(
            Lang::De.get1("login-wait-minutes", "minutes", 1),
            "1 Minute"
        );
        assert_eq!(
            Lang::De.get1("login-wait-minutes", "minutes", 3),
            "3 Minuten"
        );
        assert_eq!(
            Lang::En.get1("login-wait-seconds", "seconds", 1),
            "1 second"
        );
        assert_eq!(
            Lang::En.get1("login-wait-seconds", "seconds", 20),
            "20 seconds"
        );
    }

    #[test]
    fn german_numbers_use_a_decimal_comma() {
        assert_eq!(
            decimal_comma(&FluentValue::from(2.5), &()),
            Some("2,5".to_owned())
        );
        assert_eq!(
            decimal_comma(&FluentValue::from(3), &()),
            Some("3".to_owned())
        );
        assert_eq!(decimal_comma(&FluentValue::from("a.b"), &()), None);
    }

    #[test]
    fn unknown_messages_come_back_as_their_id() {
        assert_eq!(Lang::En.get("no-such-message"), "no-such-message");
    }

    #[test]
    fn msg_keeps_plain_text_and_translates_keys() {
        let msg = Msg::new("login-wait-seconds").with("seconds", 5);
        assert_eq!(Lang::En.msg(&msg), "5 seconds");
        assert_eq!(msg.to_string(), "5 Sekunden");
        assert_eq!(Lang::En.msg(&Msg::Text("as is".to_owned())), "as is");
    }

    #[tokio::test]
    async fn the_request_language_can_be_changed_within_the_scope() {
        assert_eq!(current(), Lang::De);
        let seen = scope(Lang::En, async {
            let before = current();
            set_current(Lang::De);
            (before, current())
        })
        .await;
        assert_eq!(seen, (Lang::En, Lang::De));
    }
}
