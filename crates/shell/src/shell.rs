//! Zustand der Shell: Einträge, Bildschirme, Fokus, Historie, laufende Apps und die Verbindung
//! zur UI (Hauptfenster, in gamescope zusätzlich Overlay-Fenster und Fenstersteuerung).

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use consoleui_core::launch::{self, ProcessHandle};
use consoleui_core::netpages::Page;
use consoleui_core::osk::{KeyboardEvent, OnScreenKeyboard};
use consoleui_core::ownership::{self, FIRST_DESKTOP_APP_ID, Owner, SHELL_APP_ID};
use consoleui_core::running::{MIN_HOLD, Phase, RunningApp};
use consoleui_core::settings::{GlyphChoice, Setting, Settings};
use consoleui_core::sound::Sound;
use consoleui_core::system;
use consoleui_core::{
    Action, AppEntry, AppKind, Carousel, Collection, Grid, History, Launch, LibraryFilter, LibrarySort, home_view,
    library_view, search, steam,
};
use slint::{ComponentHandle, Image, ModelRc, VecModel};

use crate::audio::Audio;
use crate::context::{ContextAction, ContextMenu, ContextOutcome, EntryState};
use crate::gamescope::{Gamescope, x11_window_id};
use crate::loader::Library;
use crate::network::Network;
use crate::quick::{ForegroundApp, QuickAction, QuickMenu, QuickOutcome};
use crate::{AppTile, AppWindow, ControllerStyle, GlyphButton, Hint, KeyCap, NotificationWindow, OverlayWindow, Screen, SettingRow, Theme, tiles};

/// So viele Einträge zeigt die Home-Reihe vor der Bibliothekskachel (Angeheftete immer alle).
const HOME_COUNT: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchArea {
    Keyboard,
    Results,
}

enum Stop {
    Process(ProcessHandle),
    Steam(u32),
}

/// Eine gestartete App, solange sie läuft (oder gerade startet).
struct Running {
    entry_id: String,
    /// App-ID für gamescope (`STEAM_GAME`)
    app_id: u32,
    app: RunningApp,
    stop: Stop,
    /// Woran ihre Prozesse erkennbar sind (für die Fensterzuordnung)
    owner: Option<Owner>,
    /// Schon ein Fenster in gamescope zugeordnet?
    has_window: bool,
}

/// Nur in der gamescope-Session: Fenstersteuerung, das Overlay-Fenster für das Quick-Menü und das
/// Benachrichtigungsfenster für Hinweise über laufenden Apps.
pub struct GamescopeUi {
    pub control: Gamescope,
    pub overlay: OverlayWindow,
    /// X11-ID, für die das Overlay vorbereitet ist (ändert sich, falls Slint das Fenster neu anlegt)
    overlay_window: Option<u32>,
    notification: NotificationWindow,
    /// Wie `overlay_window`, für das Benachrichtigungsfenster
    notification_window: Option<u32>,
}

impl GamescopeUi {
    pub fn new(control: Gamescope, overlay: OverlayWindow, notification: NotificationWindow) -> Self {
        Self { control, overlay, overlay_window: None, notification, notification_window: None }
    }
}

pub struct Shell {
    entries: Vec<AppEntry>,
    /// Kachel je Eintrag, in derselben Reihenfolge wie `entries`
    tiles: Vec<AppTile>,
    library_tile: AppTile,
    home_model: Rc<VecModel<AppTile>>,
    library_model: Rc<VecModel<AppTile>>,
    results_model: Rc<VecModel<AppTile>>,

    screen: Screen,
    home: Carousel,
    /// Indizes in `entries` für die Home-Reihe (Angeheftete zuerst, ohne Ausgeblendete)
    home_entries: Vec<usize>,
    grid: Grid,
    /// Indizes in `entries` für die Bibliothek (gefiltert und sortiert)
    library: Vec<usize>,
    filter: LibraryFilter,
    sort: LibrarySort,

    query: String,
    /// Indizes in `entries` für die Suchtreffer
    results: Vec<usize>,
    result_row: Carousel,
    keyboard: OnScreenKeyboard,
    search_area: SearchArea,
    /// Wohin „Zurück“ aus der Suche führt
    search_origin: Screen,

    history: History,
    history_path: Option<PathBuf>,
    /// Angeheftete und ausgeblendete Einträge
    collection: Collection,
    collection_path: Option<PathBuf>,
    /// Offenes Kontextmenü für den fokussierten Eintrag
    context: Option<ContextMenu>,
    /// Hero-Bilder werden erst beim Fokussieren geladen und dann behalten.
    heroes: HashMap<String, Image>,
    /// Eintrag, dessen Hero gerade angezeigt wird (verhindert unnötiges Überblenden)
    hero_entry: Option<String>,

    running: Vec<Running>,
    next_app_id: u32,
    /// App, die in gamescope gerade vorne ist (`None`: die Shell)
    foreground: Option<u32>,
    /// Zeitpunkt eines fehlgeschlagenen Starts; der Startbildschirm zeigt den Fehler kurz an.
    failed_launch: Option<Instant>,
    /// Eintrags-ID, deren Beenden gerade bestätigt werden soll
    confirm_quit: Option<String>,
    quick: Option<QuickMenu>,
    gamescope: Option<GamescopeUi>,

    settings: Settings,
    settings_path: Option<PathBuf>,
    settings_row: usize,
    audio: Option<Audio>,
    /// Zählt Hinweise hoch, damit ein älterer Ausblend-Timer keinen neueren Hinweis schließt
    toast_generation: Rc<Cell<u64>>,
    /// Warnung „Controller-Akku schwach“ schon gezeigt
    battery_warned: bool,
    /// Unterseiten WLAN und Bluetooth der Einstellungen
    network: Network,
}

/// So lange bleibt ein Hinweis stehen
const TOAST_DURATION: Duration = Duration::from_millis(3500);
/// Unter diesem Akkustand warnt die Shell einmal
const LOW_BATTERY: u8 = 15;

/// Was sich durch eine Eingabe geändert hat, um den passenden Klang zu wählen
#[derive(PartialEq)]
struct Feedback {
    screen: Screen,
    quick_open: bool,
    context_open: bool,
    launching: bool,
    focus: [usize; 8],
    query_len: usize,
    settings: Settings,
    network: [usize; 5],
}

/// Startwerte der Animationsdauern in Millisekunden (wie in `theme.slint`)
const DURATIONS: [i64; 5] = [220, 500, 120, 450, 200];

impl Shell {
    /// Startet ohne Einträge; die Bibliothek kommt per [`Shell::set_library`], sobald sie geladen ist.
    pub fn new(
        ui: &AppWindow,
        gamescope: Option<GamescopeUi>,
        settings: Settings,
        settings_path: Option<PathBuf>,
    ) -> Self {
        let keyboard = OnScreenKeyboard::default();
        let keys: Vec<KeyCap> = keyboard
            .keys()
            .iter()
            .map(|k| KeyCap { label: k.key.label().into(), row: k.row as i32, x: k.x, width: k.width })
            .collect();
        ui.set_keys(ModelRc::new(VecModel::from(keys)));

        let home_model = Rc::new(VecModel::default());
        let library_model = Rc::new(VecModel::default());
        let results_model = Rc::new(VecModel::default());
        ui.set_home_tiles(ModelRc::from(home_model.clone()));
        ui.set_library_tiles(ModelRc::from(library_model.clone()));
        ui.set_results(ModelRc::from(results_model.clone()));

        if let Some(gamescope) = &gamescope {
            // Im Steam-Modus ist ohne Baselayer gar nichts zu sehen.
            gamescope.control.set_baselayer(&[SHELL_APP_ID]);
        }

        let mut shell = Self {
            library_tile: tiles::library_tile(0),
            entries: Vec::new(),
            tiles: Vec::new(),
            home_model,
            library_model,
            results_model,
            screen: Screen::Home,
            home: Carousel::new(0),
            home_entries: Vec::new(),
            grid: Grid::new(0),
            library: Vec::new(),
            filter: LibraryFilter::default(),
            sort: LibrarySort::default(),
            query: String::new(),
            results: Vec::new(),
            result_row: Carousel::new(0),
            keyboard,
            search_area: SearchArea::Keyboard,
            search_origin: Screen::Home,
            history: History::default(),
            history_path: None,
            collection: Collection::default(),
            collection_path: None,
            context: None,
            heroes: HashMap::new(),
            hero_entry: None,
            running: Vec::new(),
            next_app_id: FIRST_DESKTOP_APP_ID,
            foreground: None,
            failed_launch: None,
            confirm_quit: None,
            quick: None,
            gamescope,
            settings,
            settings_path,
            settings_row: 0,
            audio: Audio::start(),
            toast_generation: Rc::default(),
            battery_warned: false,
            network: Network::default(),
        };
        shell.apply_settings(ui);
        shell.refresh(ui);
        shell
    }

    /// Übernimmt die im Hintergrund geladene Bibliothek.
    pub fn set_library(&mut self, ui: &AppWindow, library: Library) {
        let today = tiles::today();
        self.tiles = library.entries.iter().enumerate().map(|(i, loaded)| tiles::to_tile(loaded, i, today)).collect();
        self.entries = library.entries.into_iter().map(|loaded| loaded.entry).collect();
        self.library_tile = tiles::library_tile(self.entries.len());
        self.history = library.history;
        self.history_path = library.history_path;
        self.collection = library.collection;
        self.collection_path = library.collection_path;
        self.hero_entry = None;
        self.refresh(ui);
    }

    /// Zeigt einen Hinweis oben rechts; ein neuer ersetzt den alten. Ist in gamescope eine App
    /// vorne, erscheint er im Benachrichtigungsfenster über ihr, sonst im Hauptfenster.
    pub fn toast(&mut self, ui: &AppWindow, symbol: &str, title: &str, text: &str) {
        let generation = self.toast_generation.get() + 1;
        self.toast_generation.set(generation);
        let over_app = !self.shell_in_front(ui);
        if let Some(gamescope) = self.gamescope.as_mut().filter(|_| over_app) {
            ui.set_toast_shown(false);
            let notification = &gamescope.notification;
            notification.set_symbol(symbol.into());
            notification.set_title_text(title.into());
            notification.set_text(text.into());
            if notification.window().is_visible() {
                notification.set_shown(true);
            } else {
                show_notification_window(gamescope);
                // Erst nach dem Anzeigen einblenden, damit der Hinweis hereingleitet.
                let weak = gamescope.notification.as_weak();
                let current = self.toast_generation.clone();
                slint::Timer::single_shot(Duration::from_millis(30), move || {
                    if let Some(notification) = weak.upgrade()
                        && current.get() == generation
                    {
                        notification.set_shown(true);
                    }
                });
            }
        } else {
            self.hide_notification();
            ui.set_toast_symbol(symbol.into());
            ui.set_toast_title(title.into());
            ui.set_toast_text(text.into());
            ui.set_toast_shown(true);
        }
        let weak = ui.as_weak();
        let notification = self.gamescope.as_ref().map(|g| g.notification.as_weak());
        let current = self.toast_generation.clone();
        slint::Timer::single_shot(TOAST_DURATION, move || {
            if current.get() != generation {
                return;
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_shown(false);
            }
            if let Some(notification) = notification.as_ref().and_then(|n| n.upgrade()) {
                hide_notification_window(&notification);
            }
        });
    }

    /// Blendet einen Hinweis im Benachrichtigungsfenster aus (falls einer zu sehen ist).
    fn hide_notification(&self) {
        if let Some(gamescope) = &self.gamescope
            && gamescope.notification.get_shown()
        {
            hide_notification_window(&gamescope.notification);
        }
    }

    /// Warnt einmal, wenn der Controller-Akku knapp wird; lädt er wieder, wird neu gewarnt.
    pub fn check_controller_battery(&mut self, ui: &AppWindow, percent: Option<u8>) {
        match percent {
            Some(p) if p <= LOW_BATTERY && !self.battery_warned => {
                self.battery_warned = true;
                self.toast(ui, "▁", "Controller-Akku schwach", &format!("Noch {p} %"));
            }
            Some(p) if p > LOW_BATTERY + 5 => self.battery_warned = false,
            _ => {}
        }
    }

    /// Gewünschte Tastensymbole aus den Einstellungen (`Auto`: nach dem Controller).
    pub fn glyph_choice(&self) -> GlyphChoice {
        self.settings.glyphs
    }

    /// Startbildschirm setzen (Entwickler-Option `--start`).
    pub fn open(&mut self, ui: &AppWindow, screen: Screen) {
        match screen {
            Screen::Search => self.open_search(ui),
            other => self.screen = other,
        }
        self.sync(ui);
    }

    /// Anzahl der Einträge in der Home-Reihe (inklusive Bibliothekskachel), für den Benchmark.
    pub fn home_len(&self) -> usize {
        self.home_entries.len() + 1
    }

    /// Solange die Steuerzentrale offen ist, liest die Shell die Controller exklusiv.
    pub fn wants_exclusive_input(&self) -> bool {
        self.quick.is_some()
    }

    /// Ob eine Controller-Eingabe an die Shell geht. gilrs liest die Geräte direkt, also bekäme die
    /// Shell sonst auch Tasten, die für das Spiel vorne gedacht sind. Im Quick-Menü gilt alles,
    /// die PS-Taste immer. In gamescope entscheidet, ob die Shell vorne ist (der X11-Fokus ist dort
    /// nach dem Overlay unzuverlässig), auf dem Desktop der Fensterfokus.
    pub fn accepts_gamepad(&self, ui: &AppWindow, action: Action) -> bool {
        if self.quick.is_some() {
            return true;
        }
        if self.gamescope.is_none() {
            return ui.get_window_active();
        }
        action == Action::Guide || self.shell_in_front(ui)
    }

    /// In gamescope: Die Shell ist zu sehen, solange keine App vorne ist, der Startbildschirm läuft
    /// oder die App vorne noch kein Fenster hat.
    fn shell_in_front(&self, ui: &AppWindow) -> bool {
        match self.foreground {
            None => true,
            Some(app_id) => {
                ui.get_launching() || !self.running.iter().any(|r| r.app_id == app_id && r.has_window)
            }
        }
    }

    pub fn apply(&mut self, ui: &AppWindow, action: Action) {
        let before = self.feedback(ui);
        self.apply_action(ui, action);
        let after = self.feedback(ui);
        self.play_feedback(action, &before, &after);
    }

    fn feedback(&self, ui: &AppWindow) -> Feedback {
        Feedback {
            screen: self.screen,
            quick_open: self.quick.is_some(),
            context_open: self.context.is_some(),
            launching: ui.get_launching(),
            focus: [
                self.home.focused(),
                self.grid.focused(),
                self.result_row.focused(),
                self.keyboard.focused(),
                self.settings_row,
                self.quick.as_ref().map_or(0, QuickMenu::focused),
                usize::from(self.search_area == SearchArea::Results),
                self.context.as_ref().map_or(0, ContextMenu::focused),
            ],
            query_len: self.query.len(),
            settings: self.settings,
            network: self.network.pages.feedback(),
        }
    }

    fn play_feedback(&mut self, action: Action, before: &Feedback, after: &Feedback) {
        if before == after {
            return;
        }
        let sound = if after.quick_open != before.quick_open {
            if after.quick_open { Sound::Open } else { Sound::Back }
        } else if after.context_open != before.context_open && !after.launching {
            if after.context_open { Sound::Open } else { Sound::Back }
        } else if after.launching && !before.launching {
            Sound::Confirm
        } else if after.screen != before.screen || after.launching != before.launching {
            if action == Action::Back { Sound::Back } else { Sound::Open }
        } else if action == Action::Confirm && after.focus == before.focus {
            Sound::Confirm
        } else {
            Sound::Move
        };
        if self.settings.sounds
            && let Some(audio) = &mut self.audio
        {
            audio.play(sound);
        }
    }

    fn apply_action(&mut self, ui: &AppWindow, action: Action) {
        if self.quick.is_some() {
            self.apply_quick(ui, action);
            return;
        }
        if action == Action::Guide {
            self.close_context(ui);
            self.open_quick(ui);
            return;
        }
        if ui.get_launching() {
            // „Zurück“ holt die Übersicht, die App läuft weiter und bleibt verfolgt.
            if action == Action::Back {
                self.show_shell(ui);
            }
            return;
        }
        if let Some(id) = self.confirm_quit.clone() {
            match action {
                Action::Confirm => {
                    self.quit_running(&id);
                    self.close_confirm(ui);
                }
                Action::Back => self.close_confirm(ui),
                _ => {}
            }
            return;
        }
        if self.context.is_some() {
            self.apply_context(ui, action);
            self.sync(ui);
            return;
        }
        match (self.screen, action) {
            (Screen::Settings, _) if self.network.pages.is_open() => self.apply_network(ui, action),
            (Screen::Home, Action::NextTab) => self.screen = Screen::Library,
            (Screen::Library, Action::PrevTab) => self.screen = Screen::Home,
            (Screen::Library, Action::NextTab) => self.screen = Screen::Settings,
            (Screen::Settings, Action::PrevTab) => self.screen = Screen::Library,
            (Screen::Home | Screen::Library | Screen::Settings, Action::Search) => self.open_search(ui),
            (Screen::Home, _) => self.apply_home(ui, action),
            (Screen::Library, _) => self.apply_library(ui, action),
            (Screen::Settings, _) => self.apply_settings_page(ui, action),
            (Screen::Search, _) => self.apply_search(ui, action),
        }
        self.sync(ui);
    }

    fn apply_home(&mut self, ui: &AppWindow, action: Action) {
        match action {
            Action::Confirm => match self.home_entry() {
                Some(index) => self.launch(ui, index),
                None => self.screen = Screen::Library,
            },
            Action::Menu => self.open_context(ui),
            _ => {
                self.home.apply(action);
            }
        }
    }

    fn apply_library(&mut self, ui: &AppWindow, action: Action) {
        match action {
            Action::Confirm => {
                if let Some(&index) = self.library.get(self.grid.focused()) {
                    self.launch(ui, index);
                }
            }
            Action::Back => self.screen = Screen::Home,
            Action::Secondary => {
                self.filter = self.filter.next();
                self.grid.focus(0);
                self.refresh_library(ui);
            }
            // Sortierung steht seit dem Kontextmenü (Story 2.7) dort mit drin.
            Action::Menu => self.open_context(ui),
            _ => {
                self.grid.apply(action, ui.get_library_columns().max(1) as usize);
            }
        }
    }

    fn apply_settings_page(&mut self, ui: &AppWindow, action: Action) {
        let setting = Setting::ALL[self.settings_row];
        match action {
            Action::Up => self.settings_row = self.settings_row.saturating_sub(1),
            Action::Down => self.settings_row = (self.settings_row + 1).min(Setting::ALL.len() - 1),
            Action::Confirm if setting.opens_page() => {
                let page = if setting == Setting::Wifi { Page::Wifi } else { Page::Bluetooth };
                let requests = self.network.pages.open(page);
                self.network.send(requests);
                ui.set_net_view(self.network.view());
            }
            Action::Left | Action::Right if setting.opens_page() => {}
            Action::Left | Action::Right | Action::Confirm => {
                self.settings.change(setting, action != Action::Left);
                if let Some(path) = &self.settings_path
                    && let Err(err) = self.settings.save(path)
                {
                    eprintln!("Einstellungen konnten nicht gespeichert werden ({}): {err}", path.display());
                }
                self.apply_settings(ui);
            }
            Action::Back => self.screen = Screen::Home,
            _ => {}
        }
    }

    /// Überträgt Einstellungen in die UI: Akzentfarbe, Animationsdauern, Einstellungszeilen.
    fn apply_settings(&self, ui: &AppWindow) {
        let [r, g, b] = self.settings.accent.rgb();
        let accent = slint::Color::from_rgb_u8(r, g, b);
        let durations = if self.settings.animations { DURATIONS } else { [0; 5] };
        let apply_theme = |theme: Theme| {
            theme.set_accent(accent);
            theme.set_focus_duration(durations[0]);
            theme.set_background_duration(durations[1]);
            theme.set_pulse_duration(durations[2].max(1));
            theme.set_launch_duration(durations[3]);
            theme.set_page_duration(durations[4]);
        };
        apply_theme(ui.global::<Theme>());
        if let Some(gamescope) = &self.gamescope {
            apply_theme(gamescope.overlay.global::<Theme>());
            apply_theme(gamescope.notification.global::<Theme>());
        }
        self.update_settings_rows(ui);
        let footer = match &self.settings_path {
            Some(path) => format!("Wird automatisch gespeichert: {}", path.display()),
            None => "Einstellungen werden nicht gespeichert".into(),
        };
        ui.set_settings_footer(footer.into());
    }

    fn update_settings_rows(&self, ui: &AppWindow) {
        let rows: Vec<SettingRow> = Setting::ALL
            .iter()
            .map(|&setting| SettingRow {
                label: setting.label().into(),
                description: setting.description().into(),
                value: match setting {
                    Setting::Wifi => self.network.pages.summary(Page::Wifi).into(),
                    Setting::Bluetooth => self.network.pages.summary(Page::Bluetooth).into(),
                    _ => self.settings.value_label(setting).into(),
                },
                opens_page: setting.opens_page(),
            })
            .collect();
        ui.set_settings_rows(ModelRc::new(VecModel::from(rows)));
    }

    // --- WLAN und Bluetooth (Stories 5.3/5.4) ----------------------------------------------

    /// Startet den Netzwerk-Thread (`fake`: simuliert, ändert nichts am System) und liest einmal.
    pub fn start_network(&mut self, fake: bool) {
        self.network.start(fake);
        self.network.send(vec![consoleui_core::network::Request::Refresh]);
    }

    /// Läuft per Timer: Antworten des Netzwerk-Threads übernehmen und Hinweise zeigen.
    pub fn poll_network(&mut self, ui: &AppWindow) {
        let (changed, notices) = self.network.poll(self.screen == Screen::Settings);
        for notice in notices {
            self.toast(ui, notice.symbol, &notice.title, &notice.text);
        }
        if changed {
            self.update_settings_rows(ui);
            if self.network.pages.is_open() {
                ui.set_net_view(self.network.view());
                self.update_hints(ui);
            }
        }
    }

    fn apply_network(&mut self, ui: &AppWindow, action: Action) {
        let requests = self.network.pages.apply(action);
        self.network.send(requests);
        ui.set_net_view(self.network.view());
    }

    fn apply_search(&mut self, ui: &AppWindow, action: Action) {
        match (self.search_area, action) {
            (SearchArea::Keyboard, Action::Back) => self.screen = self.search_origin,
            (_, Action::Secondary) => self.erase(ui),
            (SearchArea::Keyboard, Action::Search) => self.insert(ui, ' '),
            (SearchArea::Keyboard, _) => match self.keyboard.apply(action) {
                KeyboardEvent::Type(c) => self.insert(ui, c),
                KeyboardEvent::Space => self.insert(ui, ' '),
                KeyboardEvent::Backspace => self.erase(ui),
                KeyboardEvent::Done => {
                    if !self.results.is_empty() {
                        self.search_area = SearchArea::Results;
                    }
                }
                KeyboardEvent::Moved | KeyboardEvent::Shift | KeyboardEvent::None => {}
            },
            (SearchArea::Results, Action::Up | Action::Back) => self.search_area = SearchArea::Keyboard,
            (SearchArea::Results, Action::Confirm) => {
                if let Some(&index) = self.results.get(self.result_row.focused()) {
                    self.launch(ui, index);
                }
            }
            (SearchArea::Results, _) => {
                self.result_row.apply(action);
            }
        }
    }

    /// Eingabe über die echte Tastatur in der Suche. Steuerzeichen und Sondertasten (Slint meldet
    /// sie als Zeichen aus dem Private-Use-Bereich) werden abgelehnt.
    pub fn type_text(&mut self, ui: &AppWindow, text: &str) -> bool {
        let printable = !text.is_empty()
            && text.chars().all(|c| !c.is_control() && !('\u{e000}'..='\u{f8ff}').contains(&c));
        if printable && self.screen == Screen::Settings && self.quick.is_none() && self.network.pages.type_text(text) {
            ui.set_net_view(self.network.view());
            return true;
        }
        if !printable || self.screen != Screen::Search || ui.get_launching() || self.quick.is_some() {
            return false;
        }
        self.search_area = SearchArea::Keyboard;
        for c in text.chars() {
            self.query.push(c);
        }
        self.refresh_results(ui);
        self.sync(ui);
        true
    }

    pub fn erase_text(&mut self, ui: &AppWindow) {
        if self.screen == Screen::Settings && self.network.pages.erase() {
            ui.set_net_view(self.network.view());
            return;
        }
        self.erase(ui);
        self.sync(ui);
    }

    fn insert(&mut self, ui: &AppWindow, c: char) {
        self.query.push(c);
        self.refresh_results(ui);
    }

    fn erase(&mut self, ui: &AppWindow) {
        if self.query.pop().is_some() {
            self.refresh_results(ui);
        }
    }

    fn open_search(&mut self, ui: &AppWindow) {
        self.search_origin = self.screen;
        self.screen = Screen::Search;
        self.search_area = SearchArea::Keyboard;
        self.keyboard.reset_focus();
        self.query.clear();
        self.refresh_results(ui);
    }

    fn home_entry(&self) -> Option<usize> {
        self.home_entries.get(self.home.focused()).copied()
    }

    /// Index in `entries` des Eintrags, der auf dem aktuellen Bildschirm fokussiert ist.
    fn focused_entry(&self) -> Option<usize> {
        match self.screen {
            Screen::Home => self.home_entry(),
            Screen::Library => self.library.get(self.grid.focused()).copied(),
            Screen::Search => self.results.get(self.result_row.focused()).copied(),
            Screen::Settings => None,
        }
    }

    fn running_for(&self, index: usize) -> Option<&Running> {
        self.running.iter().find(|r| r.entry_id == self.entries[index].id)
    }

    // --- Modelle und Anzeige ---------------------------------------------------------------

    /// Baut alle Kachelmodelle neu auf (nach Start, Ende einer App oder Umsortierung).
    fn refresh(&mut self, ui: &AppWindow) {
        for (tile, entry) in self.tiles.iter_mut().zip(&self.entries) {
            tile.pinned = self.collection.is_pinned(&entry.id);
        }
        self.home_entries = home_view(&self.entries, &self.collection, HOME_COUNT);
        let mut home: Vec<AppTile> = self.home_entries.iter().map(|&i| self.tiles[i].clone()).collect();
        home.push(self.library_tile.clone());
        let focused = self.home.focused();
        self.home = Carousel::new(home.len());
        self.home.focus(focused);
        self.home_model.set_vec(home);
        self.refresh_library(ui);
        self.refresh_results(ui);
        self.sync(ui);
    }

    fn refresh_library(&mut self, ui: &AppWindow) {
        self.library = library_view(&self.entries, self.filter, self.sort, &self.collection);
        self.grid.set_len(self.library.len());
        self.library_model.set_vec(self.library.iter().map(|&i| self.tiles[i].clone()).collect::<Vec<_>>());
        ui.set_filter_label(self.filter.label().into());
        ui.set_sort_label(self.sort.label().into());
    }

    fn refresh_results(&mut self, ui: &AppWindow) {
        let keep = self.results.get(self.result_row.focused()).map(|&i| self.entries[i].id.clone());
        self.results = search::search(&self.entries, &self.query, &self.collection);
        self.result_row = Carousel::new(self.results.len());
        if let Some(pos) = keep.and_then(|id| self.results.iter().position(|&i| self.entries[i].id == id)) {
            self.result_row.focus(pos);
        }
        if self.results.is_empty() {
            self.search_area = SearchArea::Keyboard;
        }
        self.results_model.set_vec(self.results.iter().map(|&i| self.tiles[i].clone()).collect::<Vec<_>>());
        ui.set_search_query(self.query.as_str().into());
    }

    /// Überträgt Bildschirm, Fokus, Hintergrund und Hinweise in die UI.
    fn sync(&mut self, ui: &AppWindow) {
        ui.set_screen(self.screen);
        ui.set_home_focused(self.home.focused() as i32);
        ui.set_library_focused(self.grid.focused() as i32);
        ui.set_result_focused(self.result_row.focused() as i32);
        ui.set_key_focused(self.keyboard.focused() as i32);
        ui.set_settings_focused(self.settings_row as i32);
        ui.set_keyboard_active(self.search_area == SearchArea::Keyboard);
        self.show_hero(ui);
        self.update_hints(ui);
    }

    /// Lädt das Hero-Bild in die gerade unsichtbare Ebene und blendet dann um.
    fn show_hero(&mut self, ui: &AppWindow) {
        let index = self.focused_entry();
        let id = index.map(|i| self.entries[i].id.clone());
        if id == self.hero_entry {
            return;
        }
        let hero = index.and_then(|i| self.hero(i)).unwrap_or_default();
        self.hero_entry = id;
        let front_a = !ui.get_hero_front_a();
        if front_a {
            ui.set_hero_a(hero);
        } else {
            ui.set_hero_b(hero);
        }
        ui.set_hero_front_a(front_a);
    }

    fn hero(&mut self, index: usize) -> Option<Image> {
        let entry = self.entries.get(index)?;
        let path = entry.artwork.hero.as_deref()?;
        if !self.heroes.contains_key(&entry.id) {
            self.heroes.insert(entry.id.clone(), tiles::load(path)?);
        }
        self.heroes.get(&entry.id).cloned()
    }

    /// Hinweisleiste unten, passend zu Bildschirm und Auswahl.
    pub fn update_hints(&self, ui: &AppWindow) {
        let hint = |button, label: &str| Hint { button, label: label.into() };
        let keyboard = ui.get_controller_style() == ControllerStyle::Keyboard;
        let start_label = |index: usize| match self.entries[index].kind {
            AppKind::Game => "Spielen",
            AppKind::App => "Öffnen",
        };
        let mut hints = Vec::new();
        if self.context.is_some() {
            hints.push(hint(GlyphButton::Updown, "Auswählen"));
            hints.push(hint(GlyphButton::Confirm, "OK"));
            hints.push(hint(GlyphButton::Back, "Schließen"));
            ui.set_hints(ModelRc::new(VecModel::from(hints)));
            return;
        }
        match self.screen {
            Screen::Home => {
                hints.push(hint(GlyphButton::Dpad, "Navigieren"));
                match self.home_entry() {
                    Some(i) => {
                        let label = if self.running_for(i).is_some() { "Fortsetzen" } else { start_label(i) };
                        hints.push(hint(GlyphButton::Confirm, label));
                        hints.push(hint(GlyphButton::Menu, "Optionen"));
                    }
                    None => hints.push(hint(GlyphButton::Confirm, "Bibliothek öffnen")),
                }
                hints.push(hint(GlyphButton::North, "Suchen"));
                hints.push(hint(GlyphButton::R1, "Bibliothek"));
                hints.push(hint(GlyphButton::Guide, "Steuerzentrale"));
                if keyboard {
                    hints.push(hint(GlyphButton::Quit, "Shell beenden"));
                }
            }
            Screen::Library => {
                hints.push(hint(GlyphButton::Dpad, "Navigieren"));
                if let Some(i) = self.focused_entry() {
                    let label = if self.running_for(i).is_some() { "Fortsetzen" } else { start_label(i) };
                    hints.push(hint(GlyphButton::Confirm, label));
                }
                hints.push(hint(GlyphButton::West, "Filter"));
                hints.push(hint(GlyphButton::Menu, "Optionen"));
                hints.push(hint(GlyphButton::North, "Suchen"));
                hints.push(hint(GlyphButton::Back, "Zurück"));
            }
            Screen::Settings if self.network.pages.is_open() => {
                hints.extend(self.network.hints().into_iter().map(|(button, label)| hint(button, label)));
            }
            Screen::Settings if Setting::ALL[self.settings_row].opens_page() => {
                hints.push(hint(GlyphButton::Updown, "Auswählen"));
                hints.push(hint(GlyphButton::Confirm, "Öffnen"));
                hints.push(hint(GlyphButton::L1, "Bibliothek"));
                hints.push(hint(GlyphButton::Back, "Zurück"));
            }
            Screen::Settings => {
                hints.push(hint(GlyphButton::Updown, "Auswählen"));
                hints.push(hint(GlyphButton::Dpad, "Ändern"));
                hints.push(hint(GlyphButton::L1, "Bibliothek"));
                hints.push(hint(GlyphButton::Back, "Zurück"));
            }
            Screen::Search if self.search_area == SearchArea::Keyboard => {
                if keyboard {
                    hints.push(hint(GlyphButton::Dpad, "Tippen oder Tasten wählen"));
                } else {
                    hints.push(hint(GlyphButton::Confirm, "Eingeben"));
                    hints.push(hint(GlyphButton::West, "Löschen"));
                    hints.push(hint(GlyphButton::North, "Leerzeichen"));
                }
                hints.push(hint(GlyphButton::Back, "Schließen"));
            }
            Screen::Search => {
                hints.push(hint(GlyphButton::Dpad, "Navigieren"));
                if let Some(i) = self.focused_entry() {
                    hints.push(hint(GlyphButton::Confirm, start_label(i)));
                }
                hints.push(hint(GlyphButton::Back, "Zur Tastatur"));
            }
        }
        ui.set_hints(ModelRc::new(VecModel::from(hints)));
    }

    // --- Kontextmenü -----------------------------------------------------------------------

    /// Öffnet das Kontextmenü für den fokussierten Eintrag (in der Bibliothek auch ohne Eintrag,
    /// wegen der Sortierung). Auf der Bibliothekskachel in Home passiert nichts.
    fn open_context(&mut self, ui: &AppWindow) {
        let entry = self.focused_entry().map(|i| {
            let id = self.entries[i].id.clone();
            EntryState {
                title: self.entries[i].title.clone(),
                pinned: self.collection.is_pinned(&id),
                hidden: self.collection.is_hidden(&id),
                running: self.running_for(i).is_some(),
                id,
            }
        });
        let menu = ContextMenu::new(entry, self.screen == Screen::Library);
        if !menu.is_empty() {
            self.context = Some(menu);
            self.show_context(ui);
        }
    }

    fn apply_context(&mut self, ui: &AppWindow, action: Action) {
        let Some(menu) = self.context.as_mut() else { return };
        match menu.apply(action) {
            ContextOutcome::Stay => self.show_context(ui),
            ContextOutcome::Close => self.close_context(ui),
            ContextOutcome::Run(action) => self.run_context(ui, action),
        }
    }

    fn run_context(&mut self, ui: &AppWindow, action: ContextAction) {
        if action == ContextAction::Sort {
            let keep = self.focused_entry().map(|i| self.entries[i].id.clone());
            self.sort = self.sort.next();
            self.refresh_library(ui);
            if let Some(id) = keep {
                self.focus_entry(&id);
            }
            self.show_context(ui);
            return;
        }
        let Some(id) = self.context.as_ref().and_then(|m| m.entry_id()).map(str::to_string) else { return };
        self.close_context(ui);
        match action {
            ContextAction::Quit => {
                if let Some(index) = self.entries.iter().position(|e| e.id == id) {
                    self.ask_quit(ui, index);
                }
                return;
            }
            ContextAction::Pin => self.collection.pin(&id),
            ContextAction::Unpin => self.collection.unpin(&id),
            ContextAction::Hide => self.collection.hide(&id),
            ContextAction::Show => self.collection.show(&id),
            ContextAction::Sort => {}
        }
        if let Some(path) = &self.collection_path
            && let Err(err) = self.collection.save(path)
        {
            eprintln!("Sammlung konnte nicht gespeichert werden ({}): {err}", path.display());
        }
        self.refresh(ui);
        // Angeheftet/gelöst: Der Fokus wandert mit dem Eintrag an seinen neuen Platz.
        if matches!(action, ContextAction::Pin | ContextAction::Unpin) {
            self.focus_entry(&id);
        }
    }

    fn show_context(&self, ui: &AppWindow) {
        let Some(menu) = &self.context else { return };
        ui.set_context_title(menu.title().to_uppercase().into());
        ui.set_context_items(ModelRc::new(VecModel::from(menu.items(self.sort))));
        ui.set_context_focused(menu.focused() as i32);
        ui.set_context_open(true);
    }

    fn close_context(&mut self, ui: &AppWindow) {
        if self.context.take().is_some() {
            ui.set_context_open(false);
        }
    }

    // --- Quick-Menü ------------------------------------------------------------------------

    fn open_quick(&mut self, ui: &AppWindow) {
        let foreground = self.foreground.and_then(|app_id| {
            let running = self.running.iter().find(|r| r.app_id == app_id)?;
            let entry = self.entries.iter().find(|e| e.id == running.entry_id)?;
            Some(ForegroundApp { app_id, entry_id: entry.id.clone(), title: entry.title.clone() })
        });
        self.quick = Some(QuickMenu::new(foreground));
        self.show_quick(ui, true);
    }

    fn apply_quick(&mut self, ui: &AppWindow, action: Action) {
        let Some(quick) = self.quick.as_mut() else { return };
        match quick.apply(action) {
            QuickOutcome::Stay => self.show_quick(ui, true),
            QuickOutcome::Close => self.close_quick(ui),
            QuickOutcome::Run(action) => {
                self.close_quick(ui);
                match action {
                    QuickAction::Resume(app_id) => self.bring_to_front(ui, app_id),
                    QuickAction::Home => {
                        self.show_shell(ui);
                        self.screen = Screen::Home;
                        self.sync(ui);
                    }
                    QuickAction::Quit(entry_id) => self.quit_running(&entry_id),
                    QuickAction::Volume => {}
                    QuickAction::Power(power) => {
                        if let Err(err) = system::power(power) {
                            eprintln!("Energieaktion fehlgeschlagen: {err}");
                        }
                    }
                    QuickAction::Logout => {
                        let _ = slint::quit_event_loop();
                    }
                }
            }
        }
    }

    fn close_quick(&mut self, ui: &AppWindow) {
        self.show_quick(ui, false);
        self.quick = None;
    }

    /// Zeigt das Quick-Menü im Overlay-Fenster (gamescope) oder als Ebene im Hauptfenster.
    fn show_quick(&mut self, ui: &AppWindow, open: bool) {
        let Some(quick) = &self.quick else { return };
        let items = ModelRc::new(VecModel::from(quick.items()));
        let (title, focused) = (quick.title(), quick.focused() as i32);
        let Some(gamescope) = &mut self.gamescope else {
            ui.set_quick_items(items);
            ui.set_quick_title(title.into());
            ui.set_quick_focused(focused);
            ui.set_quick_open(open);
            return;
        };
        let overlay = &gamescope.overlay;
        overlay.set_items(items);
        overlay.set_title_text(title.into());
        overlay.set_focused(focused);
        overlay.set_controller_style(ui.get_controller_style());
        if open && !overlay.get_open() {
            show_overlay_window(gamescope);
            // Erst nach dem Anzeigen öffnen, damit die Leiste hereingleitet.
            let weak = gamescope.overlay.as_weak();
            slint::Timer::single_shot(Duration::from_millis(30), move || {
                if let Some(overlay) = weak.upgrade() {
                    overlay.set_open(true);
                }
            });
        } else if !open {
            overlay.set_open(false);
            let weak = gamescope.overlay.as_weak();
            slint::Timer::single_shot(Duration::from_millis(450), move || {
                if let Some(overlay) = weak.upgrade().filter(|o| !o.get_open()) {
                    let _ = overlay.hide();
                }
            });
        }
    }

    // --- Starten, Verfolgen, Beenden -------------------------------------------------------

    fn launch(&mut self, ui: &AppWindow, index: usize) {
        // Läuft die App schon, kommt sie nur wieder nach vorn.
        if let Some(app_id) = self.running_for(index).map(|r| r.app_id) {
            self.bring_to_front(ui, app_id);
            return;
        }

        let hero = self.hero(index);
        let entry = &self.entries[index];
        let tile = &self.tiles[index];
        ui.set_launch_title(tile.title.clone());
        ui.set_launch_image(hero.unwrap_or_else(|| tile.cover.clone()));
        ui.set_launch_accent(tile.accent);

        let now = Instant::now();
        let id = entry.id.clone();
        match launch::spawn(&entry.launch) {
            Ok(handle) => {
                let (app_id, app, stop, owner) = match entry.launch {
                    Launch::Steam(steam_id) => {
                        (steam_id, RunningApp::steam(steam_id, now), Stop::Steam(steam_id), Some(Owner::Steam(steam_id)))
                    }
                    _ => {
                        let app_id = self.next_app_id;
                        self.next_app_id += 1;
                        (app_id, RunningApp::process(handle.clone(), now), Stop::Process(handle.clone()), handle.owner())
                    }
                };
                self.running.push(Running { entry_id: id.clone(), app_id, app, stop, owner, has_window: false });
                ui.set_launch_status("wird gestartet …".into());
                ui.set_launching(true);
                self.set_running_flag(&id, true);
                self.record_launch(index);
                self.home.focus(0);
                self.refresh(ui);
                self.focus_entry(&id);
                self.set_foreground(Some(app_id));
            }
            Err(err) => {
                eprintln!("{} konnte nicht gestartet werden: {err}", entry.title);
                ui.set_launch_status("konnte nicht gestartet werden".into());
                ui.set_launching(true);
                self.failed_launch = Some(now);
                let title = entry.title.clone();
                self.toast(ui, "⚠", "Start fehlgeschlagen", &title);
            }
        }
    }

    /// Holt eine laufende App nach vorn. Ohne gamescope geht das nicht; dann zeigt der
    /// Startbildschirm, dass sie läuft.
    fn bring_to_front(&mut self, ui: &AppWindow, app_id: u32) {
        if self.gamescope.is_some() {
            self.set_foreground(Some(app_id));
            return;
        }
        let Some(index) = self.running.iter().find(|r| r.app_id == app_id).and_then(|r| {
            self.entries.iter().position(|e| e.id == r.entry_id)
        }) else {
            return;
        };
        let hero = self.hero(index);
        ui.set_launch_title(self.tiles[index].title.clone());
        ui.set_launch_image(hero.unwrap_or_else(|| self.tiles[index].cover.clone()));
        ui.set_launch_accent(self.tiles[index].accent);
        ui.set_launch_status("läuft".into());
        ui.set_launching(true);
    }

    /// Die Shell nach vorn holen; laufende Apps laufen im Hintergrund weiter.
    fn show_shell(&mut self, ui: &AppWindow) {
        ui.set_launching(false);
        self.set_foreground(None);
    }

    fn set_foreground(&mut self, app_id: Option<u32>) {
        self.foreground = app_id;
        if let Some(gamescope) = &self.gamescope {
            match app_id {
                Some(app_id) => gamescope.control.set_baselayer(&[app_id, SHELL_APP_ID]),
                None => gamescope.control.set_baselayer(&[SHELL_APP_ID]),
            }
        }
    }

    /// Fokus in Home, Bibliothek und Suche auf den Eintrag setzen (nach dem Umsortieren).
    fn focus_entry(&mut self, id: &str) {
        if let Some(pos) = self.home_entries.iter().position(|&i| self.entries[i].id == id) {
            self.home.focus(pos);
        }
        if let Some(pos) = self.library.iter().position(|&i| self.entries[i].id == id) {
            self.grid.focus(pos);
        }
        if let Some(pos) = self.results.iter().position(|&i| self.entries[i].id == id) {
            self.result_row.focus(pos);
        }
    }

    /// Läuft regelmäßig: ordnet in gamescope neue Fenster ihren Apps zu und prüft, welche Apps
    /// beendet sind. Ist die App vorne fertig, kommt die Shell mit Animation zurück.
    pub fn watch_running(&mut self, ui: &AppWindow) {
        if let Some(failed) = self.failed_launch
            && failed.elapsed() >= MIN_HOLD
        {
            self.failed_launch = None;
            ui.set_launching(false);
        }
        self.tag_windows(ui);
        self.restore_keyboard_focus(ui);

        let now = Instant::now();
        let steam_games =
            if self.running.iter().any(|r| r.app.is_steam()) { steam::running_games() } else { Default::default() };
        let mut finished = Vec::new();
        for running in &mut self.running {
            match running.app.phase(now, &steam_games) {
                Phase::Starting => {}
                Phase::Running => {
                    if ui.get_launching() && self.foreground == Some(running.app_id) {
                        ui.set_launch_status("läuft".into());
                    }
                }
                Phase::Finished => finished.push((running.entry_id.clone(), running.app_id)),
            }
        }
        if finished.is_empty() {
            return;
        }
        self.running.retain(|r| !finished.iter().any(|(_, app_id)| *app_id == r.app_id));
        for (entry_id, app_id) in &finished {
            self.set_running_flag(entry_id, false);
            if self.confirm_quit.as_deref() == Some(entry_id.as_str()) {
                self.close_confirm(ui);
            }
            if self.foreground == Some(*app_id) {
                self.show_shell(ui);
            }
        }
        self.refresh(ui);
    }

    /// Ist die Shell vorne, hat aber keinen Tastaturfokus (passiert nach dem Overlay), holt sie ihn sich.
    /// Ebenso für die App vorne, deren Fenster den Fokus nach dem Overlay sonst nicht zurückbekommt.
    fn restore_keyboard_focus(&self, ui: &AppWindow) {
        let Some(gamescope) = &self.gamescope else { return };
        if self.quick.is_some() {
            return;
        }
        if self.shell_in_front(ui) {
            if !ui.get_window_active()
                && let Some(window) = x11_window_id(ui.window())
            {
                gamescope.control.focus(window);
            }
        } else if let Some(app_id) = self.foreground {
            gamescope.control.focus_app(app_id);
        }
    }

    /// gamescope zeigt im Steam-Modus nur Fenster mit App-ID; die vergibt die Shell hier.
    fn tag_windows(&mut self, ui: &AppWindow) {
        let Some(gamescope) = &mut self.gamescope else { return };
        let own_pid = std::process::id();
        let apps: Vec<(u32, Owner)> =
            self.running.iter().filter_map(|r| r.owner.clone().map(|owner| (r.app_id, owner))).collect();
        // Fenster ohne erkennbaren Besitzer (z. B. Steam selbst beim ersten Start) gehören zur
        // zuletzt gestarteten App, die noch kein Fenster hat.
        let fallback = self.running.iter().rev().find(|r| !r.has_window).map(|r| r.app_id);
        let tagged = gamescope.control.tag_windows(
            |pid| {
                if pid == own_pid {
                    Some(SHELL_APP_ID)
                } else {
                    ownership::owner_of(pid, &apps, ownership::read_process)
                }
            },
            fallback,
        );
        for app_id in tagged {
            if let Some(running) = self.running.iter_mut().find(|r| r.app_id == app_id) {
                running.has_window = true;
                // Das Spiel ist jetzt zu sehen; der Startbildschirm darunter wird nicht mehr gebraucht.
                if self.foreground == Some(app_id) {
                    ui.set_launching(false);
                }
            }
        }
    }

    /// Setzt das „Läuft“-Badge und die Unterzeile; gibt zurück, ob es den Eintrag gab.
    fn set_running_flag(&mut self, id: &str, running: bool) -> bool {
        let Some(index) = self.entries.iter().position(|e| e.id == id) else { return false };
        let tile = &mut self.tiles[index];
        tile.running = running;
        tile.action_label = tiles::action_label(self.entries[index].kind, running).into();
        tile.subtitle =
            if running { "Läuft gerade".into() } else { tiles::subtitle(&self.entries[index], tiles::today()).into() };
        true
    }

    fn ask_quit(&mut self, ui: &AppWindow, index: usize) {
        if self.running_for(index).is_none() {
            return;
        }
        self.confirm_quit = Some(self.entries[index].id.clone());
        ui.set_confirm_title(format!("{} beenden?", self.entries[index].title).into());
        ui.set_confirm_text("Nicht gespeicherter Fortschritt kann verloren gehen.".into());
        ui.set_confirm_open(true);
    }

    fn close_confirm(&mut self, ui: &AppWindow) {
        self.confirm_quit = None;
        ui.set_confirm_open(false);
        self.update_hints(ui);
    }

    /// Bittet die App, sich zu beenden. Das Badge verschwindet, sobald `watch_running` das Ende sieht.
    fn quit_running(&mut self, entry_id: &str) {
        let Some(running) = self.running.iter().find(|r| r.entry_id == entry_id) else { return };
        let result = match &running.stop {
            Stop::Process(handle) => handle.stop(),
            Stop::Steam(app_id) => steam::stop_game(*app_id).map(|_| ()),
        };
        if let Err(err) = result {
            eprintln!("Beenden fehlgeschlagen: {err}");
        }
    }

    /// Speichert den Start in der Historie und schiebt den Eintrag nach vorn. Das passiert unter
    /// dem Startbildschirm, deshalb sieht man die Umsortierung nicht.
    fn record_launch(&mut self, index: usize) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let mut entry = self.entries.remove(index);
        entry.last_used = Some(now);
        self.history.record(&entry.id, now);
        if let Some(path) = &self.history_path
            && let Err(err) = self.history.save(path)
        {
            eprintln!("Historie konnte nicht gespeichert werden ({}): {err}", path.display());
        }
        let tile = self.tiles.remove(index);
        self.entries.insert(0, entry);
        self.tiles.insert(0, tile);
    }
}

/// Zeigt das Overlay-Fenster. Beim ersten Mal (oder wenn Slint das Fenster neu angelegt hat)
/// bekommt es zuerst die Overlay-Atome und wird dann neu gemappt, weil gamescope sie nur beim
/// Mappen liest.
fn show_overlay_window(gamescope: &mut GamescopeUi) {
    let overlay = &gamescope.overlay;
    overlay.window().set_fullscreen(true);
    if let Err(err) = overlay.show() {
        eprintln!("Overlay konnte nicht angezeigt werden: {err}");
        return;
    }
    let window = x11_window_id(overlay.window());
    if window.is_some() && window != gamescope.overlay_window {
        if let Some(id) = window {
            gamescope.control.prepare_overlay(id);
        }
        let _ = overlay.hide();
        let _ = overlay.show();
        gamescope.overlay_window = window;
    }
}

/// Zeigt das Benachrichtigungsfenster. gamescope wählt es nur als Benachrichtigung (statt als
/// interaktives Overlay), wenn es schmaler als das Root-Fenster ist. Es wird zentriert und auf die
/// Ausgabe skaliert, die eigene Position zählt nicht; deshalb ist es genau ein Pixel schmaler als
/// der Bildschirm und volle Höhe (Maßstab 1), und der Hinweis steht darin oben rechts. Die Größe
/// ist fest (min = max in den Size-Hints), sonst zieht gamescope es auf Bildschirmgröße.
fn show_notification_window(gamescope: &mut GamescopeUi) {
    let notification = &gamescope.notification;
    if let Some((width, height)) = gamescope.control.root_size() {
        let scale = notification.window().scale_factor();
        notification.set_window_width(width.saturating_sub(1).max(1) as f32 / scale);
        notification.set_window_height(height as f32 / scale);
    }
    if let Err(err) = notification.show() {
        eprintln!("Hinweisfenster konnte nicht angezeigt werden: {err}");
        return;
    }
    let window = x11_window_id(notification.window());
    if window.is_some() && window != gamescope.notification_window {
        if let Some(id) = window {
            gamescope.control.prepare_notification(id);
        }
        let _ = notification.hide();
        let _ = notification.show();
        gamescope.notification_window = window;
    }
}

/// Blendet den Hinweis aus und versteckt das Fenster danach, sofern inzwischen kein neuer kam.
fn hide_notification_window(notification: &NotificationWindow) {
    notification.set_shown(false);
    let weak = notification.as_weak();
    slint::Timer::single_shot(Duration::from_millis(450), move || {
        if let Some(notification) = weak.upgrade().filter(|n| !n.get_shown()) {
            let _ = notification.hide();
        }
    });
}
