//! Plugins in the shell (Epic 9): starts the ones the profile turned on, puts their entries into
//! the lists, shows their panels from the quick menu and the plugin list of the settings. The bus
//! side is `crate::plugins`, the model `consoleui_core::plugins`.

use std::sync::mpsc::Receiver;

use consoleui_core::plugins::{
    self as model, ItemKind, Panel, PanelItem, PanelOutcome, PanelView, PluginInfo, PluginSettings, Provides, Value,
};
use consoleui_core::{i18n, xdg};

use super::*;
use crate::PanelRow;
use crate::loader::LoadedEntry;
use crate::plugins::{Event, Host, Request};

#[derive(Default)]
pub(super) struct Plugins {
    /// Installed, found at the start and when the list opens
    infos: Vec<PluginInfo>,
    settings: PluginSettings,
    settings_path: Option<PathBuf>,
    host: Option<Host>,
    /// Entries of a plugin whose artwork is being decoded
    decoding: Vec<(String, Receiver<Vec<LoadedEntry>>)>,
    pub panel: Option<OpenPanel>,
}

pub(super) struct OpenPanel {
    /// `None`: the plugin list of the settings
    plugin: Option<String>,
    view: PanelView,
    /// Over an app: in the overlay window
    in_overlay: bool,
}

impl Plugins {
    fn info(&self, id: &str) -> Option<&PluginInfo> {
        self.infos.iter().find(|info| info.id == id)
    }

    /// The plugins that are on and have a panel, for the quick menu's cards.
    pub fn panels(&self) -> Vec<(String, String)> {
        let active = self.settings.active(&self.infos);
        active
            .iter()
            .filter(|info| info.provides(Provides::Panel))
            .map(|info| (info.id.clone(), info.name.clone()))
            .collect()
    }

    /// "1 of 2 on", for the settings row
    pub fn summary(&self) -> String {
        if self.infos.is_empty() {
            return tr("None installed").to_string();
        }
        let on = self.settings.active(&self.infos).len().to_string();
        trf("{} of {} on", &[&on, &self.infos.len().to_string()]).to_string()
    }
}

fn discover() -> Vec<PluginInfo> {
    model::discover(&xdg::data_dirs(), &i18n::desktop_languages(i18n::language(), xdg::languages()))
}

impl Shell {
    /// After the profile changed or a plugin was turned on or off: the host runs exactly the
    /// plugins that are on now.
    pub(super) fn run_plugins(&mut self, settings_path: Option<PathBuf>) {
        let plugins = &mut self.plugins;
        if plugins.infos.is_empty() {
            plugins.infos = discover();
        }
        if let Some(path) = settings_path {
            plugins.settings = PluginSettings::load(&path);
            plugins.settings_path = Some(path);
        }
        let active: Vec<PluginInfo> = plugins.settings.active(&plugins.infos).into_iter().cloned().collect();
        if active.is_empty() && plugins.host.is_none() {
            return;
        }
        plugins.host.get_or_insert_with(Host::start).send(Request::Run(active));
    }

    /// When the shell quits: the plugins it started end with it.
    pub fn stop_plugins(&mut self) {
        if let Some(host) = self.plugins.host.take() {
            host.stop();
        }
    }

    fn save_plugin_settings(&self) {
        if let Some(path) = &self.plugins.settings_path
            && let Err(err) = self.plugins.settings.save(path)
        {
            eprintln!("Cannot save the plugins ({}): {err}", path.display());
        }
    }

    /// Answers of the plugins and decoded artwork of their entries.
    pub fn poll_plugins(&mut self, ui: &AppWindow) {
        let mut decoded = Vec::new();
        self.plugins.decoding.retain(|(plugin, rx)| match rx.try_recv() {
            Ok(loaded) => {
                decoded.push((plugin.clone(), loaded));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
        });
        for (plugin, loaded) in decoded {
            self.merge_plugin_entries(ui, &plugin, loaded);
        }
        let events: Vec<Event> = self.plugins.host.iter().flat_map(|host| host.events.try_iter()).collect();
        for event in events {
            match event {
                Event::Entries { plugin, entries } => {
                    let Some(info) = self.plugins.info(&plugin).cloned() else { continue };
                    let dirs = xdg::data_dirs();
                    let entries: Vec<AppEntry> =
                        entries.into_iter().filter_map(|entry| entry.into_app_entry(&info, &dirs)).collect();
                    let rx = loader::spawn_decode(entries, self.settings.artwork);
                    // Only the newest answer of a plugin counts
                    self.plugins.decoding.retain(|(other, _)| *other != plugin);
                    self.plugins.decoding.push((plugin, rx));
                }
                Event::Panel { plugin, panel } => {
                    if let Some(open) = self.plugins.panel.as_mut().filter(|open| open.plugin.as_ref() == Some(&plugin))
                    {
                        open.view.set(panel);
                        self.show_panel(ui);
                    }
                }
                Event::PanelChanged(plugin) => {
                    if self.plugins.panel.as_ref().is_some_and(|open| open.plugin.as_ref() == Some(&plugin))
                        && let Some(host) = &self.plugins.host
                    {
                        host.send(Request::Panel(plugin));
                    }
                }
                Event::Failed { plugin, error } => {
                    let name = self.plugins.info(&plugin).map_or(plugin.clone(), |info| info.name.clone());
                    let open = self.plugins.panel.as_mut().filter(|open| open.plugin.as_ref() == Some(&plugin));
                    if let Some(open) = open {
                        // The panel says it instead of waiting forever
                        let item = PanelItem {
                            id: "error".into(),
                            label: tr("The plugin does not answer").into(),
                            detail: error.clone(),
                            kind: ItemKind::Info,
                            value: Value::Text(String::new()),
                        };
                        open.view.set(Panel { title: name.clone(), items: vec![item] });
                        self.show_panel(ui);
                    }
                    self.toast(ui, "⚠", &name, &error);
                }
            }
        }
    }

    /// Replaces the entries of `plugin` with `loaded` (running ones stay until they end).
    fn merge_plugin_entries(&mut self, ui: &AppWindow, plugin: &str, loaded: Vec<LoadedEntry>) {
        eprintln!("Plugin {plugin}: {} entries", loaded.len());
        let keep = self.focused_entry().map(|i| self.entries[i].id.clone());
        let mut index = 0;
        while index < self.entries.len() {
            let id = &self.entries[index].id;
            let gone = model::is_entry_of(id, plugin)
                && !loaded.iter().any(|l| &l.entry.id == id)
                && !self.running.iter().any(|r| &r.entry_id == id);
            if gone {
                self.entries.remove(index);
                self.tiles.remove(index);
            } else {
                index += 1;
            }
        }
        let today = tiles::today();
        for loaded in loaded {
            self.discovered_last_used.insert(loaded.entry.id.clone(), loaded.entry.last_used);
            match self.entries.iter().position(|e| e.id == loaded.entry.id) {
                Some(index) => {
                    let running = self.tiles[index].running;
                    self.tiles[index] = tiles::to_tile(&loaded, index, today);
                    self.tiles[index].running = running;
                    self.entries[index] = loaded.entry;
                }
                None => {
                    self.tiles.push(tiles::to_tile(&loaded, self.tiles.len(), today));
                    self.entries.push(loaded.entry);
                }
            }
        }
        apply_history(&mut self.entries, &self.history);
        self.resort();
        for index in 0..self.tiles.len() {
            self.relabel_tile(index);
        }
        self.library_tile = tiles::library_tile(self.entries.len());
        self.refresh(ui);
        if let Some(id) = keep {
            self.focus_entry(&id);
        }
    }

    /// For the navigation sounds: where the focus is in an open panel
    pub(super) fn panel_focus(&self) -> usize {
        self.plugins.panel.as_ref().map_or(usize::MAX, |open| open.view.focused)
    }

    /// From the quick menu's card of a plugin.
    pub(super) fn open_plugin_panel(&mut self, ui: &AppWindow, plugin: &str) {
        let Some(name) = self.plugins.info(plugin).map(|info| info.name.clone()) else { return };
        let in_overlay = self.quick.is_some() && self.overlay_window().is_some();
        self.plugins.panel =
            Some(OpenPanel { plugin: Some(plugin.into()), view: PanelView::loading(&name), in_overlay });
        if let Some(host) = &self.plugins.host {
            host.send(Request::Panel(plugin.into()));
        }
        self.show_panel(ui);
    }

    /// The settings row "Plugins": every installed plugin with a switch.
    pub(super) fn open_plugin_list(&mut self, ui: &AppWindow) {
        self.plugins.infos = discover();
        let mut view = PanelView::loading(tr("Plugins"));
        view.set(self.plugin_list());
        self.plugins.panel = Some(OpenPanel { plugin: None, view, in_overlay: false });
        self.show_panel(ui);
    }

    fn plugin_list(&self) -> Panel {
        let items = if self.plugins.infos.is_empty() {
            vec![PanelItem {
                id: String::new(),
                label: tr("No plugins installed").into(),
                detail: tr("Plugins come as packages, like apps").into(),
                kind: ItemKind::Info,
                value: Value::Text(String::new()),
            }]
        } else {
            self.plugins
                .infos
                .iter()
                .map(|info| PanelItem {
                    id: info.id.clone(),
                    label: info.name.clone(),
                    detail: if info.comment.is_empty() { info.summary() } else { info.comment.clone() },
                    kind: ItemKind::Toggle,
                    value: Value::Bool(self.plugins.settings.is_enabled(&info.id)),
                })
                .collect()
        };
        Panel { title: tr("Plugins").into(), items }
    }

    pub(super) fn apply_panel(&mut self, ui: &AppWindow, action: Action) {
        let Some(open) = self.plugins.panel.as_mut() else { return };
        match open.view.apply(action) {
            PanelOutcome::Nothing => return,
            PanelOutcome::Moved => {}
            PanelOutcome::Close => return self.close_panel(ui),
            PanelOutcome::Activate { item, value } => match open.plugin.clone() {
                Some(plugin) => {
                    if let Some(host) = &self.plugins.host {
                        host.send(Request::Activate { plugin, item, value });
                    }
                }
                None => self.switch_plugin(ui, &item, value == Value::Bool(true)),
            },
        }
        self.show_panel(ui);
    }

    /// The switch of a plugin in the list: off at once, on only after the user agreed.
    fn switch_plugin(&mut self, ui: &AppWindow, id: &str, on: bool) {
        let Some(info) = self.plugins.info(id).cloned() else { return };
        if !on {
            self.plugins.settings.set(id, false);
            self.save_plugin_settings();
            self.run_plugins(None);
            return;
        }
        // The switch stays off until the dialog says yes
        let list = self.plugin_list();
        if let Some(open) = self.plugins.panel.as_mut() {
            open.view.set(list);
        }
        let mut text = trf("{} comes from its author, not from ConsoleUI. It can: {}.", &[&info.name, &info.summary()]);
        if info.provides(Provides::Library) {
            text = format!("{text} {}", tr("It starts programs on this computer when you open its games."));
        }
        let buttons = vec![
            DialogButton { label: tr("Cancel").into(), tag: DialogTag::Close },
            DialogButton { label: tr("Turn on").into(), tag: DialogTag::EnablePlugin(id.to_string()) },
        ];
        self.show_dialog(ui, Dialog::new(trf("Turn on {}?", &[&info.name]), text, buttons, 1));
    }

    /// The dialog said yes.
    pub(super) fn enable_plugin(&mut self, ui: &AppWindow, id: &str) {
        self.plugins.settings.set(id, true);
        self.save_plugin_settings();
        self.run_plugins(None);
        let list = self.plugin_list();
        if let Some(open) = self.plugins.panel.as_mut().filter(|open| open.plugin.is_none()) {
            open.view.set(list);
        }
        self.show_panel(ui);
        self.update_settings_rows(ui);
    }

    pub(super) fn close_panel(&mut self, ui: &AppWindow) {
        let Some(open) = self.plugins.panel.take() else { return };
        match open.in_overlay.then(|| self.overlay_window()).flatten() {
            Some(overlay) => overlay.set_panel_open(false),
            None => ui.set_panel_open(false),
        }
        self.update_settings_rows(ui);
        self.update_hints(ui);
    }

    /// Puts the open panel into its window; hidden while a dialog is open above it.
    pub(super) fn show_panel(&self, ui: &AppWindow) {
        let Some(open) = &self.plugins.panel else { return };
        let view = &open.view;
        let rows: Vec<PanelRow> = view
            .panel
            .items
            .iter()
            .map(|item| PanelRow {
                label: item.label.as_str().into(),
                detail: item.detail.as_str().into(),
                value: item.value_label().into(),
                adjustable: item.adjustable(),
                button: item.kind == ItemKind::Button,
                level: match (&item.kind, &item.value) {
                    (ItemKind::Slider { min, max, .. }, Value::Number(value)) if max > min => {
                        ((value - min) / (max - min) * 100.0).round() as i32
                    }
                    _ => -1,
                },
            })
            .collect();
        let focused = view.panel.items.get(view.focused);
        let hint = |button, label: &str| Hint { button, label: label.into() };
        let mut hints = vec![hint(GlyphButton::Dpad, tr("Select"))];
        match focused.map(|item| &item.kind) {
            Some(ItemKind::Button) => hints.push(hint(GlyphButton::Confirm, tr("OK"))),
            Some(ItemKind::Toggle | ItemKind::Slider { .. } | ItemKind::Choice(_)) => {
                hints.push(hint(GlyphButton::Confirm, tr("Change")));
            }
            _ => {}
        }
        hints.push(hint(GlyphButton::Back, tr("Close")));
        let subtitle = match &open.plugin {
            None => tr("Plugins come from their authors. Turn on only the ones you trust."),
            Some(_) => "",
        };
        let title = view.panel.title.as_str();
        let set = |window: PanelWindow| {
            window.title(title.into());
            window.subtitle(subtitle.into());
            window.rows(ModelRc::new(VecModel::from(rows.clone())));
            window.focused(view.focused as i32);
            window.loading(view.loading);
            window.hints(ModelRc::new(VecModel::from(hints.clone())));
            window.open(self.dialog.is_none());
        };
        match open.in_overlay.then(|| self.overlay_window()).flatten() {
            Some(overlay) => set(PanelWindow::Overlay(&overlay)),
            None => set(PanelWindow::Main(ui)),
        }
    }
}

/// The two windows that can show a panel, with the same properties.
enum PanelWindow<'a> {
    Main(&'a AppWindow),
    Overlay(&'a OverlayWindow),
}

macro_rules! both {
    ($name:ident, $setter:ident, $type:ty) => {
        fn $name(&self, value: $type) {
            match self {
                PanelWindow::Main(window) => window.$setter(value),
                PanelWindow::Overlay(window) => window.$setter(value),
            }
        }
    };
}

impl PanelWindow<'_> {
    both!(title, set_panel_title, slint::SharedString);
    both!(subtitle, set_panel_subtitle, slint::SharedString);
    both!(rows, set_panel_rows, ModelRc<PanelRow>);
    both!(focused, set_panel_focused, i32);
    both!(loading, set_panel_loading, bool);
    both!(hints, set_panel_hints, ModelRc<Hint>);
    both!(open, set_panel_open, bool);
}
