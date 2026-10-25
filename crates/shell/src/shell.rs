//! Zustand der Shell: Einträge, Bildschirme, Fokus, Historie und die Verbindung zur UI.

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use consoleui_core::launch::{self, ProcessHandle};
use consoleui_core::osk::{KeyboardEvent, OnScreenKeyboard};
use consoleui_core::running::{MIN_HOLD, Phase, RunningApp};
use consoleui_core::{
    Action, AppEntry, AppKind, Carousel, Grid, History, Launch, LibraryFilter, LibrarySort, library_view, search,
    steam,
};
use slint::{Image, ModelRc, VecModel};

use crate::{AppTile, AppWindow, ControllerStyle, GlyphButton, Hint, KeyCap, Screen, tiles};

/// So viele zuletzt genutzte Einträge zeigt die Home-Reihe vor der Bibliothekskachel.
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

/// Die zuletzt gestartete App, solange sie läuft (oder gerade startet).
struct Running {
    entry_id: String,
    app: RunningApp,
    stop: Stop,
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
    /// Hero-Bilder werden erst beim Fokussieren geladen und dann behalten.
    heroes: HashMap<String, Image>,
    /// Eintrag, dessen Hero gerade angezeigt wird (verhindert unnötiges Überblenden)
    hero_entry: Option<String>,
    running: Option<Running>,
    /// Zeitpunkt eines fehlgeschlagenen Starts; der Startbildschirm zeigt den Fehler kurz an.
    failed_launch: Option<Instant>,
    /// Eintrags-ID, deren Beenden gerade bestätigt werden soll
    confirm_quit: Option<String>,
}

impl Shell {
    pub fn new(ui: &AppWindow, mut entries: Vec<AppEntry>, history: History, history_path: Option<PathBuf>) -> Self {
        let today = tiles::today();
        let tile_list = entries.iter_mut().enumerate().map(|(i, entry)| tiles::to_tile(entry, i, today)).collect();

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

        let mut shell = Self {
            library_tile: tiles::library_tile(entries.len()),
            entries,
            tiles: tile_list,
            home_model,
            library_model,
            results_model,
            screen: Screen::Home,
            home: Carousel::new(0),
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
            history,
            history_path,
            heroes: HashMap::new(),
            hero_entry: None,
            running: None,
            failed_launch: None,
            confirm_quit: None,
        };
        shell.refresh(ui);
        shell
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
        self.tiles.len().min(HOME_COUNT) + 1
    }

    pub fn apply(&mut self, ui: &AppWindow, action: Action) {
        if ui.get_launching() {
            // „Zurück“ holt die Übersicht, die App läuft weiter und bleibt verfolgt.
            if action == Action::Back {
                ui.set_launching(false);
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
        match (self.screen, action) {
            (Screen::Home, Action::NextTab) => self.screen = Screen::Library,
            (Screen::Library, Action::PrevTab) => self.screen = Screen::Home,
            (Screen::Home | Screen::Library, Action::Search) => self.open_search(ui),
            (Screen::Home, _) => self.apply_home(ui, action),
            (Screen::Library, _) => self.apply_library(ui, action),
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
            Action::Menu => {
                if let Some(index) = self.home_entry() {
                    self.ask_quit(ui, index);
                }
            }
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
            Action::Menu => {
                let keep = self.focused_entry().map(|i| self.entries[i].id.clone());
                self.sort = self.sort.next();
                self.refresh_library(ui);
                if let Some(pos) = keep.and_then(|id| self.library.iter().position(|&i| self.entries[i].id == id)) {
                    self.grid.focus(pos);
                }
            }
            _ => {
                self.grid.apply(action, ui.get_library_columns().max(1) as usize);
            }
        }
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
                KeyboardEvent::Moved | KeyboardEvent::None => {}
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
        if !printable || self.screen != Screen::Search || ui.get_launching() {
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
        let focused = self.home.focused();
        (focused < self.tiles.len().min(HOME_COUNT)).then_some(focused)
    }

    /// Index in `entries` des Eintrags, der auf dem aktuellen Bildschirm fokussiert ist.
    fn focused_entry(&self) -> Option<usize> {
        match self.screen {
            Screen::Home => self.home_entry(),
            Screen::Library => self.library.get(self.grid.focused()).copied(),
            Screen::Search => self.results.get(self.result_row.focused()).copied(),
        }
    }

    fn is_running(&self, index: usize) -> bool {
        self.running.as_ref().is_some_and(|r| r.entry_id == self.entries[index].id)
    }

    // --- Modelle und Anzeige ---------------------------------------------------------------

    /// Baut alle Kachelmodelle neu auf (nach Start, Ende einer App oder Umsortierung).
    fn refresh(&mut self, ui: &AppWindow) {
        let mut home: Vec<AppTile> = self.tiles.iter().take(HOME_COUNT).cloned().collect();
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
        self.library = library_view(&self.entries, self.filter, self.sort);
        self.grid.set_len(self.library.len());
        self.library_model.set_vec(self.library.iter().map(|&i| self.tiles[i].clone()).collect::<Vec<_>>());
        ui.set_filter_label(self.filter.label().into());
        ui.set_sort_label(self.sort.label().into());
    }

    fn refresh_results(&mut self, ui: &AppWindow) {
        let keep = self.results.get(self.result_row.focused()).map(|&i| self.entries[i].id.clone());
        self.results = search::search(&self.entries, &self.query);
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
        match self.screen {
            Screen::Home => {
                hints.push(hint(GlyphButton::Dpad, "Navigieren"));
                match self.home_entry() {
                    Some(i) if self.is_running(i) => {
                        hints.push(hint(GlyphButton::Confirm, "Anzeigen"));
                        hints.push(hint(GlyphButton::Menu, "Beenden"));
                    }
                    Some(i) => hints.push(hint(GlyphButton::Confirm, start_label(i))),
                    None => hints.push(hint(GlyphButton::Confirm, "Bibliothek öffnen")),
                }
                hints.push(hint(GlyphButton::North, "Suchen"));
                hints.push(hint(GlyphButton::R1, "Bibliothek"));
                if keyboard {
                    hints.push(hint(GlyphButton::Quit, "Shell beenden"));
                }
            }
            Screen::Library => {
                hints.push(hint(GlyphButton::Dpad, "Navigieren"));
                if let Some(i) = self.focused_entry() {
                    hints.push(hint(GlyphButton::Confirm, start_label(i)));
                }
                hints.push(hint(GlyphButton::West, "Filter"));
                hints.push(hint(GlyphButton::Menu, "Sortierung"));
                hints.push(hint(GlyphButton::North, "Suchen"));
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

    // --- Starten, Verfolgen, Beenden -------------------------------------------------------

    fn launch(&mut self, ui: &AppWindow, index: usize) {
        let hero = self.hero(index);
        let entry = &self.entries[index];
        let tile = &self.tiles[index];
        ui.set_launch_title(tile.title.clone());
        ui.set_launch_image(hero.unwrap_or_else(|| tile.cover.clone()));
        ui.set_launch_accent(tile.accent);

        // Läuft die App schon, zeigen wir nur den Startbildschirm, statt sie ein zweites Mal zu starten.
        if self.is_running(index) {
            ui.set_launch_status("läuft".into());
            ui.set_launching(true);
            return;
        }

        let now = Instant::now();
        let id = entry.id.clone();
        match launch::spawn(&entry.launch) {
            Ok(handle) => {
                let (app, stop) = match entry.launch {
                    Launch::Steam(app_id) => (RunningApp::steam(app_id, now), Stop::Steam(app_id)),
                    _ => (RunningApp::process(handle.clone(), now), Stop::Process(handle)),
                };
                // Es wird nur die zuletzt gestartete App verfolgt.
                if let Some(previous) = self.running.take() {
                    self.set_running_flag(&previous.entry_id, false);
                }
                self.running = Some(Running { entry_id: id.clone(), app, stop });
                ui.set_launch_status("wird gestartet …".into());
                ui.set_launching(true);
                self.set_running_flag(&id, true);
                self.record_launch(index);
                self.home.focus(0);
                self.refresh(ui);
                self.focus_entry(&id);
            }
            Err(err) => {
                eprintln!("{} konnte nicht gestartet werden: {err}", entry.title);
                ui.set_launch_status("konnte nicht gestartet werden".into());
                ui.set_launching(true);
                self.failed_launch = Some(now);
            }
        }
    }

    /// Fokus in Bibliothek und Suche auf den Eintrag setzen (nach dem Umsortieren).
    fn focus_entry(&mut self, id: &str) {
        if let Some(pos) = self.library.iter().position(|&i| self.entries[i].id == id) {
            self.grid.focus(pos);
        }
        if let Some(pos) = self.results.iter().position(|&i| self.entries[i].id == id) {
            self.result_row.focus(pos);
        }
    }

    /// Prüft regelmäßig, ob die gestartete App noch läuft. In gamescope wird die Shell wieder
    /// sichtbar, sobald das App-Fenster zu ist; dann fährt hier der Startbildschirm zurück.
    pub fn watch_running(&mut self, ui: &AppWindow) {
        if let Some(failed) = self.failed_launch
            && failed.elapsed() >= MIN_HOLD
        {
            self.failed_launch = None;
            ui.set_launching(false);
        }
        let Some(running) = self.running.as_mut() else { return };
        let steam_running = if running.app.is_steam() { steam::running_app_id() } else { None };
        match running.app.phase(Instant::now(), steam_running) {
            Phase::Starting => {}
            Phase::Running => {
                if ui.get_launching() {
                    ui.set_launch_status("läuft".into());
                }
            }
            Phase::Finished => {
                let id = running.entry_id.clone();
                self.running = None;
                ui.set_launching(false);
                if self.confirm_quit.as_deref() == Some(id.as_str()) {
                    self.close_confirm(ui);
                }
                if self.set_running_flag(&id, false) {
                    self.refresh(ui);
                }
            }
        }
    }

    /// Setzt das „Läuft“-Badge und die Unterzeile; gibt zurück, ob es den Eintrag gab.
    fn set_running_flag(&mut self, id: &str, running: bool) -> bool {
        let Some(index) = self.entries.iter().position(|e| e.id == id) else { return false };
        let tile = &mut self.tiles[index];
        tile.running = running;
        tile.subtitle =
            if running { "Läuft gerade".into() } else { tiles::subtitle(&self.entries[index], tiles::today()).into() };
        true
    }

    fn ask_quit(&mut self, ui: &AppWindow, index: usize) {
        if !self.is_running(index) {
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
    fn quit_running(&mut self, id: &str) {
        let Some(running) = self.running.as_ref().filter(|r| r.entry_id == id) else { return };
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
