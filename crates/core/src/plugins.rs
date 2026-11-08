//! Plugins (Epic 9, contract in `docs/plugins.md`): programs of their own that add library entries
//! and quick menu panels over the session bus. This is the part without the bus: finding them,
//! their entries, the panel and its input, and which ones the user turned on.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Action;
use crate::desktop::{self, exec_to_argv};
use crate::i18n::tr;
use crate::icons;
use crate::library::{AppEntry, AppKind, Artwork, Launch, Names, palette_for};

pub const INTERFACE: &str = "org.consoleui.Plugin1";
pub const PATH: &str = "/org/consoleui/Plugin1";
/// The protocol version the shell speaks
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provides {
    Library,
    Panel,
    /// Its own settings page (`Settings()`), opened from the plugin list
    Settings,
    /// Cover, background and logo for games without them (`Artwork(id, title)`)
    Artwork,
    /// Theme music for games (`Theme(id, title)`), asked before YouTube
    Themes,
}

impl Provides {
    pub fn label(self) -> &'static str {
        match self {
            Self::Library => tr("Adds games and starts them"),
            Self::Panel => tr("Controls in the quick menu"),
            Self::Settings => tr("Own settings"),
            Self::Artwork => tr("Pictures for games"),
            Self::Themes => tr("Music for games"),
        }
    }
}

/// An installed plugin, from its desktop entry.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginInfo {
    /// The desktop entry's ID without `.desktop`; also the key in the profile's list
    pub id: String,
    pub bus_name: String,
    pub name: String,
    pub comment: String,
    pub exec: Vec<String>,
    pub provides: Vec<Provides>,
}

impl PluginInfo {
    pub fn provides(&self, what: Provides) -> bool {
        self.provides.contains(&what)
    }

    /// "Adds games and starts them · Controls in the quick menu"
    pub fn summary(&self) -> String {
        self.provides.iter().map(|p| p.label()).collect::<Vec<_>>().join(" · ")
    }
}

/// The plugins installed in `data_dirs` (see [`crate::xdg::data_dirs`]), sorted by name.
pub fn discover(data_dirs: &[PathBuf], langs: &[String]) -> Vec<PluginInfo> {
    let mut plugins: Vec<PluginInfo> = desktop::all_entries(data_dirs)
        .into_iter()
        .filter(|entry| !entry.hidden)
        .filter_map(|entry| {
            let bus_name = entry.plugin.clone()?;
            let provides = entry
                .plugin_provides
                .iter()
                .filter_map(|what| match what.as_str() {
                    "library" => Some(Provides::Library),
                    "panel" => Some(Provides::Panel),
                    "settings" => Some(Provides::Settings),
                    "artwork" => Some(Provides::Artwork),
                    "themes" => Some(Provides::Themes),
                    _ => None,
                })
                .collect();
            let (name, comment) = entry.names.resolve(langs);
            let id = entry.id.strip_suffix(".desktop").unwrap_or(&entry.id).to_string();
            Some(PluginInfo {
                id,
                bus_name,
                name,
                comment,
                exec: exec_to_argv(entry.exec.as_deref().unwrap_or_default()),
                provides,
            })
        })
        .collect();
    plugins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    plugins
}

/// Pictures a plugin found for an entry (absolute paths or icon names, as in [`PluginEntry`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginArtwork {
    pub cover: Option<String>,
    pub hero: Option<String>,
    pub logo: Option<String>,
}

impl PluginArtwork {
    pub fn is_empty(&self) -> bool {
        self.cover.is_none() && self.hero.is_none() && self.logo.is_none()
    }

    /// Fills in what `artwork` lacks, with files the host can read.
    pub fn fill(&self, artwork: &mut Artwork, data_dirs: &[PathBuf]) {
        let theme = desktop::current_icon_theme();
        let resolve = |value: &Option<String>| {
            let value = value.as_deref().filter(|v| !v.is_empty())?;
            if value.starts_with('/') {
                Some(PathBuf::from(value)).filter(|path| path.is_file())
            } else {
                icons::find(value, &theme, data_dirs)
            }
        };
        if artwork.cover.is_none() {
            artwork.cover = resolve(&self.cover);
        }
        if artwork.hero.is_none() {
            artwork.hero = resolve(&self.hero);
        }
        if artwork.logo.is_none() {
            artwork.logo = resolve(&self.logo);
        }
    }
}

/// A library entry of a plugin, as it came over the bus.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginEntry {
    pub id: String,
    pub title: String,
    pub kind: AppKind,
    pub command: Vec<String>,
    pub cover: Option<String>,
    pub hero: Option<String>,
    pub logo: Option<String>,
    pub icon: Option<String>,
    pub last_played: Option<u64>,
}

/// `plugin:<plugin>:<entry>`, the ID in the shell's lists and history.
pub fn entry_id(plugin: &str, entry: &str) -> String {
    format!("plugin:{plugin}:{entry}")
}

/// Whether `id` is an entry of `plugin`.
pub fn is_entry_of(id: &str, plugin: &str) -> bool {
    id.strip_prefix("plugin:").and_then(|rest| rest.strip_prefix(plugin)).is_some_and(|rest| rest.starts_with(':'))
}

impl PluginEntry {
    /// `None` for an entry the shell cannot use (no title or nothing to start).
    pub fn into_app_entry(self, plugin: &PluginInfo, data_dirs: &[PathBuf]) -> Option<AppEntry> {
        if self.id.is_empty() || self.title.trim().is_empty() || self.command.is_empty() {
            return None;
        }
        let theme = desktop::current_icon_theme();
        let art = |value: &Option<String>| {
            let value = value.as_deref().filter(|v| !v.is_empty())?;
            if value.starts_with('/') {
                Some(PathBuf::from(value)).filter(|path| path.is_file())
            } else {
                icons::find(value, &theme, data_dirs)
            }
        };
        let id = entry_id(&plugin.id, &self.id);
        let (accent, accent2) = palette_for(&id);
        Some(AppEntry {
            artwork: Artwork {
                cover: art(&self.cover),
                hero: art(&self.hero),
                logo: art(&self.logo),
                icon: art(&self.icon),
            },
            id,
            title: self.title.trim().to_string(),
            description: plugin.name.clone(),
            names: Names::Fixed,
            kind: self.kind,
            accent,
            accent2,
            launch: Launch::Command(self.command),
            last_used: self.last_played.filter(|&t| t > 0),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Number(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    Toggle,
    Slider { min: f64, max: f64, step: f64, unit: String },
    Choice(Vec<String>),
    Button,
    Info,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelItem {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub kind: ItemKind,
    pub value: Value,
}

impl PanelItem {
    /// What ◀ ▶ makes of it: a toggle flips, a slider steps, a choice moves on; `None` for buttons
    /// and info lines.
    pub fn change(&self, forward: bool) -> Option<Value> {
        match (&self.kind, &self.value) {
            (ItemKind::Toggle, Value::Bool(on)) => Some(Value::Bool(!on)),
            (ItemKind::Slider { min, max, step, .. }, Value::Number(value)) => {
                let step = if *step > 0.0 { *step } else { 1.0 };
                let next = if forward { value + step } else { value - step };
                let next = next.clamp(min.min(*max), max.max(*min));
                (next != *value).then_some(Value::Number(next))
            }
            (ItemKind::Choice(choices), Value::Text(current)) if !choices.is_empty() => {
                let at = choices.iter().position(|c| c == current).unwrap_or(0);
                let next = if forward { (at + 1) % choices.len() } else { (at + choices.len() - 1) % choices.len() };
                Some(Value::Text(choices[next].clone()))
            }
            _ => None,
        }
    }

    /// What ✕ does: a button is pressed, everything else changes as with ▶.
    pub fn confirm(&self) -> Option<Value> {
        match self.kind {
            ItemKind::Button => Some(Value::Bool(true)),
            _ => self.change(true),
        }
    }

    /// The value as the row shows it.
    pub fn value_label(&self) -> String {
        match (&self.kind, &self.value) {
            (ItemKind::Toggle, Value::Bool(on)) => if *on { tr("On") } else { tr("Off") }.to_string(),
            (ItemKind::Slider { step, unit, .. }, Value::Number(value)) => {
                let digits = if step.fract() == 0.0 { 0 } else { 1 };
                format!("{value:.digits$}{unit}")
            }
            (ItemKind::Button, _) => String::new(),
            (_, Value::Text(text)) => text.clone(),
            (_, Value::Number(value)) => value.to_string(),
            (_, Value::Bool(on)) => if *on { tr("On") } else { tr("Off") }.to_string(),
        }
    }

    /// The row has a value that ◀ ▶ changes.
    pub fn adjustable(&self) -> bool {
        matches!(self.kind, ItemKind::Toggle | ItemKind::Slider { .. } | ItemKind::Choice(_))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Panel {
    pub title: String,
    pub items: Vec<PanelItem>,
}

/// What an input on an open panel leads to.
#[derive(Debug, Clone, PartialEq)]
pub enum PanelOutcome {
    Nothing,
    Moved,
    /// Tell the plugin; the panel already shows the new value
    Activate {
        item: String,
        value: Value,
    },
    Close,
}

/// The open panel: what the plugin sent last and where the focus is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PanelView {
    pub panel: Panel,
    pub focused: usize,
    /// No answer from the plugin yet
    pub loading: bool,
}

impl PanelView {
    pub fn loading(title: &str) -> Self {
        Self { panel: Panel { title: title.to_string(), items: Vec::new() }, focused: 0, loading: true }
    }

    /// A new panel from the plugin; the focus stays on the same item if it is still there.
    pub fn set(&mut self, panel: Panel) {
        let current = self.panel.items.get(self.focused).map(|item| item.id.clone());
        self.focused = current
            .and_then(|id| panel.items.iter().position(|item| item.id == id))
            .unwrap_or(0)
            .min(panel.items.len().saturating_sub(1));
        self.panel = panel;
        self.loading = false;
    }

    pub fn apply(&mut self, action: Action) -> PanelOutcome {
        let len = self.panel.items.len();
        match action {
            Action::Up if self.focused > 0 => {
                self.focused -= 1;
                PanelOutcome::Moved
            }
            Action::Down if self.focused + 1 < len => {
                self.focused += 1;
                PanelOutcome::Moved
            }
            Action::Left | Action::Right | Action::Confirm => {
                let Some(item) = self.panel.items.get_mut(self.focused) else { return PanelOutcome::Nothing };
                let value = match action {
                    Action::Confirm => item.confirm(),
                    _ => item.change(action == Action::Right),
                };
                let Some(value) = value else { return PanelOutcome::Nothing };
                if item.kind != ItemKind::Button {
                    item.value = value.clone();
                }
                PanelOutcome::Activate { item: item.id.clone(), value }
            }
            Action::Back | Action::Guide | Action::Menu => PanelOutcome::Close,
            _ => PanelOutcome::Nothing,
        }
    }
}

/// The plugins the user turned on, per profile (`plugins.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSettings {
    #[serde(default)]
    pub enabled: Vec<String>,
}

impl PluginSettings {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(io::Error::other)?)
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled.iter().any(|enabled| enabled == id)
    }

    pub fn set(&mut self, id: &str, on: bool) {
        self.enabled.retain(|enabled| enabled != id);
        if on {
            self.enabled.push(id.to_string());
        }
    }

    /// The installed plugins that are on.
    pub fn active<'a>(&self, plugins: &'a [PluginInfo]) -> Vec<&'a PluginInfo> {
        let on: HashSet<&str> = self.enabled.iter().map(String::as_str).collect();
        plugins.iter().filter(|plugin| on.contains(plugin.id.as_str())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin() -> PluginInfo {
        PluginInfo {
            id: "consoleui-plugin-lutris".into(),
            bus_name: "org.consoleui.plugins.Lutris".into(),
            name: "Lutris".into(),
            comment: String::new(),
            exec: vec!["consoleui-plugin-lutris".into()],
            provides: vec![Provides::Library],
        }
    }

    #[test]
    fn finds_plugins_and_keeps_them_out_of_the_apps() {
        let dir = std::env::temp_dir().join(format!("consoleui-plugins-{}", std::process::id()));
        let apps = dir.join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            apps.join("consoleui-plugin-lutris.desktop"),
            "[Desktop Entry]\nType=Application\nName=Lutris\nExec=consoleui-plugin-lutris --x\nNoDisplay=true\n\
             X-ConsoleUI=true\nX-ConsoleUI-Plugin=org.consoleui.plugins.Lutris\nX-ConsoleUI-Plugin-Provides=library;panel;odd;\n",
        )
        .unwrap();
        let found = discover(std::slice::from_ref(&dir), &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "consoleui-plugin-lutris");
        assert_eq!(found[0].bus_name, "org.consoleui.plugins.Lutris");
        assert_eq!(found[0].exec, ["consoleui-plugin-lutris", "--x"]);
        assert_eq!(found[0].provides, [Provides::Library, Provides::Panel]);
        assert!(desktop::discover(std::slice::from_ref(&dir), &[]).is_empty(), "a plugin is no app");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn entries_get_a_prefixed_id_and_need_a_command() {
        let entry = PluginEntry {
            id: "3".into(),
            title: " EA App ".into(),
            command: vec!["lutris".into(), "lutris:rungameid/3".into()],
            last_played: Some(0),
            ..PluginEntry::default()
        };
        let app = entry.clone().into_app_entry(&plugin(), &[]).unwrap();
        assert_eq!(app.id, "plugin:consoleui-plugin-lutris:3");
        assert_eq!(app.title, "EA App");
        assert_eq!(app.description, "Lutris");
        assert_eq!(app.last_used, None);
        assert!(is_entry_of(&app.id, "consoleui-plugin-lutris"));
        assert!(!is_entry_of(&app.id, "consoleui-plugin-lutr"));
        let nothing = PluginEntry { command: Vec::new(), ..entry };
        assert!(nothing.into_app_entry(&plugin(), &[]).is_none());
    }

    fn item(kind: ItemKind, value: Value) -> PanelItem {
        PanelItem { id: "x".into(), label: "X".into(), detail: String::new(), kind, value }
    }

    #[test]
    fn items_change_by_kind() {
        let toggle = item(ItemKind::Toggle, Value::Bool(false));
        assert_eq!(toggle.change(true), Some(Value::Bool(true)));
        let slider = item(ItemKind::Slider { min: 0.0, max: 10.0, step: 5.0, unit: " W".into() }, Value::Number(10.0));
        assert_eq!(slider.change(true), None, "already at the top");
        assert_eq!(slider.change(false), Some(Value::Number(5.0)));
        assert_eq!(slider.value_label(), "10 W");
        let choices = vec!["a".to_string(), "b".to_string()];
        let choice = item(ItemKind::Choice(choices), Value::Text("b".into()));
        assert_eq!(choice.change(true), Some(Value::Text("a".into())));
        assert_eq!(choice.change(false), Some(Value::Text("a".into())));
        let button = item(ItemKind::Button, Value::Bool(false));
        assert_eq!(button.change(true), None);
        assert_eq!(button.confirm(), Some(Value::Bool(true)));
        assert_eq!(item(ItemKind::Info, Value::Text("42 °C".into())).confirm(), None);
    }

    #[test]
    fn the_panel_moves_activates_and_keeps_its_focus() {
        let mut view = PanelView::loading("Power");
        let mut second = item(ItemKind::Toggle, Value::Bool(true));
        second.id = "y".into();
        view.set(Panel {
            title: "Power".into(),
            items: vec![item(ItemKind::Info, Value::Text(String::new())), second],
        });
        assert_eq!(view.apply(Action::Down), PanelOutcome::Moved);
        assert_eq!(view.apply(Action::Down), PanelOutcome::Nothing);
        assert_eq!(view.apply(Action::Confirm), PanelOutcome::Activate { item: "y".into(), value: Value::Bool(false) });
        assert_eq!(view.panel.items[1].value, Value::Bool(false), "shows the new value at once");
        // The plugin sends its panel again with the items in another order: the focus follows "y"
        let again = vec![view.panel.items[1].clone(), view.panel.items[0].clone()];
        view.set(Panel { title: "Power".into(), items: again });
        assert_eq!(view.focused, 0);
        assert_eq!(view.apply(Action::Back), PanelOutcome::Close);
    }

    #[test]
    fn artwork_fills_only_what_is_missing() {
        let dir = std::env::temp_dir().join(format!("consoleui-art-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cover = dir.join("cover.png");
        std::fs::write(&cover, b"x").unwrap();
        let mut artwork = Artwork { hero: Some("/own/hero.jpg".into()), ..Artwork::default() };
        let found = PluginArtwork {
            cover: Some(cover.display().to_string()),
            hero: Some(cover.display().to_string()),
            logo: Some("/does/not/exist.png".into()),
        };
        found.fill(&mut artwork, &[]);
        assert_eq!(artwork.cover.as_deref(), Some(cover.as_path()));
        assert_eq!(artwork.hero.as_deref(), Some(std::path::Path::new("/own/hero.jpg")), "own pictures stay");
        assert_eq!(artwork.logo, None, "a path that is not there is left out");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_remember_what_is_on() {
        let mut settings = PluginSettings::default();
        settings.set("consoleui-plugin-lutris", true);
        settings.set("consoleui-plugin-lutris", true);
        assert_eq!(settings.enabled.len(), 1);
        let plugins = [plugin()];
        assert_eq!(settings.active(&plugins).len(), 1);
        settings.set("consoleui-plugin-lutris", false);
        assert!(settings.active(&plugins).is_empty());
    }
}
