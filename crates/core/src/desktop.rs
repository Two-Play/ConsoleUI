//! Desktop-Apps aus `.desktop`-Dateien (freedesktop Desktop Entry Specification).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::icons;
use crate::launch;
use crate::library::{AppEntry, AppKind, Artwork, Launch, palette_for};
use crate::xdg;

/// Kategorien, die auf dem Fernseher Sinn ergeben: Spiele, Medien, Browser, Bilder.
const WANTED_CATEGORIES: &[&str] = &["Game", "AudioVideo", "Audio", "Video", "Network", "Graphics"];
/// Systemtools, Entwickler- und Büro-Apps (z. B. Mail, PDF) bleiben draußen, auch wenn sie eine
/// gewünschte Kategorie haben.
const EXCLUDED_CATEGORIES: &[&str] = &["Settings", "System", "Development", "Office", "ConsoleOnly"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DesktopEntry {
    pub id: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    pub exec: Option<String>,
    pub try_exec: Option<String>,
    pub icon: Option<String>,
    pub categories: Vec<String>,
    pub entry_type: String,
    pub no_display: bool,
    pub hidden: bool,
    pub terminal: bool,
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
            // Steam legt für Spiele eigene Verknüpfungen an; die kommen schon über die Steam-Quelle.
            && self.exec.as_deref().is_some_and(|e| !e.contains("steam://rungameid"))
            && self.try_exec.as_deref().is_none_or(launch::program_exists)
    }
}

/// Findet alle passenden Desktop-Apps. `data_dirs` ist nach Priorität sortiert (siehe [`xdg::data_dirs`]).
pub fn discover(data_dirs: &[PathBuf], langs: &[String]) -> Vec<AppEntry> {
    let theme = icon_theme();
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for dir in data_dirs {
        for (id, path) in desktop_files(&dir.join("applications")) {
            // Die erste Fundstelle gewinnt, auch wenn sie die App versteckt (Hidden=true im Benutzerordner).
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(entry) = fs::read_to_string(&path).ok().and_then(|c| parse(&id, &c, langs)) else {
                continue;
            };
            if entry.is_console_friendly() {
                entries.push(to_app_entry(entry, data_dirs, &theme));
            }
        }
    }
    entries
}

fn to_app_entry(entry: DesktopEntry, data_dirs: &[PathBuf], theme: &str) -> AppEntry {
    let id = format!("desktop:{}", entry.id);
    let (accent, accent2) = palette_for(&id);
    let kind = if entry.has_category(&["Game"]) { AppKind::Game } else { AppKind::App };
    AppEntry {
        title: entry.name,
        description: entry.generic_name.or(entry.comment).unwrap_or_default(),
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

/// Alle `.desktop`-Dateien unterhalb von `root`. Die ID von `kde/foo.desktop` ist `kde-foo.desktop`.
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

/// Parst die Gruppe `[Desktop Entry]`. Gibt `None` zurück, wenn kein Name vorhanden ist.
pub fn parse(id: &str, content: &str, langs: &[String]) -> Option<DesktopEntry> {
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

    let localized = |key: &str| {
        langs
            .iter()
            .find_map(|lang| values.get(format!("{key}[{lang}]").as_str()))
            .or_else(|| values.get(key))
            .map(|v| unescape(v))
    };
    let plain = |key: &str| values.get(key).map(|v| unescape(v));
    let flag = |key: &str| values.get(key).is_some_and(|v| *v == "true");

    Some(DesktopEntry {
        id: id.to_string(),
        name: localized("Name")?.trim().to_string(),
        generic_name: localized("GenericName"),
        comment: localized("Comment"),
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

/// Zerlegt `Exec` in Programm und Argumente und entfernt Feldcodes wie `%U` sowie Flatpaks `@@u … @@`.
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

/// Icon-Theme aus den KDE-Einstellungen, sonst `hicolor`.
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
GenericName=Web Browser
Exec=firefox %u
Icon=firefox
Categories=Network;WebBrowser;

[Desktop Action new-window]
Name=New Window
Exec=firefox --new-window %u
";

    #[test]
    fn parses_main_group_with_locale() {
        let entry = parse("firefox.desktop", FIREFOX, &["de_DE".into(), "de".into()]).unwrap();
        assert_eq!(entry.name, "Firefox-Browser");
        assert_eq!(entry.exec.as_deref(), Some("firefox %u"));
        assert_eq!(entry.categories, ["Network", "WebBrowser"]);
        assert!(entry.is_console_friendly());
    }

    #[test]
    fn falls_back_to_unlocalized_name() {
        let entry = parse("firefox.desktop", FIREFOX, &["fr".into()]).unwrap();
        assert_eq!(entry.name, "Firefox");
    }

    #[test]
    fn filters_unwanted_entries() {
        let base = parse("firefox.desktop", FIREFOX, &[]).unwrap();
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
