//! Translations (Story 6.6). English is the source language; German comes from the gettext catalog
//! `lang/de/LC_MESSAGES/consoleui.po` at the repository root. Slint bundles the same file for
//! `@tr()` in the `.slint` files (see `crates/shell/build.rs`), and this module embeds it for the
//! strings built in Rust, so both sides share one catalog and nothing has to be installed.
//!
//! - [`tr`] for a fixed string, [`trf`] with format arguments, [`trn`] with a plural.
//! - Placeholders follow Slint's `@tr()` syntax: `{}` takes the next argument, `{0}`/`{1}` a
//!   specific one, `{n}` is the count of a plural, `{{`/`}}` are literal braces. Never concatenate
//!   translated fragments; put the whole sentence into one message.
//! - [`tr_noop`] marks a string for the catalog where it is stored untranslated (e.g. in a
//!   `const` table) and translated later with `tr(value)`.
//!
//! The test `catalog_matches_sources` scans all `.rs` and `.slint` files for these calls and fails
//! if a message has no German translation or the catalog has a message nobody uses any more.

use std::collections::HashMap;
use std::fmt::{Display, Write};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

/// A language of the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    English,
    German,
}

impl Language {
    pub const ALL: [Language; 2] = [Self::English, Self::German];

    /// Code of the language, also the directory name of its catalog (as Slint expects it in
    /// `select_bundled_translation`).
    pub const fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::German => "de",
        }
    }

    /// German for every locale that starts with `de` (`de_DE.UTF-8`, `de_AT`, `de`), else English.
    pub fn from_locale(locale: &str) -> Self {
        if locale.starts_with("de") { Self::German } else { Self::English }
    }

    /// Language from the environment, in gettext order: `LANGUAGE` (first entry of the list),
    /// `LC_ALL`, `LC_MESSAGES`, `LANG`; the first one that is set decides.
    pub fn from_env() -> Self {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|name| var(name))
            .find_map(|value| value.split(':').next().filter(|first| !first.is_empty()).map(str::to_string))
            .map_or(Self::English, |locale| Self::from_locale(&locale))
    }
}

/// The language setting of the settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LanguageChoice {
    /// Follows the environment ([`Language::from_env`])
    #[default]
    Auto,
    English,
    German,
}

impl LanguageChoice {
    pub const ALL: [LanguageChoice; 3] = [Self::Auto, Self::English, Self::German];

    pub fn resolve(self) -> Language {
        match self {
            Self::Auto => Language::from_env(),
            Self::English => Language::English,
            Self::German => Language::German,
        }
    }

    /// Language names stay in their own language, so everyone finds theirs.
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => tr("Automatic"),
            Self::English => "English",
            Self::German => "Deutsch",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Sets the language for all following [`tr`] calls (any thread). The shell also has to select
/// the Slint translation and rebuild the texts it already handed to the UI.
pub fn set_language(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn language() -> Language {
    if CURRENT.load(Ordering::Relaxed) == Language::German as u8 { Language::German } else { Language::English }
}

/// Locale codes for localized `.desktop` keys (`Name[de]`): the environment's, as long as they
/// belong to `language`, otherwise just the language code.
pub fn desktop_languages(language: Language, env: Vec<String>) -> Vec<String> {
    if env.first().is_some_and(|first| first.starts_with(language.code())) {
        env
    } else {
        vec![language.code().to_string()]
    }
}

/// Translates `msgid` into the current language.
pub fn tr(msgid: &str) -> &str {
    lookup(language(), msgid)
}

/// Translates `msgid` into `language` instead of the current one (e.g. for texts that are built
/// for a language before it is selected, or in tests).
pub fn tr_in(language: Language, msgid: &str) -> &str {
    lookup(language, msgid)
}

/// Translates `msgid` and fills its placeholders (`{}`, `{0}`, …).
pub fn trf(msgid: &str, args: &[&dyn Display]) -> String {
    format(tr(msgid), args, None)
}

/// Translates a message with a plural form; `{n}` is replaced by `n`, other placeholders by `args`.
pub fn trn(singular: &str, plural: &str, n: usize, args: &[&dyn Display]) -> String {
    format(lookup_plural(language(), singular, plural, n), args, Some(n))
}

/// Marks `msgid` for the catalog without translating it (translate it later with [`tr`]).
pub const fn tr_noop(msgid: &str) -> &str {
    msgid
}

const GERMAN_PO: &str = include_str!("../../../lang/de/LC_MESSAGES/consoleui.po");

static GERMAN: LazyLock<Catalog> =
    LazyLock::new(|| Catalog::parse(GERMAN_PO).unwrap_or_else(|err| panic!("German catalog is invalid: {err}")));

fn lookup(language: Language, msgid: &str) -> &str {
    match language {
        Language::English => msgid,
        Language::German => GERMAN.get(msgid).and_then(|m| m.translations.first()).map_or(msgid, String::as_str),
    }
}

fn lookup_plural<'a>(language: Language, singular: &'a str, plural: &'a str, n: usize) -> &'a str {
    // German and English share the rule `n != 1`; the test checks the catalog header.
    let form = usize::from(n != 1);
    let english = if form == 0 { singular } else { plural };
    match language {
        Language::English => english,
        Language::German => GERMAN.get(singular).and_then(|m| m.translations.get(form)).map_or(english, String::as_str),
    }
}

/// Fills `{}`, `{0}`, `{n}` like Slint's `@tr()`; unknown placeholders stay as they are.
fn format(template: &str, args: &[&dyn Display], n: Option<usize>) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut rest = template;
    while let Some(pos) = rest.find(['{', '}']) {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        let Some(end) = tail.find('}').filter(|_| tail.starts_with('{')) else {
            out.push_str(&tail[..1]);
            rest = &tail[1..];
            continue;
        };
        let name = tail[1..end].trim();
        let value: Option<&dyn Display> = if name.is_empty() {
            next += 1;
            args.get(next - 1).copied()
        } else if let Ok(index) = name.parse::<usize>() {
            args.get(index).copied()
        } else {
            None
        };
        match (value, name, n) {
            (Some(value), _, _) => {
                let _ = write!(out, "{value}");
            }
            (None, "n", Some(n)) => {
                let _ = write!(out, "{n}");
            }
            _ => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// One message of a catalog
#[derive(Debug, Clone, PartialEq, Eq)]
struct Message {
    plural: Option<String>,
    /// `msgstr`, or `msgstr[0]`, `msgstr[1]`, … for plurals
    translations: Vec<String>,
}

/// A gettext `.po` file, as far as this project uses it: `msgid`, `msgid_plural`, `msgstr`,
/// `msgstr[i]`, strings over several lines and comments. Contexts are not used.
#[derive(Debug, Default)]
struct Catalog {
    header: String,
    messages: HashMap<String, Message>,
}

impl Catalog {
    fn get(&self, msgid: &str) -> Option<&Message> {
        self.messages.get(msgid).filter(|m| m.translations.iter().all(|t| !t.is_empty()))
    }

    fn parse(content: &str) -> Result<Self, String> {
        #[derive(PartialEq)]
        enum Field {
            None,
            Id,
            Plural,
            Str(usize),
        }
        let mut catalog = Catalog::default();
        let mut id: Option<String> = None;
        let mut plural = None;
        let mut translations: Vec<String> = Vec::new();
        let mut field = Field::None;
        let mut finish = |id: &mut Option<String>, plural: &mut Option<String>, translations: &mut Vec<String>| {
            if let Some(id) = id.take() {
                let message = Message { plural: plural.take(), translations: std::mem::take(translations) };
                if id.is_empty() {
                    catalog.header = message.translations.concat();
                } else if catalog.messages.insert(id.clone(), message).is_some() {
                    return Err(format!("duplicate msgid {id:?}"));
                }
            }
            Ok(())
        };
        for (number, line) in content.lines().enumerate() {
            let line = line.trim();
            let error = |what: &str| format!("line {}: {what}", number + 1);
            if line.is_empty() || line.starts_with('#') {
                if line.starts_with("#~") {
                    return Err(error("obsolete entries (#~) are not allowed"));
                }
                continue;
            }
            let (keyword, rest) = match line.find('"') {
                Some(pos) => (line[..pos].trim(), &line[pos..]),
                None => return Err(error("expected a string")),
            };
            let text = unquote(rest).ok_or_else(|| error("broken string"))?;
            match keyword {
                "" => match field {
                    Field::Id => id.get_or_insert_default().push_str(&text),
                    Field::Plural => plural.get_or_insert_with(String::new).push_str(&text),
                    Field::Str(i) => translations[i].push_str(&text),
                    Field::None => return Err(error("string without keyword")),
                },
                "msgctxt" => return Err(error("msgctxt is not used in this project")),
                "msgid" => {
                    finish(&mut id, &mut plural, &mut translations).map_err(|e| error(&e))?;
                    id = Some(text);
                    field = Field::Id;
                }
                "msgid_plural" => {
                    plural = Some(text);
                    field = Field::Plural;
                }
                "msgstr" => {
                    translations = vec![text];
                    field = Field::Str(0);
                }
                other => {
                    let index = other
                        .strip_prefix("msgstr[")
                        .and_then(|s| s.strip_suffix(']'))
                        .and_then(|s| s.parse::<usize>().ok())
                        .filter(|&i| i == translations.len())
                        .ok_or_else(|| error(&format!("unexpected keyword {other:?}")))?;
                    translations.push(text);
                    field = Field::Str(index);
                }
            }
        }
        finish(&mut id, &mut plural, &mut translations)?;
        Ok(catalog)
    }
}

/// Content of a `"…"` string in a `.po` file
fn unquote(quoted: &str) -> Option<String> {
    let inner = quoted.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(match chars.next()? {
                'n' => '\n',
                't' => '\t',
                other => other,
            }),
            '"' => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    #[test]
    fn language_from_environment() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
        };
        assert_eq!(Language::from_vars(env(&[("LANG", "de_DE.UTF-8")])), Language::German);
        assert_eq!(Language::from_vars(env(&[("LANG", "en_US.UTF-8")])), Language::English);
        assert_eq!(Language::from_vars(env(&[])), Language::English);
        assert_eq!(Language::from_vars(env(&[("LANG", "de_DE.UTF-8"), ("LC_ALL", "C")])), Language::English);
        assert_eq!(Language::from_vars(env(&[("LANG", "en_US"), ("LANGUAGE", "de_AT:en")])), Language::German);
        assert_eq!(Language::from_vars(env(&[("LANGUAGE", ""), ("LC_MESSAGES", "de")])), Language::German);
    }

    #[test]
    fn desktop_languages_follow_the_choice() {
        let env = vec!["de_DE".to_string(), "de".to_string()];
        assert_eq!(desktop_languages(Language::German, env.clone()), env);
        assert_eq!(desktop_languages(Language::English, env), ["en"]);
        assert_eq!(desktop_languages(Language::German, Vec::new()), ["de"]);
    }

    #[test]
    fn formats_like_slint() {
        let name = "Pad";
        assert_eq!(format("Connected to {}", &[&name], None), "Connected to Pad");
        assert_eq!(format("{1}: {0}", &[&"a", &"b"], None), "b: a");
        assert_eq!(format("{n} devices {{x}}", &[], Some(3)), "3 devices {x}");
        assert_eq!(format("{} and {}", &[&1], None), "1 and {}");
        assert_eq!(format("unbalanced { and }", &[], None), "unbalanced { and }");
    }

    #[test]
    fn looks_up_german() {
        assert_eq!(lookup(Language::German, "Automatic"), "Automatisch");
        assert_eq!(lookup(Language::English, "Automatic"), "Automatic");
        assert_eq!(lookup(Language::German, "not in the catalog"), "not in the catalog");
        let one = "{n} device connected";
        let many = "{n} devices connected";
        assert_eq!(format(lookup_plural(Language::German, one, many, 1), &[], Some(1)), "1 Gerät verbunden");
        assert_eq!(format(lookup_plural(Language::German, one, many, 3), &[], Some(3)), "3 Geräte verbunden");
        assert_eq!(format(lookup_plural(Language::English, one, many, 3), &[], Some(3)), "3 devices connected");
    }

    #[test]
    fn parses_po_files() {
        let po = "# comment\nmsgid \"\"\nmsgstr \"Plural-Forms: nplurals=2;\\n\"\n\n\
                  msgid \"a \\\"b\\\"\"\nmsgstr \"\"\n\"c\"\n\n\
                  msgid \"{n} x\"\nmsgid_plural \"{n} xs\"\nmsgstr[0] \"{n} y\"\nmsgstr[1] \"{n} ys\"\n";
        let catalog = Catalog::parse(po).unwrap();
        assert_eq!(catalog.header, "Plural-Forms: nplurals=2;\n");
        assert_eq!(catalog.get("a \"b\"").unwrap().translations, ["c"]);
        let plural = catalog.get("{n} x").unwrap();
        assert_eq!((plural.plural.as_deref(), plural.translations.len()), (Some("{n} xs"), 2));
        assert!(Catalog::parse("msgid \"a\"\nmsgstr \"b\"\nmsgid \"a\"\nmsgstr \"c\"\n").is_err());
        assert!(Catalog::parse("msgctxt \"x\"\nmsgid \"a\"\nmsgstr \"b\"\n").is_err());
    }

    /// A translatable message found in the sources
    #[derive(Debug, PartialEq)]
    struct Found {
        plural: Option<String>,
        place: String,
    }

    /// Reads a Rust or Slint string literal at the start of `s`; returns its content and the rest.
    fn literal(s: &str) -> Option<(String, &str)> {
        let s = s.trim_start();
        let inner = s.strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = inner.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return Some((out, &inner[i + 1..])),
                '\\' => match chars.next()?.1 {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'u' => {
                        let start = i + 3;
                        let end = start + inner[start..].find('}')?;
                        out.push(char::from_u32(u32::from_str_radix(&inner[start..end], 16).ok()?)?);
                        while chars.next()?.0 < end {}
                    }
                    // Line continuation in Rust: skip the line break and the indentation
                    '\n' => {
                        let rest = &inner[i + 2..];
                        let skip = rest.len() - rest.trim_start().len();
                        for _ in 0..rest[..skip].chars().count() {
                            chars.next();
                        }
                    }
                    other => out.push(other),
                },
                c => out.push(c),
            }
        }
        None
    }

    /// All `tr("…")`, `trf("…"`, `trn("…", "…"`, `tr_noop("…")` in Rust and `@tr("…" | "…" % n)`
    /// in Slint.
    fn scan(path: &Path, content: &str, found: &mut BTreeMap<String, Found>) {
        let slint = path.extension().is_some_and(|e| e == "slint");
        let calls: &[&str] = if slint { &["@tr("] } else { &["tr(", "trf(", "trn(", "tr_noop("] };
        for call in calls {
            for (pos, _) in content.match_indices(call) {
                let before = content[..pos].chars().next_back();
                if !slint && before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let line = content[..pos].lines().count();
                let place = format!("{}:{line}", path.display());
                let Some((msgid, rest)) = literal(&content[pos + call.len()..]) else {
                    // A variable, e.g. `tr(label)`: its value is marked elsewhere.
                    continue;
                };
                assert!(!rest.trim_start().starts_with("=>"), "{place}: translation contexts are not used");
                let plural = if slint {
                    rest.trim_start().strip_prefix('|').and_then(literal).map(|(p, _)| p)
                } else if *call == "trn(" {
                    let rest = rest.trim_start().strip_prefix(',').unwrap_or_else(|| panic!("{place}: trn needs a plural"));
                    Some(literal(rest).unwrap_or_else(|| panic!("{place}: plural must be a literal")).0)
                } else {
                    None
                };
                if let Some(previous) = found.get(&msgid) {
                    assert_eq!(previous.plural, plural, "{place}: {msgid:?} is used with different plurals");
                }
                found.insert(msgid, Found { plural, place });
            }
        }
    }

    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs" || e == "slint") && !path.ends_with("core/src/i18n.rs") {
                out.push(path);
            }
        }
    }

    fn placeholders(text: &str) -> Vec<String> {
        let mut found: Vec<String> =
            text.split('{').skip(1).filter_map(|s| s.split_once('}')).map(|(name, _)| name.to_string()).collect();
        found.sort();
        found
    }

    /// Every translatable string has a German translation with the same placeholders, and the
    /// catalog has no stale messages.
    #[test]
    fn catalog_matches_sources() {
        let catalog = Catalog::parse(GERMAN_PO).unwrap();
        assert!(catalog.header.contains("plural=(n != 1);"), "German must use the plural rule n != 1");
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut files = Vec::new();
        for name in ["core/src", "core/examples", "shell/src", "shell/ui", "shell/examples"] {
            if crates.join(name).is_dir() {
                sources(&crates.join(name), &mut files);
            }
        }
        let mut found = BTreeMap::new();
        for file in &files {
            scan(file, &std::fs::read_to_string(file).unwrap(), &mut found);
        }
        // Used here in i18n.rs itself
        found.insert("Automatic".into(), Found { plural: None, place: "core/src/i18n.rs".into() });
        assert!(found.len() > 100, "the scan found only {} messages", found.len());

        let mut problems = Vec::new();
        for (msgid, place) in &found {
            match catalog.messages.get(msgid) {
                None => problems.push(format!("missing: {msgid:?} ({})", place.place)),
                Some(message) => {
                    let forms = if place.plural.is_some() { 2 } else { 1 };
                    if message.plural != place.plural {
                        problems.push(format!("plural differs: {msgid:?} ({})", place.place));
                    }
                    if message.translations.len() != forms || message.translations.iter().any(String::is_empty) {
                        problems.push(format!("untranslated: {msgid:?}"));
                    }
                    let expected = placeholders(msgid);
                    for translation in &message.translations {
                        let mut got = placeholders(translation);
                        if place.plural.is_some() {
                            got.retain(|p| p != "n");
                        }
                        let mut want = expected.clone();
                        want.retain(|p| p != "n");
                        if got != want {
                            problems.push(format!("placeholders differ: {msgid:?} -> {translation:?}"));
                        }
                    }
                }
            }
        }
        for msgid in catalog.messages.keys() {
            if !found.contains_key(msgid) {
                problems.push(format!("stale (not used any more): {msgid:?}"));
            }
        }
        assert!(problems.is_empty(), "catalog lang/de/LC_MESSAGES/consoleui.po:\n{}", problems.join("\n"));
    }
}
