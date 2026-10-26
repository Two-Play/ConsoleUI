//! Shell state: entries, screens, focus, history, running apps and the connection to the UI
//! (main window; in gamescope also the overlay window and window control; in our own compositor
//! the control socket).

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use consoleui_core::dialog::{Dialog, DialogButton, DialogOutcome};
use consoleui_core::game_options::{GameOptions, GameOptionsPanel, GameOptionsStore, PanelOutcome};
use consoleui_core::i18n::{self, Language, tr, trf};
use consoleui_core::keymap::KeyboardLayout;
use consoleui_core::launch::{self, ProcessHandle};
use consoleui_core::netpages::Page;
use consoleui_core::osk::{KeyboardEvent, OnScreenKeyboard};
use consoleui_core::ownership::{self, FIRST_DESKTOP_APP_ID, Owner, SHELL_APP_ID};
use consoleui_core::profile_picker::{PickerOutcome, ProfilePicker};
use consoleui_core::profiles::{self, ProfilePaths, Profiles};
use consoleui_core::protocol::{BlurRegion, CompositorMessage, Rect, ShellMessage, Transition, WindowRole};
use consoleui_core::running::{MIN_HOLD, Phase, RunningApp};
use consoleui_core::settings::{BackgroundChoice, GlyphChoice, Setting, Settings};
use consoleui_core::sound::Sound;
use consoleui_core::system;
use consoleui_core::{
    Action, AppEntry, AppKind, Carousel, Collection, Grid, History, HomeArea, HomeOutcome, Launch, LibraryFilter,
    LibrarySort, apply_history, compare_recent, home_view, library_view, localize_entries, search, steam,
};
use slint::{ComponentHandle, Image, Model, ModelRc, VecModel};

use crate::audio::Audio;
use crate::backdrop::Backdrop;
use crate::compositor::Compositor;
use crate::context::{ContextAction, ContextMenu, ContextOutcome, EntryState};
use crate::gamescope::{Gamescope, x11_window_id};
use crate::hero::Heroes;
use crate::loader::Library;
use crate::music::Music;
use crate::network::Network;
use crate::profiles::{self as profile_view, ProfileStart};
use crate::quick::{ForegroundApp, QuickAction, QuickMenu, QuickOutcome};
use crate::{
    AppTile, AppWindow, ControllerStyle, Glass, GlyphButton, Hint, KeyCap, NotificationWindow, OverlayWindow,
    QuickItem, Screen, SettingRow, Theme, tiles,
};

/// Number of entries the home row shows before the library tile (pinned ones always all).
const HOME_COUNT: usize = 12;
/// Buttons below the info on the home screen: the main action and "more options"
const HOME_BUTTONS: usize = 2;
/// The tabs of the top bar, left to right
/// What a button of the modal dialog does.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DialogTag {
    /// Quit the running entry with this ID
    QuitApp(String),
    Close,
}

const HOME_TABS: [Screen; 3] = [Screen::Home, Screen::Library, Screen::Settings];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchArea {
    Keyboard,
    Results,
}

enum Stop {
    Process(ProcessHandle),
    Steam(u32),
}

/// A launched app, as long as it is running (or still starting).
struct Running {
    entry_id: String,
    /// App ID for gamescope (`STEAM_GAME`)
    app_id: u32,
    app: RunningApp,
    stop: Stop,
    /// How its processes are recognized (for window assignment)
    owner: Option<Owner>,
    /// Already assigned a window in gamescope (or shown by our compositor)?
    has_window: bool,
}

/// Only in the gamescope session: window control, the overlay window for the quick menu and the
/// notification window for toasts over running apps.
pub struct GamescopeUi {
    pub control: Gamescope,
    pub overlay: OverlayWindow,
    /// X11 ID the overlay is prepared for (changes if Slint recreates the window)
    overlay_window: Option<u32>,
    notification: NotificationWindow,
    /// Like `overlay_window`, for the notification window
    notification_window: Option<u32>,
}

impl GamescopeUi {
    pub fn new(control: Gamescope, overlay: OverlayWindow, notification: NotificationWindow) -> Self {
        Self { control, overlay, overlay_window: None, notification, notification_window: None }
    }
}

/// Only in our own compositor (Story 7.6): the quick menu and toasts as separate toplevels. The
/// compositor knows them by their titles (`WindowRole`), stacks them above the app in front and
/// keeps toasts from ever taking input. Used only if the compositor supports it.
pub struct CompositorUi {
    pub overlay: OverlayWindow,
    notification: NotificationWindow,
}

impl CompositorUi {
    pub fn new(overlay: OverlayWindow, notification: NotificationWindow) -> Self {
        Self { overlay, notification }
    }
}

pub struct Shell {
    entries: Vec<AppEntry>,
    /// Tile per entry, in the same order as `entries`
    tiles: Vec<AppTile>,
    library_tile: AppTile,
    home_model: Rc<VecModel<AppTile>>,
    library_model: Rc<VecModel<AppTile>>,
    results_model: Rc<VecModel<AppTile>>,

    screen: Screen,
    home: Carousel,
    /// On the home screen: the tile row, the buttons below it or the tabs of the top bar
    home_area: HomeArea,
    /// Indices into `entries` for the home row (pinned first, without hidden ones)
    home_entries: Vec<usize>,
    grid: Grid,
    /// Indices into `entries` for the library (filtered and sorted)
    library: Vec<usize>,
    filter: LibraryFilter,
    sort: LibrarySort,

    query: String,
    /// Indices into `entries` for the search results
    results: Vec<usize>,
    result_row: Carousel,
    keyboard: OnScreenKeyboard,
    search_area: SearchArea,
    /// Where "Back" from the search leads
    search_origin: Screen,

    history: History,
    history_path: Option<PathBuf>,
    /// Pinned and hidden entries
    collection: Collection,
    collection_path: Option<PathBuf>,
    /// Open context menu for the focused entry
    context: Option<ContextMenu>,
    /// Nested gamescope options per entry (Story 7.10)
    game_options: GameOptionsStore,
    game_options_path: Option<PathBuf>,
    /// Open options panel (from the context menu)
    game_options_panel: Option<GameOptionsPanel>,
    /// Hero images, decoded in the background and cached with a limit
    heroes: Heroes,

    running: Vec<Running>,
    next_app_id: u32,
    /// App currently in front in gamescope (`None`: the shell)
    foreground: Option<u32>,
    /// Time of a failed launch; the launch screen shows the error briefly.
    failed_launch: Option<Instant>,
    /// Our compositor zooms the app in over the launch screen (Story 7.7); it closes afterwards.
    close_launch_at: Option<Instant>,
    /// The launch screen is still opening: the launched app (`foreground`) is handed to gamescope
    /// or our compositor once a frame was rendered after this time (the screen is fully open), so
    /// it never shows up over a half-open launch screen.
    front_due: Option<Instant>,
    /// A frame was rendered after `front_due`
    launch_shown: bool,
    /// Entry ID whose quitting is waiting for confirmation
    dialog: Option<Dialog<DialogTag>>,
    /// Entries launched but not yet moved to the front of the lists: they move once the shell is
    /// in front again (the launch screen opens at the tile's old place, and the tile glides forward
    /// after the return)
    pending_front: Vec<String>,
    quick: Option<QuickMenu>,
    gamescope: Option<GamescopeUi>,
    /// Our own compositor (Epic 7), instead of gamescope
    compositor: Option<Compositor>,
    /// Overlay and notification windows for our compositor
    compositor_ui: Option<CompositorUi>,

    settings: Settings,
    settings_path: Option<PathBuf>,
    settings_row: usize,
    audio: Option<Audio>,
    /// Background music; plays only while the shell is in front
    music: Music,
    /// Animated background (settings row "Background")
    backdrop: Backdrop,
    /// Counts toasts up so that an older hide timer does not close a newer toast
    toast_generation: Rc<Cell<u64>>,
    /// Counts quick menu openings up, so that the hide timer of a closing menu does not hide the
    /// window of one opened again meanwhile
    quick_generation: Rc<Cell<u64>>,
    /// The quick menu's cards; kept across actions so that only changed cards are updated and the
    /// focus animates instead of the whole row being rebuilt
    quick_items: Rc<VecModel<QuickItem>>,
    /// The quick menu was last shown open (`show_quick`)
    quick_shown: bool,
    /// "Controller battery low" warning already shown
    battery_warned: bool,
    /// Wi-Fi and Bluetooth sub-pages of the settings
    network: Network,

    /// All profiles (Story 5.7); history, collection, game options and settings belong to the
    /// active one
    profiles: Profiles,
    /// `$XDG_DATA_HOME/consoleui` (`None`: nothing is saved)
    profiles_root: Option<PathBuf>,
    /// ID of the active profile
    profile: String,
    /// Open "Who's playing?" screen
    picker: Option<ProfilePicker>,
    /// Made-up entries: nothing of the library is saved for them
    dummy: bool,
    /// `last_used` as discovered (Steam's own), before a profile's history is applied
    discovered_last_used: HashMap<String, Option<u64>>,
}

/// How long a toast stays visible
const TOAST_DURATION: Duration = Duration::from_millis(3500);
/// Below this battery level the shell warns once
const LOW_BATTERY: u8 = 15;

/// What an input changed, to pick the matching sound
#[derive(PartialEq)]
struct Feedback {
    screen: Screen,
    quick_open: bool,
    /// Context menu or game options panel open
    context_open: bool,
    /// Focus and values of the game options panel
    game_options: Option<(usize, GameOptions)>,
    launching: bool,
    focus: [usize; 8],
    home_area: HomeArea,
    query_len: usize,
    settings: Settings,
    network: [usize; 5],
    /// Profile screen open, and its focus
    profiles: Option<[usize; 4]>,
}

/// How long the launch screen stays under an app our compositor zooms in (a bit longer than the
/// compositor's transition)
const LAUNCH_HANDOVER: Duration = Duration::from_millis(700);
/// The launched app comes to the front at the latest this long after the launch screen's
/// animation ended, even if no frame of the open screen was reported
const FRONT_FALLBACK: Duration = Duration::from_millis(300);

/// Initial animation durations in milliseconds (as in `theme.slint`)
const DURATIONS: [i64; 5] = [220, 500, 120, 450, 200];

impl Shell {
    /// Starts without entries; the library arrives via [`Shell::set_library`] once it is loaded.
    pub fn new(
        ui: &AppWindow,
        gamescope: Option<GamescopeUi>,
        compositor: Option<Compositor>,
        compositor_ui: Option<CompositorUi>,
        start: ProfileStart,
    ) -> Self {
        let settings_path = start.paths().map(|paths| paths.settings);
        let keyboard = OnScreenKeyboard::default();
        show_keys(ui, &keyboard);

        let home_model = Rc::new(VecModel::default());
        let library_model = Rc::new(VecModel::default());
        let results_model = Rc::new(VecModel::default());
        ui.set_home_tiles(ModelRc::from(home_model.clone()));
        ui.set_library_tiles(ModelRc::from(library_model.clone()));
        ui.set_results(ModelRc::from(results_model.clone()));

        if let Some(gamescope) = &gamescope {
            // In Steam mode nothing at all is visible without a baselayer.
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
            home_area: HomeArea::Tiles,
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
            game_options: GameOptionsStore::default(),
            game_options_path: None,
            game_options_panel: None,
            heroes: Heroes::new(ui),
            running: Vec::new(),
            next_app_id: FIRST_DESKTOP_APP_ID,
            foreground: None,
            failed_launch: None,
            close_launch_at: None,
            front_due: None,
            launch_shown: false,
            dialog: None,
            pending_front: Vec::new(),
            quick: None,
            gamescope,
            compositor,
            compositor_ui,
            settings: start.settings,
            settings_path,
            settings_row: 0,
            audio: Audio::start(),
            music: Music::start(),
            backdrop: Backdrop::new(),
            toast_generation: Rc::default(),
            quick_generation: Rc::default(),
            quick_items: Rc::default(),
            quick_shown: false,
            battery_warned: false,
            network: Network::default(),
            profiles: start.profiles,
            profiles_root: start.root,
            profile: start.active,
            picker: None,
            dummy: false,
            discovered_last_used: HashMap::new(),
        };
        shell.show_profiles(ui);
        shell.apply_settings(ui);
        shell.send_keyboard_layout();
        shell.refresh(ui);
        shell
    }

    /// Takes over the library loaded in the background.
    pub fn set_library(&mut self, ui: &AppWindow, library: Library) {
        let today = tiles::today();
        self.tiles = library.entries.iter().enumerate().map(|(i, loaded)| tiles::to_tile(loaded, i, today)).collect();
        self.entries = library.entries.into_iter().map(|loaded| loaded.entry).collect();
        self.library_tile = tiles::library_tile(self.entries.len());
        self.dummy = library.dummy;
        self.discovered_last_used = self.entries.iter().map(|e| (e.id.clone(), e.last_used)).collect();
        self.load_profile_data();
        self.heroes.invalidate();
        self.refresh(ui);
    }

    // --- Profiles (Story 5.7) -------------------------------------------------------------

    fn profile_paths(&self) -> Option<ProfilePaths> {
        self.profiles_root.as_deref().map(|root| ProfilePaths::new(root, &self.profile))
    }

    /// Loads history, collection and game options of the active profile (not for dummy entries)
    /// and sorts the entries by its history.
    fn load_profile_data(&mut self) {
        let paths = self.profile_paths().filter(|_| !self.dummy);
        self.history_path = paths.as_ref().map(|p| p.history.clone());
        self.history = self.history_path.as_deref().map(History::load).unwrap_or_default();
        self.collection_path = paths.as_ref().map(|p| p.collection.clone());
        self.collection = self.collection_path.as_deref().map(Collection::load).unwrap_or_default();
        self.game_options_path = paths.map(|p| p.game_options);
        self.game_options = self.game_options_path.as_deref().map(GameOptionsStore::load).unwrap_or_default();
        for entry in &mut self.entries {
            entry.last_used = self.discovered_last_used.get(&entry.id).copied().flatten();
        }
        apply_history(&mut self.entries, &self.history);
        self.resort();
        for index in 0..self.tiles.len() {
            self.relabel_tile(index);
        }
    }

    /// Opens the profile screen; `startup`: "Who's playing?" before the shell, without a way back.
    pub fn open_profiles(&mut self, ui: &AppWindow, startup: bool) {
        self.close_context(ui);
        self.close_game_options(ui);
        self.picker = Some(ProfilePicker::new(&self.profiles, Some(&self.profile), startup));
        self.show_profiles(ui);
        self.sync(ui);
    }

    /// Number of profiles; with more than one the shell asks who is playing at the start.
    pub fn profile_count(&self) -> usize {
        self.profiles.len()
    }

    fn show_profiles(&self, ui: &AppWindow) {
        let view = self.picker.as_ref().map(|picker| picker.view(&self.profiles));
        ui.set_profile_view(profile_view::to_view(view, self.profiles.get(&self.profile)));
    }

    fn save_profiles(&self) {
        if let Some(root) = &self.profiles_root
            && let Err(err) = self.profiles.save(root)
        {
            eprintln!("Cannot save the profiles ({}): {err}", root.display());
        }
    }

    fn apply_picker(&mut self, ui: &AppWindow, action: Action) {
        let Some(picker) = self.picker.as_mut() else { return };
        match picker.apply(&mut self.profiles, action) {
            PickerOutcome::Stay => {}
            PickerOutcome::Close => self.picker = None,
            PickerOutcome::Select(id) => self.switch_profile(ui, id),
            PickerOutcome::Changed => self.save_profiles(),
            PickerOutcome::Deleted(id) => {
                self.save_profiles();
                if let Some(root) = &self.profiles_root
                    && let Err(err) = profiles::remove_files(root, &id)
                {
                    eprintln!("Cannot delete the files of profile {id}: {err}");
                }
            }
        }
        self.show_profiles(ui);
    }

    /// Continues as profile `id`: its settings (language, accent, ...), history, pinned entries
    /// and game options replace the current ones. Running apps keep running.
    fn switch_profile(&mut self, ui: &AppWindow, id: String) {
        self.picker = None;
        self.profiles.set_last(&id);
        self.save_profiles();
        if id == self.profile {
            return;
        }
        self.profile = id;
        self.settings_path = self.profile_paths().map(|paths| paths.settings);
        self.settings = self.settings_path.as_deref().map(Settings::load).unwrap_or_default();
        self.load_profile_data();
        self.screen = Screen::Home;
        self.home.focus(0);
        self.grid.focus(0);
        self.settings_row = 0;
        self.heroes.invalidate();
        self.apply_settings(ui);
        self.apply_language(ui);
        self.send_keyboard_layout();
        if let Some(profile) = self.profiles.get(&self.profile) {
            let title = trf("Hello, {}!", &[&profile.name]);
            self.toast(ui, "☺", &title, tr("Your games and settings are loaded."));
        }
    }

    /// Shows a toast at the top right; a new one replaces the old one. If an app is in front in
    /// gamescope or our compositor, it appears in the notification window above it, otherwise in
    /// the main window.
    pub fn toast(&mut self, ui: &AppWindow, symbol: &str, title: &str, text: &str) {
        let generation = self.toast_generation.get() + 1;
        self.toast_generation.set(generation);
        let over_app = !self.shell_in_front(ui);
        if let Some(notification) = self.notification_window().filter(|_| over_app).map(|n| n.clone_strong()) {
            ui.set_toast_shown(false);
            notification.set_symbol(symbol.into());
            notification.set_title_text(title.into());
            notification.set_text(text.into());
            if notification.window().is_visible() {
                notification.set_shown(true);
            } else {
                match &mut self.gamescope {
                    Some(gamescope) => show_notification_window(gamescope),
                    None => show_compositor_notification(&notification, self.output_size(ui)),
                }
                // Fade in only after showing, so the toast slides in.
                let weak = notification.as_weak();
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
        let notification = self.notification_window().map(|n| n.as_weak());
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

    /// Hides a toast in the notification window (if one is visible).
    fn hide_notification(&self) {
        if let Some(notification) = self.notification_window()
            && notification.get_shown()
        {
            hide_notification_window(notification);
        }
    }

    /// The overlay and notification windows of our compositor, if it places them (Story 7.6).
    fn compositor_ui(&self) -> Option<&CompositorUi> {
        self.compositor_ui.as_ref().filter(|_| self.compositor.as_ref().is_some_and(Compositor::overlays))
    }

    /// Tells our compositor where the quick menu bar and the toast are, so it blurs what is below
    /// them (Story 7.13). Called after every frame of their windows, so the blur moves with the
    /// sliding panel, and from the compositor poll; only changes are sent. The region of a frame
    /// can reach the compositor one frame before its buffer; on the console session that is not
    /// visible, nested in KWin it showed as a flickering edge (then `CONSOLEUI_NO_BLUR=1`).
    pub fn sync_blur(&mut self) {
        let Some(windows) = self.compositor_ui() else { return };
        let Some(blur) = self.compositor.as_ref().map(Compositor::blur) else { return };
        windows.overlay.global::<Glass>().set_backdrop(blur);
        windows.notification.global::<Glass>().set_backdrop(blur);
        if !blur {
            return;
        }
        let region = |x: f32, y: f32, width: f32, height: f32, radius: f32| BlurRegion {
            rect: Rect {
                x: x.round() as i32,
                y: y.round() as i32,
                width: width.round() as i32,
                height: height.round() as i32,
            },
            radius: radius.round() as i32,
        };
        let overlay = &windows.overlay;
        let mut menu = Vec::new();
        if overlay.window().is_visible() {
            let size = overlay.window().size().to_logical(overlay.window().scale_factor());
            let top = overlay.get_panel_y();
            let height = overlay.get_panel_height().min(size.height - top);
            if height >= 1.0 {
                menu.push(region(0.0, top, size.width, height, 0.0));
            }
        }
        let notification = &windows.notification;
        let mut toast = Vec::new();
        if notification.get_shown() && notification.window().is_visible() {
            toast.push(region(
                notification.get_toast_x(),
                notification.get_toast_y(),
                notification.get_toast_width(),
                notification.get_toast_height(),
                notification.get_toast_radius(),
            ));
        }
        let Some(compositor) = &mut self.compositor else { return };
        compositor.set_blur(WindowRole::Overlay, menu, 0);
        compositor.set_blur(WindowRole::Notification, toast, 0);
    }

    /// The overlay and notification windows whose frames move the compositor's blur (Story 7.13).
    pub fn blur_windows(&self) -> Option<(slint::Weak<OverlayWindow>, slint::Weak<NotificationWindow>)> {
        let windows = self.compositor_ui.as_ref()?;
        Some((windows.overlay.as_weak(), windows.notification.as_weak()))
    }

    /// The window for toasts over running apps (gamescope or our compositor).
    fn notification_window(&self) -> Option<&NotificationWindow> {
        match &self.gamescope {
            Some(gamescope) => Some(&gamescope.notification),
            None => self.compositor_ui().map(|c| &c.notification),
        }
    }

    /// Warns once when the controller battery runs low; after charging it warns again.
    pub fn check_controller_battery(&mut self, ui: &AppWindow, percent: Option<u8>) {
        match percent {
            Some(p) if p <= LOW_BATTERY && !self.battery_warned => {
                self.battery_warned = true;
                self.toast(ui, "▁", tr("Controller battery low"), &trf("{} % left", &[&p]));
            }
            Some(p) if p > LOW_BATTERY + 5 => self.battery_warned = false,
            _ => {}
        }
    }

    /// Button glyphs chosen in the settings (`Auto`: by the controller).
    pub fn glyph_choice(&self) -> GlyphChoice {
        self.settings.glyphs
    }

    /// Sets the start screen (developer option `--start`).
    pub fn open(&mut self, ui: &AppWindow, screen: Screen) {
        match screen {
            Screen::Search => self.open_search(ui),
            other => self.screen = other,
        }
        self.sync(ui);
    }

    /// Number of entries in the home row (including the library tile), for the benchmark.
    pub fn home_len(&self) -> usize {
        self.home_entries.len() + 1
    }

    /// While the control center is open, the shell reads the controllers exclusively.
    pub fn wants_exclusive_input(&self) -> bool {
        self.quick.is_some()
    }

    /// Whether a controller input goes to the shell. gilrs reads the devices directly, so the shell
    /// would otherwise also get buttons meant for the game in front. In the quick menu everything
    /// counts, the PS button always. In gamescope it depends on whether the shell is in front (the X11
    /// focus is unreliable there after the overlay), on the desktop on the window focus.
    pub fn accepts_gamepad(&self, ui: &AppWindow, action: Action) -> bool {
        if self.quick.is_some() {
            return true;
        }
        if self.gamescope.is_none() && self.compositor.is_none() {
            return ui.get_window_active();
        }
        action == Action::Guide || self.shell_in_front(ui)
    }

    /// In gamescope: the shell is visible while no app is in front, the launch screen is running
    /// or the app in front has no window yet.
    fn shell_in_front(&self, ui: &AppWindow) -> bool {
        match self.foreground {
            None => true,
            Some(app_id) => ui.get_launching() || !self.running.iter().any(|r| r.app_id == app_id && r.has_window),
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
            context_open: self.context.is_some() || self.game_options_panel.is_some(),
            game_options: self.game_options_panel.as_ref().map(|p| (p.focused(), p.options())),
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
            home_area: self.home_area,
            query_len: self.query.len(),
            settings: self.settings,
            network: self.network.pages.feedback(),
            profiles: self.picker.as_ref().map(ProfilePicker::feedback),
        }
    }

    fn play_feedback(&mut self, action: Action, before: &Feedback, after: &Feedback) {
        if before == after {
            return;
        }
        let sound = if after.quick_open != before.quick_open {
            if after.quick_open { Sound::Open } else { Sound::Back }
        } else if after.profiles.is_some() != before.profiles.is_some() {
            if after.profiles.is_some() { Sound::Open } else { Sound::Confirm }
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
            self.close_game_options(ui);
            self.open_quick(ui);
            return;
        }
        if self.picker.is_some() {
            self.apply_picker(ui, action);
            self.sync(ui);
            return;
        }
        if self.dialog.is_some() {
            self.apply_dialog(ui, action);
            return;
        }
        if ui.get_launching() {
            // "Back" brings up the overview; the app keeps running and stays tracked.
            if action == Action::Back {
                self.show_shell(ui);
            }
            return;
        }
        if self.context.is_some() {
            self.apply_context(ui, action);
            self.sync(ui);
            return;
        }
        if self.game_options_panel.is_some() {
            self.apply_game_options(ui, action);
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
            (Screen::Library | Screen::Settings, _) if matches!(self.home_area, HomeArea::Tabs(_)) => {
                self.apply_tabs(action)
            }
            (Screen::Home, _) => self.apply_home(ui, action),
            (Screen::Library, _) => self.apply_library(ui, action),
            (Screen::Settings, _) => self.apply_settings_page(ui, action),
            (Screen::Search, _) => self.apply_search(ui, action),
        }
        self.sync(ui);
    }

    fn apply_home(&mut self, ui: &AppWindow, action: Action) {
        // Up from the row reaches the tabs, down the buttons below the info.
        let action = match self.home_area.apply(action, HOME_BUTTONS, HOME_TABS.len()) {
            HomeOutcome::Pass => action,
            HomeOutcome::Press(0) => Action::Confirm,
            HomeOutcome::Press(_) => Action::Menu,
            HomeOutcome::OpenTab(tab) => {
                self.screen = HOME_TABS[tab];
                return;
            }
            HomeOutcome::Moved | HomeOutcome::Ignored => return,
        };
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

    /// Selection on the tab bar of the library or settings (up from the first row): ◀ ▶ picks a
    /// tab, ✕ opens it, down or ○ goes back to the page.
    fn apply_tabs(&mut self, action: Action) {
        if let HomeOutcome::OpenTab(tab) = self.home_area.apply(action, 0, HOME_TABS.len()) {
            self.screen = HOME_TABS[tab];
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
            // Sorting has been part of the context menu since Story 2.7.
            Action::Menu => self.open_context(ui),
            _ => {
                // Up from the first row reaches the tabs.
                if !self.grid.apply(action, ui.get_library_columns().max(1) as usize) && action == Action::Up {
                    self.home_area = HomeArea::Tabs(1);
                }
            }
        }
    }

    fn apply_settings_page(&mut self, ui: &AppWindow, action: Action) {
        let rows = self.settings_list();
        let setting = rows[self.settings_row.min(rows.len() - 1)];
        match action {
            Action::Up if self.settings_row == 0 => self.home_area = HomeArea::Tabs(2),
            Action::Up => self.settings_row -= 1,
            Action::Down => self.settings_row = (self.settings_row + 1).min(rows.len() - 1),
            Action::Confirm if setting.opens_page() => {
                let page = if setting == Setting::Wifi { Page::Wifi } else { Page::Bluetooth };
                let requests = self.network.pages.open(page);
                self.network.send(requests);
                ui.set_net_view(self.network.view());
            }
            Action::Left | Action::Right if setting.opens_page() => {}
            Action::Left | Action::Right | Action::Confirm => {
                self.settings.change(setting, action != Action::Left);
                if setting == Setting::Language {
                    self.apply_language(ui);
                }
                if setting == Setting::KeyboardLayout {
                    self.send_keyboard_layout();
                }
                if let Some(path) = &self.settings_path
                    && let Err(err) = self.settings.save(path)
                {
                    eprintln!("Cannot save the settings ({}): {err}", path.display());
                }
                self.apply_settings(ui);
            }
            Action::Back => self.screen = Screen::Home,
            _ => {}
        }
    }

    /// Pushes settings to the UI: accent color, animation durations, settings rows.
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
        if let Some(windows) = &self.compositor_ui {
            apply_theme(windows.overlay.global::<Theme>());
            apply_theme(windows.notification.global::<Theme>());
        }
        self.update_settings_rows(ui);
        let footer = match &self.settings_path {
            Some(path) => trf("Saved automatically: {}", &[&path.display()]),
            None => tr("Settings are not saved").into(),
        };
        ui.set_settings_footer(footer.into());
    }

    /// Switches the language (Story 6.6). Slint updates its `@tr()` texts by itself; everything
    /// built in Rust is rebuilt here, including the names and descriptions of the entries
    /// (`Name[de]` of desktop files, dummy entries), which are re-resolved from the variants kept
    /// on `AppEntry`: no rediscovery and no image reloading. Search results and the A-Z library
    /// follow in `refresh`, since they are computed from the entries' current titles.
    fn apply_language(&mut self, ui: &AppWindow) {
        let language = self.settings.language.resolve();
        select_language(language);
        self.keyboard = OnScreenKeyboard::default();
        show_keys(ui, &self.keyboard);
        if localize_entries(&mut self.entries, language) {
            self.resort();
        }
        for index in 0..self.tiles.len() {
            self.relabel_tile(index);
        }
        self.library_tile = tiles::library_tile(self.entries.len());
        self.refresh(ui);
        ui.set_net_view(self.network.view());
    }

    /// Rows of the settings page; the keyboard layout only in our compositor.
    fn settings_list(&self) -> Vec<Setting> {
        Setting::rows(self.compositor.is_some())
    }

    /// Tells our compositor the keyboard layout from the settings (`None`: the system layout).
    fn send_keyboard_layout(&mut self) {
        let layout = self.settings.keyboard_layout.code().map(String::from);
        if let Some(compositor) = &mut self.compositor {
            compositor.send(&ShellMessage::KeyboardLayout { layout });
        }
    }

    fn update_settings_rows(&self, ui: &AppWindow) {
        let keymap = self.compositor.as_ref().and_then(Compositor::keymap);
        let rows: Vec<SettingRow> = self
            .settings_list()
            .into_iter()
            .map(|setting| SettingRow {
                label: setting.label().into(),
                description: setting.description().into(),
                value: match setting {
                    Setting::Wifi => self.network.pages.summary(Page::Wifi).into(),
                    Setting::Bluetooth => self.network.pages.summary(Page::Bluetooth).into(),
                    // Shows what "Automatic" resolved to, e.g. "Automatic: de (nodeadkeys)"
                    Setting::KeyboardLayout if self.settings.keyboard_layout == KeyboardLayout::Auto => match &keymap {
                        Some(keymap) => trf("Automatic: {}", &[&keymap]).into(),
                        None => self.settings.value_label(setting).into(),
                    },
                    _ => self.settings.value_label(setting).into(),
                },
                opens_page: setting.opens_page(),
            })
            .collect();
        ui.set_settings_rows(ModelRc::new(VecModel::from(rows)));
    }

    // --- Wi-Fi and Bluetooth (Stories 5.3/5.4) ---------------------------------------------

    /// Starts the network thread (`fake`: simulated, changes nothing on the system) and reads once.
    pub fn start_network(&mut self, fake: bool) {
        self.network.start(fake);
        self.network.send(vec![consoleui_core::network::Request::Refresh]);
    }

    /// Runs on a timer: takes over responses of the network thread and shows toasts.
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

    /// Input from the real keyboard in the search. Control characters and special keys (Slint reports
    /// them as characters from the Private Use Area) are rejected.
    pub fn type_text(&mut self, ui: &AppWindow, text: &str) -> bool {
        let printable =
            !text.is_empty() && text.chars().all(|c| !c.is_control() && !('\u{e000}'..='\u{f8ff}').contains(&c));
        if printable && self.quick.is_none() && self.picker.as_mut().is_some_and(|p| p.type_text(text)) {
            self.show_profiles(ui);
            return true;
        }
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
        if self.picker.as_mut().is_some_and(ProfilePicker::erase) {
            self.show_profiles(ui);
            return;
        }
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

    /// Index in `entries` of the entry focused on the current screen.
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

    // --- Models and display ----------------------------------------------------------------

    /// Rebuilds all tile models (after a launch, an app exit or a re-sort).
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

    /// Pushes screen, focus, background and hints to the UI.
    fn sync(&mut self, ui: &AppWindow) {
        // The tabs can be focused on the three tab screens; the buttons only on home.
        let allowed = match self.screen {
            Screen::Home => true,
            Screen::Library | Screen::Settings => matches!(self.home_area, HomeArea::Tiles | HomeArea::Tabs(_)),
            Screen::Search => false,
        };
        if !allowed {
            self.home_area = HomeArea::Tiles;
        }
        let (area, area_index) = match self.home_area {
            HomeArea::Tiles => (0, 0),
            HomeArea::Buttons(i) => (1, i),
            HomeArea::Tabs(i) => (2, i),
        };
        ui.set_home_area(area);
        ui.set_home_area_index(area_index as i32);
        ui.set_screen(self.screen);
        ui.set_home_focused(self.home.focused() as i32);
        ui.set_library_focused(self.grid.focused() as i32);
        ui.set_result_focused(self.result_row.focused() as i32);
        ui.set_key_focused(self.keyboard.focused() as i32);
        ui.set_settings_focused(self.settings_row as i32);
        ui.set_keyboard_active(self.search_area == SearchArea::Keyboard);
        self.show_hero(ui);
        self.update_hints(ui);
        self.update_ambience(ui);
        self.apply_pending_front(ui);
    }

    /// Background music and the animated background run while the shell is in front: no app, no
    /// launch screen, no boot screen.
    fn update_ambience(&mut self, ui: &AppWindow) {
        let in_front = self.foreground.is_none() && !ui.get_launching() && !ui.get_booting();
        self.music.set(self.settings.music, in_front);
        let entry = self.focused_entry();
        let has_artwork = entry.is_some_and(|i| self.entries[i].artwork.hero.is_some());
        let accent = match entry {
            Some(i) => self.tiles[i].accent,
            None => {
                let [r, g, b] = self.settings.accent.rgb();
                slint::Color::from_rgb_u8(r, g, b)
            }
        };
        let shown = self.settings.background.animated(has_artwork);
        // Paused under the quick menu: the shell then only redraws the menu, and the compositor's
        // blur under it stays cached.
        self.backdrop.update(ui, shown, in_front && self.quick.is_none(), self.settings.animations, accent);
    }

    /// Fades in the hero of the focused entry (once it is decoded), prefetches the neighbours
    /// and shifts the background slightly with the focus (parallax, Story 6.3).
    fn show_hero(&mut self, ui: &AppWindow) {
        let layout = self.focus_layout(ui);
        let entry = self.focused_entry().map(|i| &self.entries[i]);
        let prefetch = layout
            .map(|(pos, len, row)| {
                let offsets = [1, -1, row as isize, -(row as isize), 2];
                offsets
                    .iter()
                    .filter_map(|&d| pos.checked_add_signed(d).filter(|&p| p < len && p != pos))
                    .filter_map(|p| self.entry_at(p))
                    .filter_map(|i| {
                        let e = &self.entries[i];
                        e.artwork.hero.clone().map(|path| (e.id.clone(), path))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // "Animated": no artwork over the animated background
        let hero = entry
            .and_then(|e| e.artwork.hero.as_deref())
            .filter(|_| self.settings.background != BackgroundChoice::Animated);
        self.heroes.show(entry.map(|e| e.id.as_str()), hero, prefetch);

        let parallax = match layout {
            Some((pos, len, row)) if self.settings.animations => {
                let (col, cols) = if row < len { (pos % row.max(1), row) } else { (pos, len) };
                if cols > 1 { col as f32 / (cols - 1) as f32 * 2.0 - 1.0 } else { 0.0 }
            }
            _ => 0.0,
        };
        ui.set_hero_parallax(parallax);
    }

    /// Focus on the current screen: (position, number of tiles, tiles per row).
    fn focus_layout(&self, ui: &AppWindow) -> Option<(usize, usize, usize)> {
        match self.screen {
            Screen::Home => Some((self.home.focused(), self.home_len(), self.home_len())),
            Screen::Library => {
                Some((self.grid.focused(), self.library.len(), ui.get_library_columns().max(1) as usize))
            }
            Screen::Search => Some((self.result_row.focused(), self.results.len(), self.results.len())),
            Screen::Settings => None,
        }
    }

    /// Index in `entries` of the tile at position `pos` of the current screen.
    fn entry_at(&self, pos: usize) -> Option<usize> {
        match self.screen {
            Screen::Home => (pos < self.tiles.len().min(HOME_COUNT)).then_some(pos),
            Screen::Library => self.library.get(pos).copied(),
            Screen::Search => self.results.get(pos).copied(),
            Screen::Settings => None,
        }
    }

    /// Image for the launch screen: the hero if already decoded, otherwise the cover.
    fn launch_image(&self, index: usize) -> Image {
        self.heroes.cached(&self.entries[index].id).unwrap_or_else(|| self.tiles[index].cover.clone())
    }

    /// Hint bar at the bottom, matching the screen and selection.
    pub fn update_hints(&self, ui: &AppWindow) {
        let hint = |button, label: &str| Hint { button, label: label.into() };
        let keyboard = ui.get_controller_style() == ControllerStyle::Keyboard;
        let start_label = |index: usize| match self.entries[index].kind {
            AppKind::Game => tr("Play"),
            AppKind::App => tr("Open"),
        };
        let mut hints = Vec::new();
        if let Some(picker) = &self.picker {
            if picker.is_typing() {
                if keyboard {
                    hints.push(hint(GlyphButton::Dpad, tr("Type or pick keys")));
                } else {
                    hints.push(hint(GlyphButton::Confirm, tr("Type")));
                    hints.push(hint(GlyphButton::West, tr("Delete")));
                    hints.push(hint(GlyphButton::North, tr("Space")));
                }
                hints.push(hint(GlyphButton::Back, tr("Cancel")));
            } else if picker.is_menu_open() {
                hints.push(hint(GlyphButton::Updown, tr("Select")));
                hints.push(hint(GlyphButton::Confirm, tr("OK")));
                hints.push(hint(GlyphButton::Back, tr("Close")));
            } else if !picker.is_confirming() {
                let view = picker.view(&self.profiles);
                let add = view.tiles.get(view.focused).is_some_and(|t| t.add);
                hints.push(hint(GlyphButton::Dpad, tr("Navigate")));
                if add {
                    hints.push(hint(GlyphButton::Confirm, tr("Add profile")));
                } else {
                    hints.push(hint(GlyphButton::Confirm, tr("Choose")));
                    hints.push(hint(GlyphButton::Menu, tr("Options")));
                }
                if !picker.is_startup() {
                    hints.push(hint(GlyphButton::Back, tr("Back")));
                }
                hints.push(hint(GlyphButton::Guide, tr("Control center")));
            }
            ui.set_hints(ModelRc::new(VecModel::from(hints)));
            return;
        }
        if self.context.is_some() {
            hints.push(hint(GlyphButton::Updown, tr("Select")));
            hints.push(hint(GlyphButton::Confirm, tr("OK")));
            hints.push(hint(GlyphButton::Back, tr("Close")));
            ui.set_hints(ModelRc::new(VecModel::from(hints)));
            return;
        }
        if let Some(panel) = &self.game_options_panel {
            if panel.is_available() {
                hints.push(hint(GlyphButton::Updown, tr("Select")));
                hints.push(hint(GlyphButton::Dpad, tr("Change")));
            }
            hints.push(hint(GlyphButton::Back, tr("Back")));
            ui.set_hints(ModelRc::new(VecModel::from(hints)));
            return;
        }
        if matches!(self.home_area, HomeArea::Tabs(_)) {
            hints.push(hint(GlyphButton::Dpad, tr("Select")));
            hints.push(hint(GlyphButton::Confirm, tr("Open")));
            hints.push(hint(GlyphButton::Back, tr("Back")));
            ui.set_hints(ModelRc::new(VecModel::from(hints)));
            return;
        }
        match self.screen {
            Screen::Home => {
                hints.push(hint(GlyphButton::Dpad, tr("Navigate")));
                match self.home_entry() {
                    Some(i) => {
                        let label = if self.running_for(i).is_some() { tr("Resume") } else { start_label(i) };
                        hints.push(hint(GlyphButton::Confirm, label));
                        hints.push(hint(GlyphButton::Menu, tr("Options")));
                    }
                    None => hints.push(hint(GlyphButton::Confirm, tr("Open library"))),
                }
                hints.push(hint(GlyphButton::North, tr("Search")));
                hints.push(hint(GlyphButton::R1, tr("Library")));
                hints.push(hint(GlyphButton::Guide, tr("Control center")));
                if keyboard {
                    hints.push(hint(GlyphButton::Quit, tr("Quit shell")));
                }
            }
            Screen::Library => {
                hints.push(hint(GlyphButton::Dpad, tr("Navigate")));
                if let Some(i) = self.focused_entry() {
                    let label = if self.running_for(i).is_some() { tr("Resume") } else { start_label(i) };
                    hints.push(hint(GlyphButton::Confirm, label));
                }
                hints.push(hint(GlyphButton::West, tr("Filter")));
                hints.push(hint(GlyphButton::Menu, tr("Options")));
                hints.push(hint(GlyphButton::North, tr("Search")));
                hints.push(hint(GlyphButton::Back, tr("Back")));
            }
            Screen::Settings if self.network.pages.is_open() => {
                hints.extend(self.network.hints().into_iter().map(|(button, label)| hint(button, label)));
            }
            Screen::Settings if self.settings_list().get(self.settings_row).is_some_and(|s| s.opens_page()) => {
                hints.push(hint(GlyphButton::Updown, tr("Select")));
                hints.push(hint(GlyphButton::Confirm, tr("Open")));
                hints.push(hint(GlyphButton::L1, tr("Library")));
                hints.push(hint(GlyphButton::Back, tr("Back")));
            }
            Screen::Settings => {
                hints.push(hint(GlyphButton::Updown, tr("Select")));
                hints.push(hint(GlyphButton::Dpad, tr("Change")));
                hints.push(hint(GlyphButton::L1, tr("Library")));
                hints.push(hint(GlyphButton::Back, tr("Back")));
            }
            Screen::Search if self.search_area == SearchArea::Keyboard => {
                if keyboard {
                    hints.push(hint(GlyphButton::Dpad, tr("Type or pick keys")));
                } else {
                    hints.push(hint(GlyphButton::Confirm, tr("Type")));
                    hints.push(hint(GlyphButton::West, tr("Delete")));
                    hints.push(hint(GlyphButton::North, tr("Space")));
                }
                hints.push(hint(GlyphButton::Back, tr("Close")));
            }
            Screen::Search => {
                hints.push(hint(GlyphButton::Dpad, tr("Navigate")));
                if let Some(i) = self.focused_entry() {
                    hints.push(hint(GlyphButton::Confirm, start_label(i)));
                }
                hints.push(hint(GlyphButton::Back, tr("Back to keyboard")));
            }
        }
        ui.set_hints(ModelRc::new(VecModel::from(hints)));
    }

    // --- Context menu ---------------------------------------------------------------------

    /// Opens the context menu for the focused entry (in the library also without an entry,
    /// because of the sorting). Nothing happens on the library tile in home.
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
            ContextAction::GameOptions => {
                if let Some(index) = self.entries.iter().position(|e| e.id == id) {
                    self.open_game_options(ui, index);
                }
                return;
            }
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
            eprintln!("Cannot save the collection ({}): {err}", path.display());
        }
        self.refresh(ui);
        // Pinned/unpinned: the focus moves with the entry to its new place.
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

    // --- Nested gamescope options (Story 7.10) ----------------------------------------------

    fn open_game_options(&mut self, ui: &AppWindow, index: usize) {
        let entry = &self.entries[index];
        let options = self.game_options.get(&entry.id);
        self.game_options_panel = Some(GameOptionsPanel::new(&entry.id, &entry.title, &entry.launch, options));
        self.show_game_options(ui);
    }

    fn apply_game_options(&mut self, ui: &AppWindow, action: Action) {
        let Some(panel) = self.game_options_panel.as_mut() else { return };
        match panel.apply(action) {
            PanelOutcome::Stay => {}
            PanelOutcome::Close => {
                self.close_game_options(ui);
                return;
            }
            PanelOutcome::Changed => {
                self.game_options.set(panel.entry_id(), panel.options());
                if let Some(path) = &self.game_options_path
                    && let Err(err) = self.game_options.save(path)
                {
                    eprintln!("Game options could not be saved ({}): {err}", path.display());
                }
            }
        }
        self.show_game_options(ui);
    }

    fn show_game_options(&self, ui: &AppWindow) {
        let Some(panel) = &self.game_options_panel else { return };
        let rows: Vec<SettingRow> = panel
            .rows()
            .into_iter()
            .map(|(label, description, value)| SettingRow {
                label: label.into(),
                description: description.into(),
                value: value.into(),
                opens_page: false,
            })
            .collect();
        ui.set_game_options_title(panel.title().to_uppercase().into());
        ui.set_game_options_rows(ModelRc::new(VecModel::from(rows)));
        ui.set_game_options_focused(panel.focused() as i32);
        let footer = if panel.is_available() { panel.options().preview(self.output_size(ui)) } else { String::new() };
        ui.set_game_options_footer(footer.into());
        ui.set_game_options_open(true);
    }

    fn close_game_options(&mut self, ui: &AppWindow) {
        if self.game_options_panel.take().is_some() {
            ui.set_game_options_open(false);
        }
    }

    /// Size the nested gamescope outputs at: in gamescope the screen (root window), otherwise the
    /// shell's own window (the screen when fullscreen).
    fn output_size(&self, ui: &AppWindow) -> Option<(u32, u32)> {
        if let Some(size) = self.compositor.as_ref().and_then(Compositor::output_size) {
            return Some(size);
        }
        if let Some(size) = self.gamescope.as_ref().and_then(|g| g.control.root_size()) {
            return Some(size);
        }
        let size = ui.window().size();
        (size.width > 0 && size.height > 0).then_some((size.width, size.height))
    }

    // --- Quick menu ------------------------------------------------------------------------

    fn open_quick(&mut self, ui: &AppWindow) {
        // The app in front, else (on home) the app that was started last
        let in_front = self.foreground.is_some_and(|app_id| self.running.iter().any(|r| r.app_id == app_id));
        let running = if in_front {
            self.running.iter().find(|r| Some(r.app_id) == self.foreground)
        } else {
            self.running.last()
        };
        let foreground = running.and_then(|running| {
            let entry = self.entries.iter().find(|e| e.id == running.entry_id)?;
            Some(ForegroundApp {
                app_id: running.app_id,
                entry_id: entry.id.clone(),
                title: entry.title.clone(),
                in_front,
            })
        });
        let profile = self.profiles.get(&self.profile).map(|p| p.name.clone()).unwrap_or_default();
        self.quick = Some(QuickMenu::new(foreground, profile));
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
                    QuickAction::Resume(app_id) => self.bring_to_front(ui, app_id, None),
                    QuickAction::Home => {
                        // Home first, so the app shrinks into its tile there.
                        self.screen = Screen::Home;
                        self.sync(ui);
                        self.show_shell(ui);
                    }
                    QuickAction::SwitchProfile => {
                        // Like "Home": the app goes to the background, then the profile screen opens.
                        self.screen = Screen::Home;
                        self.sync(ui);
                        self.show_shell(ui);
                        self.open_profiles(ui, false);
                    }
                    QuickAction::Quit(entry_id) => self.quit_running(&entry_id),
                    QuickAction::Volume => {}
                    QuickAction::Power(power) => {
                        if let Err(err) = system::power(power) {
                            eprintln!("Power action failed: {err}");
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

    /// Shows the quick menu in the overlay window (gamescope, our compositor) or as a layer in the
    /// main window.
    fn show_quick(&mut self, ui: &AppWindow, open: bool) {
        let Some(quick) = &self.quick else { return };
        let new_items = quick.items();
        if new_items.len() == self.quick_items.row_count() {
            for (row, item) in new_items.into_iter().enumerate() {
                if self.quick_items.row_data(row).as_ref() != Some(&item) {
                    self.quick_items.set_row_data(row, item);
                }
            }
        } else {
            self.quick_items.set_vec(new_items);
        }
        let items = ModelRc::from(self.quick_items.clone());
        let (title, focused) = (quick.title(), quick.focused() as i32);
        if open && !self.quick_shown {
            self.quick_generation.set(self.quick_generation.get() + 1);
        }
        self.quick_shown = open;
        let generation = self.quick_generation.clone();
        // In our compositor the app in front gets no input while the menu is open (Story 7.5).
        if let Some(compositor) = &mut self.compositor {
            compositor.set_overlay(open);
        }
        if let Some(overlay) = self.compositor_ui().map(|c| c.overlay.clone_strong()) {
            // Story 7.6: the compositor stacks this window above the app; the game stays visible.
            overlay.set_items(items);
            overlay.set_title_text(title.into());
            overlay.set_focused(focused);
            overlay.set_controller_style(ui.get_controller_style());
            if open && !overlay.get_open() {
                overlay.window().set_fullscreen(true);
                if let Err(err) = overlay.show() {
                    eprintln!("Cannot show the overlay: {err}");
                }
                // Slint's winit backend created the Wayland toplevel up front (hidden, without a
                // buffer) and does not draw it on `show()` by itself.
                overlay.window().request_redraw();
                open_overlay_later(&overlay, generation);
            } else if !open {
                close_overlay(&overlay, generation);
                // Opened as a layer before the compositor's `Welcome` arrived
                ui.set_quick_open(false);
            }
            // The blur fades out with the closing menu; opening is picked up by the next poll.
            self.sync_blur();
            return;
        }
        let Some(gamescope) = &mut self.gamescope else {
            // Without overlay windows (desktop, older compositor): a layer in the main window; an
            // older compositor brings the whole shell window above the app.
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
            open_overlay_later(&gamescope.overlay, generation);
        } else if !open {
            close_overlay(&gamescope.overlay, generation);
        }
    }

    // --- Launching, tracking, quitting -----------------------------------------------------

    fn launch(&mut self, ui: &AppWindow, index: usize) {
        // The tile the launch screen (and in our compositor the app) grows out of
        let tile_rect = self.tile_rect(ui);
        // If the app is already running, it only comes to the front again.
        if let Some(app_id) = self.running_for(index).map(|r| r.app_id) {
            self.bring_to_front(ui, app_id, tile_rect);
            return;
        }

        ui.set_launch_image(self.launch_image(index));
        let entry = &self.entries[index];
        let tile = &self.tiles[index];
        ui.set_launch_title(tile.title.clone());
        ui.set_launch_accent(tile.accent);

        let now = Instant::now();
        let id = entry.id.clone();
        // With the nested gamescope option, gamescope is started in front of the command (in the
        // same scope, so stopping and window ownership work as before).
        let launch = self.game_options.get(&id).wrap(&entry.launch, self.output_size(ui));
        match launch::spawn(&launch) {
            Ok(handle) => {
                let (app_id, app, stop, owner) = match entry.launch {
                    Launch::Steam(steam_id) => (
                        steam_id,
                        RunningApp::steam(steam_id, now),
                        Stop::Steam(steam_id),
                        Some(Owner::Steam(steam_id)),
                    ),
                    _ => {
                        let app_id = self.next_app_id;
                        self.next_app_id += 1;
                        (
                            app_id,
                            RunningApp::process(handle.clone(), now),
                            Stop::Process(handle.clone()),
                            handle.owner(),
                        )
                    }
                };
                if let Some(compositor) = &mut self.compositor {
                    let steam_app_id = match entry.launch {
                        Launch::Steam(steam_id) => Some(steam_id),
                        _ => None,
                    };
                    compositor.send(&ShellMessage::Launched {
                        app_id,
                        pid: handle.pid(),
                        scope: handle.scope().map(str::to_owned),
                        steam_app_id,
                    });
                }
                self.running.push(Running { entry_id: id.clone(), app_id, app, stop, owner, has_window: false });
                ui.set_launch_status(tr("Starting …").into());
                ui.set_launch_busy(true);
                ui.set_launching(true);
                self.set_running_flag(&id, true);
                self.record_launch(index);
                self.pending_front.push(id.clone());
                if self.settings.animations && (self.gamescope.is_some() || self.compositor.is_some()) {
                    // The app comes to the front once the launch screen is fully open; our
                    // compositor then zooms it in from (nearly) fullscreen.
                    self.foreground = Some(app_id);
                    self.front_due = Some(now + Duration::from_millis(DURATIONS[3] as u64));
                    self.launch_shown = false;
                } else {
                    self.set_foreground(Some(app_id), self.transition(tile_rect));
                }
            }
            Err(err) => {
                eprintln!("Cannot start {}: {err}", entry.title);
                ui.set_launch_status(tr("Couldn't start").into());
                ui.set_launch_busy(false);
                ui.set_launching(true);
                self.failed_launch = Some(now);
                self.show_alert(ui, &trf("Couldn't start {}", &[&entry.title]), &err.to_string());
            }
        }
    }

    /// Brings a running app to the front (out of `tile`, if given, in our compositor). Without
    /// gamescope this is not possible; then the launch screen shows that it is running.
    fn bring_to_front(&mut self, ui: &AppWindow, app_id: u32, tile: Option<Rect>) {
        if self.gamescope.is_some() || self.compositor.is_some() {
            self.set_foreground(Some(app_id), self.transition(tile));
            return;
        }
        let Some(index) = self
            .running
            .iter()
            .find(|r| r.app_id == app_id)
            .and_then(|r| self.entries.iter().position(|e| e.id == r.entry_id))
        else {
            return;
        };
        ui.set_launch_title(self.tiles[index].title.clone());
        ui.set_launch_image(self.launch_image(index));
        ui.set_launch_accent(self.tiles[index].accent);
        ui.set_launch_status(tr("Running").into());
        ui.set_launch_busy(false);
        ui.set_launching(true);
    }

    /// Brings the shell to the front; running apps keep running in the background.
    fn show_shell(&mut self, ui: &AppWindow) {
        ui.set_launching(false);
        self.close_launch_at = None;
        let transition = self.transition(self.tile_rect(ui));
        self.set_foreground(None, transition);
    }

    /// The focused tile in output pixels (the main window is fullscreen in our compositor).
    fn tile_rect(&self, ui: &AppWindow) -> Option<Rect> {
        self.compositor.as_ref()?;
        let size = ui.get_tile_size().round() as i32;
        Some(Rect { x: ui.get_tile_x().round() as i32, y: ui.get_tile_y().round() as i32, width: size, height: size })
    }

    /// How our compositor animates the next front change (Story 7.7); `None` with animations off.
    fn transition(&self, tile: Option<Rect>) -> Option<Transition> {
        self.settings.animations.then_some(Transition { tile, card_ms: 0 })
    }

    fn set_foreground(&mut self, app_id: Option<u32>, transition: Option<Transition>) {
        self.foreground = app_id;
        self.front_due = None;
        if let Some(compositor) = &mut self.compositor {
            compositor.focus(app_id, transition);
        }
        if let Some(gamescope) = &self.gamescope {
            match app_id {
                Some(app_id) => gamescope.control.set_baselayer(&[app_id, SHELL_APP_ID]),
                None => gamescope.control.set_baselayer(&[SHELL_APP_ID]),
            }
        }
    }

    /// Sets the focus in home, library and search to the entry (after re-sorting).
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

    /// Runs periodically: in gamescope, assigns new windows to their apps and checks which apps
    /// have exited. If the app in front is done, the shell comes back with an animation.
    pub fn watch_running(&mut self, ui: &AppWindow) {
        self.apply_pending_front(ui);
        if let Some(failed) = self.failed_launch
            && failed.elapsed() >= MIN_HOLD
        {
            self.failed_launch = None;
            ui.set_launching(false);
        }
        self.tag_windows(ui);
        self.restore_keyboard_focus(ui);
        // Also after the boot screen, an app exit or the launch screen closing
        self.update_ambience(ui);

        let now = Instant::now();
        let steam_games =
            if self.running.iter().any(|r| r.app.is_steam()) { steam::running_games() } else { Default::default() };
        let mut finished = Vec::new();
        for running in &mut self.running {
            match running.app.phase(now, &steam_games) {
                Phase::Starting => {}
                Phase::Running => {
                    if ui.get_launching() && self.foreground == Some(running.app_id) {
                        ui.set_launch_status(tr("Running").into());
                        ui.set_launch_busy(false);
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
            if let Some(compositor) = &mut self.compositor {
                compositor.send(&ShellMessage::Exited { app_id: *app_id });
            }
            self.set_running_flag(entry_id, false);
            if self
                .dialog
                .as_ref()
                .is_some_and(|d| d.buttons().iter().any(|b| b.tag == DialogTag::QuitApp(entry_id.clone())))
            {
                self.close_dialog(ui);
            }
            if self.foreground == Some(*app_id) {
                self.show_shell(ui);
            }
        }
        self.refresh(ui);
    }

    /// Messages from our compositor: which launched apps show a window (the launch screen closes
    /// as soon as the app in front has one).
    /// After each rendered frame of the main window: notes when the launch screen is fully open.
    pub fn frame_rendered(&mut self) {
        if self.front_due.is_some_and(|due| Instant::now() >= due) {
            self.launch_shown = true;
        }
    }

    pub fn poll_compositor(&mut self, ui: &AppWindow) {
        // The next poll after the open launch screen was drawn (it has reached the screen by
        // then); without frame notifications a little later.
        if self.front_due.is_some_and(|due| self.launch_shown || Instant::now() >= due + FRONT_FALLBACK) {
            let app_id = self.foreground;
            self.set_foreground(app_id, self.transition(None));
            // In gamescope the app is visible at once if it already has a window.
            if self.gamescope.is_some() && self.running.iter().any(|r| Some(r.app_id) == app_id && r.has_window) {
                ui.set_launching(false);
            }
        }
        if self.close_launch_at.is_some_and(|at| Instant::now() >= at) {
            self.close_launch_at = None;
            ui.set_launching(false);
        }
        let Some(compositor) = &mut self.compositor else { return };
        let mut guide = false;
        let mut keymap_changed = false;
        let mut screenshot = None;
        for message in compositor.poll() {
            match message {
                CompositorMessage::WindowMapped { app_id, .. } => {
                    if let Some(running) = self.running.iter_mut().find(|r| r.app_id == app_id) {
                        running.has_window = true;
                        if self.foreground == Some(app_id) && ui.get_launching() {
                            if compositor.transitions() && self.settings.animations {
                                // The app zooms in above the launch screen first (Story 7.7), once
                                // that is fully open.
                                let due = self.front_due.map(|due| due + FRONT_FALLBACK);
                                let front = due.unwrap_or_else(Instant::now).max(Instant::now());
                                self.close_launch_at = Some(front + LAUNCH_HANDOVER);
                            } else {
                                ui.set_launching(false);
                            }
                        }
                    }
                }
                CompositorMessage::AppGone { app_id } => {
                    if let Some(running) = self.running.iter_mut().find(|r| r.app_id == app_id) {
                        running.has_window = false;
                    }
                }
                // The keyboard's Guide key, reserved by the compositor (Story 7.5)
                CompositorMessage::Guide => guide = true,
                CompositorMessage::Keymap { .. } => keymap_changed = true,
                // The Print key (Story 7.12)
                CompositorMessage::ScreenshotDone { path, error } => screenshot = Some((path, error)),
                _ => {}
            }
        }
        if keymap_changed {
            self.update_settings_rows(ui);
        }
        if let Some((path, error)) = screenshot {
            match error {
                None => {
                    let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned());
                    self.toast(ui, "▣", tr("Screenshot saved"), &name.unwrap_or(path));
                }
                Some(error) => self.toast(ui, "⚠", tr("Screenshot failed"), &error),
            }
        }
        self.sync_blur();
        if guide {
            self.apply(ui, Action::Guide);
        }
    }

    /// If the shell is in front but has no keyboard focus (happens after the overlay), it takes it.
    /// Likewise for the app in front, whose window otherwise does not get the focus back after the overlay.
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

    /// In Steam mode gamescope only shows windows with an app ID; the shell assigns them here.
    fn tag_windows(&mut self, ui: &AppWindow) {
        let Some(gamescope) = &mut self.gamescope else { return };
        let own_pid = std::process::id();
        let apps: Vec<(u32, Owner)> =
            self.running.iter().filter_map(|r| r.owner.clone().map(|owner| (r.app_id, owner))).collect();
        // Windows without a recognizable owner (e.g. Steam itself on first start) belong to the
        // most recently launched app that has no window yet.
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
                // The game is visible now; the launch screen below it is no longer needed.
                if self.foreground == Some(app_id) && self.front_due.is_none() {
                    ui.set_launching(false);
                }
            }
        }
    }

    /// Sets the "running" badge and the subtitle; returns whether the entry exists.
    fn set_running_flag(&mut self, id: &str, running: bool) -> bool {
        let Some(index) = self.entries.iter().position(|e| e.id == id) else { return false };
        self.tiles[index].running = running;
        self.relabel_tile(index);
        true
    }

    /// Sorts `entries` (and `tiles` with them) by last use and title again, e.g. after the titles
    /// changed with the language. Stable, so equal entries keep their order.
    fn resort(&mut self) {
        let mut pairs: Vec<(AppEntry, AppTile)> =
            std::mem::take(&mut self.entries).into_iter().zip(std::mem::take(&mut self.tiles)).collect();
        pairs.sort_by(|a, b| compare_recent(&a.0, &b.0));
        (self.entries, self.tiles) = pairs.into_iter().unzip();
    }

    /// Texts of a tile that depend on its state and the language
    fn relabel_tile(&mut self, index: usize) {
        let entry = &self.entries[index];
        let tile = &mut self.tiles[index];
        tile.title = entry.title.as_str().into();
        tile.monogram = entry.monogram().into();
        tile.kind = tiles::kind_label(entry.kind).into();
        tile.action_label = tiles::action_label(entry.kind, tile.running).into();
        tile.subtitle =
            if tile.running { tr("Running now").into() } else { tiles::subtitle(entry, tiles::today()).into() };
    }

    fn ask_quit(&mut self, ui: &AppWindow, index: usize) {
        if self.running_for(index).is_none() {
            return;
        }
        let buttons = vec![
            DialogButton { label: tr("Cancel").into(), tag: DialogTag::Close },
            DialogButton { label: tr("Quit").into(), tag: DialogTag::QuitApp(self.entries[index].id.clone()) },
        ];
        let title = trf("Quit {}?", &[&self.entries[index].title]);
        self.show_dialog(ui, Dialog::new(title, tr("Unsaved progress may be lost."), buttons, 1));
    }

    /// Opens a modal dialog; it takes all input until a button is picked or Back dismisses it.
    fn show_dialog(&mut self, ui: &AppWindow, dialog: Dialog<DialogTag>) {
        self.dialog = Some(dialog);
        self.sync_dialog(ui);
        ui.set_dialog_open(true);
        self.update_hints(ui);
    }

    /// A dialog with one button, for errors the player has to notice.
    fn show_alert(&mut self, ui: &AppWindow, title: &str, text: &str) {
        self.show_dialog(ui, Dialog::alert(title, text, tr("OK"), DialogTag::Close));
    }

    fn sync_dialog(&self, ui: &AppWindow) {
        let Some(dialog) = &self.dialog else { return };
        ui.set_dialog_title(dialog.title.as_str().into());
        ui.set_dialog_text(dialog.text.as_str().into());
        let labels: Vec<slint::SharedString> = dialog.buttons().iter().map(|b| b.label.as_str().into()).collect();
        ui.set_dialog_buttons(ModelRc::new(VecModel::from(labels)));
        ui.set_dialog_focused(dialog.focused() as i32);
    }

    fn apply_dialog(&mut self, ui: &AppWindow, action: Action) {
        let Some(dialog) = &mut self.dialog else { return };
        match dialog.apply(action) {
            DialogOutcome::None => self.sync_dialog(ui),
            DialogOutcome::Dismissed => self.close_dialog(ui),
            DialogOutcome::Chosen(tag) => {
                self.close_dialog(ui);
                match tag {
                    DialogTag::QuitApp(id) => self.quit_running(&id),
                    DialogTag::Close => {}
                }
            }
        }
    }

    /// Closes the dialog; its texts stay in the properties while it fades out.
    fn close_dialog(&mut self, ui: &AppWindow) {
        self.dialog = None;
        ui.set_dialog_open(false);
        self.update_hints(ui);
    }

    /// Asks the app to quit. The badge disappears as soon as `watch_running` sees it exit.
    fn quit_running(&mut self, entry_id: &str) {
        let Some(running) = self.running.iter().find(|r| r.entry_id == entry_id) else { return };
        let result = match &running.stop {
            Stop::Process(handle) => handle.stop(),
            Stop::Steam(app_id) => steam::stop_game(*app_id).map(|_| ()),
        };
        if let Err(err) = result {
            eprintln!("Quitting failed: {err}");
        }
    }

    /// Records the launch in the history and moves the entry to the front. This happens under
    /// the launch screen, so the re-sort is not visible.
    fn record_launch(&mut self, index: usize) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        self.entries[index].last_used = Some(now);
        self.history.record(&self.entries[index].id, now);
        if let Some(path) = &self.history_path
            && let Err(err) = self.history.save(path)
        {
            eprintln!("Cannot save the history ({}): {err}", path.display());
        }
    }

    /// Moves the launched entries to the front of the lists once the shell is in front again.
    fn apply_pending_front(&mut self, ui: &AppWindow) {
        if self.pending_front.is_empty() || self.foreground.is_some() || ui.get_launching() || ui.get_booting() {
            return;
        }
        let ids = std::mem::take(&mut self.pending_front);
        for id in &ids {
            if let Some(index) = self.entries.iter().position(|e| &e.id == id) {
                let entry = self.entries.remove(index);
                let tile = self.tiles.remove(index);
                self.entries.insert(0, entry);
                self.tiles.insert(0, tile);
            }
        }
        self.home.focus(0);
        self.refresh(ui);
        if let Some(id) = ids.last() {
            self.focus_entry(id);
        }
    }
}

/// Selects the language for strings built in Rust and for Slint's `@tr()` (Story 6.6).
pub fn select_language(language: Language) {
    i18n::set_language(language);
    if let Err(err) = slint::select_bundled_translation(language.code()) {
        eprintln!("Cannot select the translation {}: {err}", language.code());
    }
}

/// Keys of the on-screen keyboard in the search
fn show_keys(ui: &AppWindow, keyboard: &OnScreenKeyboard) {
    let keys: Vec<KeyCap> = keyboard
        .keys()
        .iter()
        .map(|k| KeyCap { label: k.key.label().into(), row: k.row as i32, x: k.x, width: k.width })
        .collect();
    ui.set_keys(ModelRc::new(VecModel::from(keys)));
}

/// Shows the overlay window. The first time (or when Slint has recreated the window) it first
/// gets the overlay atoms and is then remapped, because gamescope reads them only when
/// mapping.
fn show_overlay_window(gamescope: &mut GamescopeUi) {
    let overlay = &gamescope.overlay;
    overlay.window().set_fullscreen(true);
    if let Err(err) = overlay.show() {
        eprintln!("Cannot show the overlay: {err}");
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

/// Opens the quick menu only after the window is shown, so the bar slides in (unless it was
/// closed again within that time).
fn open_overlay_later(overlay: &OverlayWindow, generation: Rc<Cell<u64>>) {
    let weak = overlay.as_weak();
    let opened = generation.get();
    slint::Timer::single_shot(Duration::from_millis(30), move || {
        if let Some(overlay) = weak.upgrade()
            && generation.get() == opened
        {
            overlay.set_open(true);
        }
    });
}

/// Slides the quick menu out and then hides its window, unless it was opened again meanwhile.
/// `open` alone cannot tell: a menu opened again is shown at once but sets `open` only 30 ms
/// later, and the window must not be hidden in between.
fn close_overlay(overlay: &OverlayWindow, generation: Rc<Cell<u64>>) {
    overlay.set_open(false);
    let weak = overlay.as_weak();
    let closed = generation.get();
    slint::Timer::single_shot(Duration::from_millis(450), move || {
        if let Some(overlay) = weak.upgrade().filter(|o| !o.get_open())
            && generation.get() == closed
        {
            let _ = overlay.hide();
        }
    });
}

/// Shows the notification window in our compositor (Story 7.6). The compositor makes every shell
/// toplevel fullscreen and lets pointer input pass through it, so it just has the output size.
fn show_compositor_notification(notification: &NotificationWindow, output: Option<(u32, u32)>) {
    if let Some((width, height)) = output {
        let scale = notification.window().scale_factor();
        notification.set_window_width(width as f32 / scale);
        notification.set_window_height(height as f32 / scale);
    }
    notification.window().set_fullscreen(true);
    if let Err(err) = notification.show() {
        eprintln!("Cannot show the notification window: {err}");
    }
    notification.window().request_redraw();
}

/// Shows the notification window. gamescope picks it as a notification (instead of an
/// interactive overlay) only if it is narrower than the root window. It is centered and scaled
/// to the output, its own position does not matter; hence it is exactly one pixel narrower than
/// the screen and full height (scale 1), with the toast at its top right. The size is fixed
/// (min = max in the size hints), otherwise gamescope stretches it to the screen size.
fn show_notification_window(gamescope: &mut GamescopeUi) {
    let notification = &gamescope.notification;
    if let Some((width, height)) = gamescope.control.root_size() {
        let scale = notification.window().scale_factor();
        notification.set_window_width(width.saturating_sub(1).max(1) as f32 / scale);
        notification.set_window_height(height as f32 / scale);
    }
    if let Err(err) = notification.show() {
        eprintln!("Cannot show the notification window: {err}");
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

/// Hides the toast and then unmaps the window, unless a new one arrived in the meantime.
fn hide_notification_window(notification: &NotificationWindow) {
    notification.set_shown(false);
    let weak = notification.as_weak();
    slint::Timer::single_shot(Duration::from_millis(450), move || {
        if let Some(notification) = weak.upgrade().filter(|n| !n.get_shown()) {
            let _ = notification.hide();
        }
    });
}
