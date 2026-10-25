//! Desktop apps from `.desktop` files (freedesktop Desktop Entry Specification).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::i18n::Language;
use crate::icons;
use crate::launch;
use crate::library::{AppEntry, AppKind, Artwork, Launch, Names, palette_for};
use crate::xdg;

/// Categories that make sense on a TV: games, media, browsers, images.
const WANTED_CATEGORIES: &[&str] = &["Game", "AudioVideo", "Audio", "Video", "Network", "Graphics"];
/// System tools, developer and office apps (e.g. mail, PDF) stay out, even if they have a
/// wanted category.
const EXCLUDED_CATEGORIES: &[&str] = &["Settings", "System", "Development", "Office", "ConsoleOnly"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DesktopEntry {
    pub id: String,
    pub names: DesktopNames,
    pub exec: Option<String>,
    pub try_exec: Option<String>,
    pub icon: Option<String>,
    pub categories: Vec<String>,
    pub entry_type: String,
    pub no_display: bool,
    pub hidden: bool,
    pub terminal: bool,
}

/// A localized key (`Name`, `Name[de]`, `Name[de_DE]`, …). Only the variants of the languages the
/// shell has (see [`Language::ALL`]) are kept, so a runtime language switch (Story 6.6) can pick
/// another variant without reading the file again, and the dozens of other translations a
/// `.desktop` file usually carries cost no memory.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LocalizedText {
    /// The unlocalized value
    pub default: Option<Box<str>>,
    /// (locale of the key, value), e.g. `("de_DE", "…")`
    pub variants: Vec<(Box<str>, Box<str>)>,
}

impl LocalizedText {
    /// The value for the first of `langs` that has a variant, otherwise the unlocalized one
    /// (like the old resolution at discovery time).
    pub fn get(&self, langs: &[String]) -> Option<&str> {
        langs
            .iter()
            .find_map(|lang| self.variants.iter().find(|(l, _)| **l == **lang))
            .map(|(_, v)| &**v)
            .or(self.default.as_deref())
    }

    fn is_empty(&self) -> bool {
        self.default.is_none() && self.variants.is_empty()
    }
}

/// The localizable texts of a desktop entry, kept on [`AppEntry`] (behind an `Arc`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DesktopNames {
    pub name: LocalizedText,
    pub generic_name: LocalizedText,
    pub comment: LocalizedText,
}

impl DesktopNames {
    /// Title and description for the locale codes `langs` (see [`crate::i18n::desktop_languages`]).
    /// A name that only exists in another language is still better than none.
    pub fn resolve(&self, langs: &[String]) -> (String, String) {
        let title = self.name.get(langs).or_else(|| self.name.variants.first().map(|(_, v)| &**v)).unwrap_or_default();
        let description = self.generic_name.get(langs).or_else(|| self.comment.get(langs)).unwrap_or_default();
        (title.to_string(), description.to_string())
    }
}

impl DesktopEntry {
    fn has_category(&self, wanted: &[&str]) -> bool {
        self.categories.iter().any(|c| wanted.contains(&c.as_str()))
    }

    fn is_console_friendly(&self) -> bool {
        self.entry_type == "Application"
            && !self.no_display
            && !self.hidden
            && !self.terminal
            && self.has_category(WANTED_CATEGORIES)
            && !self.has_category(EXCLUDED_CATEGORIES)
            // Steam creates its own shortcuts for games; those already come from the Steam source.
            && self.exec.as_deref().is_some_and(|e| !e.contains("steam://rungameid"))
            && self.try_exec.as_deref().is_none_or(launch::program_exists)
    }
}

/// Finds all matching desktop apps. `data_dirs` is sorted by priority (see [`xdg::data_dirs`]);
/// `langs` are the locale codes the titles are resolved for first ([`AppEntry::localize`] switches).
pub fn discover(data_dirs: &[PathBuf], langs: &[String]) -> Vec<AppEntry> {
    let theme = icon_theme();
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for dir in data_dirs {
        for (id, path) in desktop_files(&dir.join("applications")) {
            // The first location wins, even if it hides the app (Hidden=true in the user directory).
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(entry) = fs::read_to_string(&path).ok().and_then(|c| parse(&id, &c)) else {
                continue;
            };
            if entry.is_console_friendly() {
                entries.push(to_app_entry(entry, data_dirs, &theme, langs));
            }
        }
    }
    entries
}

fn to_app_entry(entry: DesktopEntry, data_dirs: &[PathBuf], theme: &str, langs: &[String]) -> AppEntry {
    let id = format!("desktop:{}", entry.id);
    let (accent, accent2) = palette_for(&id);
    let kind = if entry.has_category(&["Game"]) { AppKind::Game } else { AppKind::App };
    let (title, description) = entry.names.resolve(langs);
    AppEntry {
        title,
        description,
        names: Names::Desktop(Arc::new(entry.names)),
        kind,
        accent,
        accent2,
        launch: Launch::Command(exec_to_argv(entry.exec.as_deref().unwrap_or_default())),
        artwork: Artwork {
            icon: entry.icon.as_deref().and_then(|name| icons::find(name, theme, data_dirs)),
            ..Artwork::default()
        },
        last_used: None,
        id,
    }
}

/// All `.desktop` files below `root`. The ID of `kde/foo.desktop` is `kde-foo.desktop`.
fn desktop_files(root: &Path) -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let Some(name) = entry.file_name().to_str().map(str::to_string) else { continue };
            if path.is_dir() {
                walk(&path, &format!("{prefix}{name}-"), out);
            } else if name.ends_with(".desktop") {
                out.push((format!("{prefix}{name}"), path));
            }
        }
    }
    let mut files = Vec::new();
    walk(root, "", &mut files);
    files.sort();
    files
}

/// Parses the `[Desktop Entry]` group. Returns `None` if there is no name.
pub fn parse(id: &str, content: &str) -> Option<DesktopEntry> {
    let mut values: HashMap<&str, &str> = HashMap::new();
    let mut in_main_group = false;
    for line in content.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_main_group = line == "[Desktop Entry]";
            continue;
        }
        if in_main_group && let Some((key, value)) = line.split_once('=') {
            values.entry(key.trim()).or_insert(value.trim());
        }
    }

    let localized = |key: &str, clean: fn(String) -> String| {
        let mut text = LocalizedText {
            default: values.get(key).map(|v| clean(unescape(v)).into()),
            ..LocalizedText::default()
        };
        for (full, value) in &values {
            if let Some(locale) = full.strip_prefix(key).and_then(|k| k.strip_prefix('[')).and_then(|k| k.strip_suffix(']'))
                && is_supported_locale(locale)
            {
                text.variants.push((locale.into(), clean(unescape(value)).into()));
            }
        }
        // `values` is a HashMap; a stable order keeps entries comparable.
        text.variants.sort();
        text
    };
    let plain = |key: &str| values.get(key).map(|v| unescape(v));
    let flag = |key: &str| values.get(key).is_some_and(|v| *v == "true");

    let name = localized("Name", |v| v.trim().to_string());
    if name.is_empty() {
        return None;
    }
    Some(DesktopEntry {
        id: id.to_string(),
        names: DesktopNames {
            name,
            generic_name: localized("GenericName", |v| v),
            comment: localized("Comment", |v| v),
        },
        exec: plain("Exec"),
        try_exec: plain("TryExec"),
        icon: plain("Icon"),
        categories: values
            .get("Categories")
            .map(|v| v.split(';').filter(|c| !c.is_empty()).map(str::to_string).collect())
            .unwrap_or_default(),
        entry_type: plain("Type").unwrap_or_default(),
        no_display: flag("NoDisplay"),
        hidden: flag("Hidden"),
        terminal: flag("Terminal"),
    })
}

/// Whether a `.desktop` locale (`de`, `de_DE`, `de_DE@euro`, …) belongs to a language of the shell
fn is_supported_locale(locale: &str) -> bool {
    let language = locale.split(['_', '.', '@']).next().unwrap_or_default();
    Language::ALL.iter().any(|l| l.code() == language)
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Splits `Exec` into program and arguments and removes field codes like `%U` and Flatpak's `@@u … @@`.
pub fn exec_to_argv(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut quoted = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_token = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '%' => {
                if chars.next() == Some('%') {
                    current.push('%');
                }
                in_token = true;
            }
            c if c.is_whitespace() && !quoted => {
                if in_token {
                    args.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            c => {
                current.push(c);
                in_token = true;
            }
        }
    }
    if in_token {
        args.push(current);
    }
    args.retain(|a| !a.is_empty() && !a.starts_with("@@"));
    args
}

/// Icon theme from the KDE settings, otherwise `hicolor`.
fn icon_theme() -> String {
    xdg::config_home()
        .and_then(|dir| fs::read_to_string(dir.join("kdeglobals")).ok())
        .and_then(|content| ini_value(&content, "Icons", "Theme"))
        .unwrap_or_else(|| "hicolor".into())
}

fn ini_value(content: &str, section: &str, key: &str) -> Option<String> {
    let header = format!("[{section}]");
    let mut in_section = false;
    for line in content.lines().map(str::trim) {
        if line.starts_with('[') {
            in_section = line == header;
        } else if in_section && let Some((k, v)) = line.split_once('=') && k.trim() == key {
            return Some(v.trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = "\
[Desktop Entry]
Type=Application
Name=Firefox
Name[de]=Firefox-Browser
Name[en_GB]=Firefox
Name[fr]=Navigateur Firefox
GenericName=Web Browser
GenericName[de]=Webbrowser
Exec=firefox %u
Icon=firefox
Categories=Network;WebBrowser;

[Desktop Action new-window]
Name=New Window
Exec=firefox --new-window %u
";

    #[test]
    fn parses_main_group_with_locale() {
        let entry = parse("firefox.desktop", FIREFOX).unwrap();
        let (title, description) = entry.names.resolve(&["de_DE".into(), "de".into()]);
        assert_eq!((title.as_str(), description.as_str()), ("Firefox-Browser", "Webbrowser"));
        assert_eq!(entry.exec.as_deref(), Some("firefox %u"));
        assert_eq!(entry.categories, ["Network", "WebBrowser"]);
        assert!(entry.is_console_friendly());
    }

    #[test]
    fn falls_back_to_unlocalized_name() {
        let entry = parse("firefox.desktop", FIREFOX).unwrap();
        assert_eq!(entry.names.resolve(&["fr".into()]), ("Firefox".into(), "Web Browser".into()));
        assert_eq!(entry.names.resolve(&["en".into()]).0, "Firefox");
    }

    #[test]
    fn keeps_only_supported_languages() {
        let entry = parse("firefox.desktop", FIREFOX).unwrap();
        let locales: Vec<_> = entry.names.name.variants.iter().map(|(l, _)| &**l).collect();
        assert_eq!(locales, ["de", "en_GB"]);
        assert_eq!(entry.names.generic_name.variants.len(), 1);
        assert!(is_supported_locale("de_AT@euro"));
        assert!(!is_supported_locale("fr_FR"));
    }

    #[test]
    fn name_only_in_another_language_and_comment_fallback() {
        let content = "[Desktop Entry]\nType=Application\nName[de]=Nur Deutsch\nComment[de]=Kommentar\n";
        let entry = parse("x.desktop", content).unwrap();
        assert_eq!(entry.names.resolve(&["en".into()]), ("Nur Deutsch".into(), "".into()));
        assert_eq!(entry.names.resolve(&["de".into()]), ("Nur Deutsch".into(), "Kommentar".into()));
        assert!(parse("y.desktop", "[Desktop Entry]\nName[fr]=Seulement\n").is_none());
    }

    #[test]
    fn filters_unwanted_entries() {
        let base = parse("firefox.desktop", FIREFOX).unwrap();
        let check = |change: fn(&mut DesktopEntry)| {
            let mut entry = base.clone();
            change(&mut entry);
            entry.is_console_friendly()
        };
        assert!(!check(|e| e.no_display = true));
        assert!(!check(|e| e.terminal = true));
        assert!(!check(|e| e.categories = vec!["Office".into()]));
        assert!(!check(|e| e.categories.push("System".into())));
        assert!(!check(|e| e.exec = Some("steam steam://rungameid/730".into())));
        assert!(!check(|e| e.try_exec = Some("/does/not/exist".into())));
    }

    #[test]
    fn exec_field_codes_and_quotes() {
        assert_eq!(exec_to_argv("firefox %u"), ["firefox"]);
        assert_eq!(exec_to_argv(r#""/opt/My App/run" --title "a \"b\"" 100%%"#), ["/opt/My App/run", "--title", r#"a "b""#, "100%"]);
        assert_eq!(
            exec_to_argv("/usr/bin/flatpak run --command=vlc org.videolan.VLC @@u %U @@"),
            ["/usr/bin/flatpak", "run", "--command=vlc", "org.videolan.VLC"]
        );
    }

    #[test]
    fn unescapes_values() {
        assert_eq!(unescape(r"a\sb\\c"), r"a b\c");
    }

    #[test]
    fn reads_ini_value() {
        let ini = "[General]\nTheme=x\n[Icons]\nTheme=breeze-dark\n";
        assert_eq!(ini_value(ini, "Icons", "Theme").as_deref(), Some("breeze-dark"));
        assert_eq!(ini_value(ini, "Icons", "Other"), None);
    }
}
