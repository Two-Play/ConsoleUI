//! Shell settings, stored in `$XDG_CONFIG_HOME/consoleui/settings.json`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::backdrop::{Palette, Style};
use crate::i18n::{LanguageChoice, tr};
use crate::keymap::KeyboardLayout;
use crate::xdg;

/// Color of focus frames, buttons and selected cards
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Accent {
    #[default]
    White,
    Blue,
    Violet,
    Green,
    Orange,
    Red,
}

impl Accent {
    pub const ALL: [Accent; 6] = [Self::White, Self::Blue, Self::Violet, Self::Green, Self::Orange, Self::Red];

    pub fn label(self) -> &'static str {
        match self {
            Self::White => tr("White"),
            Self::Blue => tr("Blue"),
            Self::Violet => tr("Violet"),
            Self::Green => tr("Green"),
            Self::Orange => tr("Orange"),
            Self::Red => tr("Red"),
        }
    }

    pub fn rgb(self) -> [u8; 3] {
        match self {
            Self::White => [0xf4, 0xf6, 0xfb],
            Self::Blue => [0x5a, 0xa9, 0xff],
            Self::Violet => [0xa7, 0x8b, 0xff],
            Self::Green => [0x5f, 0xe0, 0xa5],
            Self::Orange => [0xff, 0xb0, 0x4a],
            Self::Red => [0xff, 0x6b, 0x6b],
        }
    }
}

/// Which button symbols are shown
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GlyphChoice {
    /// According to the connected controller
    #[default]
    Auto,
    PlayStation,
    Xbox,
}

impl GlyphChoice {
    pub const ALL: [GlyphChoice; 3] = [Self::Auto, Self::PlayStation, Self::Xbox];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => tr("Automatic"),
            Self::PlayStation => "PlayStation",
            Self::Xbox => "Xbox",
        }
    }
}

/// What fills the background behind the pages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundChoice {
    /// The hero artwork of the focused entry; the animated background where there is none
    #[default]
    Artwork,
    /// Always the animated background (see [`crate::backdrop`])
    Animated,
    /// The artwork, and the plain accent gradient where there is none
    Classic,
}

impl BackgroundChoice {
    pub const ALL: [BackgroundChoice; 3] = [Self::Artwork, Self::Animated, Self::Classic];

    pub fn label(self) -> &'static str {
        match self {
            Self::Artwork => tr("Artwork"),
            Self::Animated => tr("Animated"),
            Self::Classic => tr("Classic"),
        }
    }

    /// Whether the animated background shows, given whether the focused entry has artwork.
    pub fn animated(self, has_artwork: bool) -> bool {
        match self {
            Self::Artwork => !has_artwork,
            Self::Animated => true,
            Self::Classic => false,
        }
    }
}

/// Where the theme music of the focused game plays (Story 6.12, see [`crate::themes`])
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMusic {
    #[default]
    Off,
    Home,
    HomeAndLibrary,
}

impl ThemeMusic {
    pub const ALL: [ThemeMusic; 3] = [Self::Off, Self::Home, Self::HomeAndLibrary];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => tr("Off"),
            Self::Home => tr("Home screen"),
            Self::HomeAndLibrary => tr("Home screen and library"),
        }
    }
}

/// Background music on the home screen (see [`crate::music`])
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicChoice {
    #[default]
    Off,
    /// The generated ambient loop
    Ambient,
    /// Files from the music folder (`$XDG_DATA_HOME/consoleui/music`)
    Folder,
}

impl MusicChoice {
    pub const ALL: [MusicChoice; 3] = [Self::Off, Self::Ambient, Self::Folder];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => tr("Off"),
            Self::Ambient => tr("Ambient"),
            Self::Folder => tr("My music"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub accent: Accent,
    pub animations: bool,
    pub background: BackgroundChoice,
    /// Look of the animated background
    pub shader: Style,
    /// Colors of the animated background
    pub palette: Palette,
    /// Load covers and logos (off: generated placeholders, nothing decoded)
    pub artwork: bool,
    /// Show messages of other programs and download news as toasts
    pub notifications: bool,
    pub sounds: bool,
    pub music: MusicChoice,
    /// File name of the track to play on repeat from the music folder (`None`: all, shuffled)
    pub music_track: Option<String>,
    /// On the home screen the theme music of the focused game plays over the background music
    /// (searched on YouTube, `consoleui_core::themes`, Story 6.12)
    pub theme_music: ThemeMusic,
    pub glyphs: GlyphChoice,
    pub language: LanguageChoice,
    /// Layout of the physical keyboard in our compositor (the on-screen keyboard follows the
    /// language instead)
    pub keyboard_layout: KeyboardLayout,
    /// The monitor in our compositor (a connector name such as `HDMI-A-1`); `None` is the
    /// automatic choice
    pub display: Option<String>,
    /// Output size in our compositor; `None` is the monitor's preferred mode
    pub resolution: Option<(u32, u32)>,
    /// Variable refresh rate while a game is in front, in our compositor
    pub vrr: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            accent: Accent::default(),
            animations: true,
            background: BackgroundChoice::default(),
            shader: Style::default(),
            palette: Palette::default(),
            artwork: true,
            notifications: true,
            sounds: true,
            music: MusicChoice::default(),
            music_track: None,
            theme_music: ThemeMusic::Off,
            glyphs: GlyphChoice::default(),
            language: LanguageChoice::default(),
            keyboard_layout: KeyboardLayout::default(),
            display: None,
            resolution: None,
            vrr: false,
        }
    }
}

/// A row of the settings page
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Accent,
    Animations,
    /// Artwork or the animated background
    Background,
    /// Look of the animated background
    AnimationStyle,
    /// Colors of the animated background
    AnimationColors,
    /// Load covers and logos
    Artwork,
    /// Toasts for messages of other programs and downloads
    Notifications,
    Sounds,
    /// Background music on the home screen
    Music,
    /// Which file of the music folder plays (only with "My music")
    MusicTrack,
    /// Theme music of the focused game on the home screen (Story 6.12)
    GameThemes,
    Glyphs,
    /// Story 6.6
    Language,
    /// Only under our compositor, which applies it to the seat keyboard
    KeyboardLayout,
    /// Only with several monitors under our compositor, which switches to it (Story 7.11)
    Display,
    /// Only on a real display under our compositor, which switches the mode (Story 5.5)
    Resolution,
    /// Only when the monitor can do it under our compositor (Story 7.9)
    Vrr,
    /// Opens the update page (Story 5.8)
    Updates,
    /// Opens the Wi-Fi page (Story 5.3)
    Wifi,
    /// Opens the Bluetooth page (Story 5.4)
    Bluetooth,
    /// Opens the plugin list (Epic 9)
    Plugins,
}

impl Setting {
    pub const ALL: [Setting; 21] = [
        Self::Accent,
        Self::Animations,
        Self::Background,
        Self::AnimationStyle,
        Self::AnimationColors,
        Self::Artwork,
        Self::Notifications,
        Self::Sounds,
        Self::Music,
        Self::MusicTrack,
        Self::GameThemes,
        Self::Glyphs,
        Self::Language,
        Self::KeyboardLayout,
        Self::Display,
        Self::Resolution,
        Self::Vrr,
        Self::Wifi,
        Self::Bluetooth,
        Self::Updates,
        Self::Plugins,
    ];

    /// Rows that open a sub-page instead of changing a value
    pub fn opens_page(self) -> bool {
        matches!(self, Self::Wifi | Self::Bluetooth | Self::Updates | Self::Plugins)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Accent => tr("Accent color"),
            Self::Animations => tr("Animations"),
            Self::Background => tr("Background"),
            Self::AnimationStyle => tr("Animation style"),
            Self::AnimationColors => tr("Animation colors"),
            Self::Artwork => tr("Game artwork"),
            Self::Notifications => tr("Notifications"),
            Self::Sounds => tr("Sounds"),
            Self::Music => tr("Background music"),
            Self::MusicTrack => tr("Music track"),
            Self::GameThemes => tr("Game themes"),
            Self::Glyphs => tr("Button symbols"),
            Self::Language => tr("Language"),
            Self::KeyboardLayout => tr("Keyboard layout"),
            Self::Display => tr("Screen"),
            Self::Resolution => tr("Resolution"),
            Self::Vrr => tr("Variable refresh rate"),
            Self::Wifi => tr("Wi-Fi"),
            Self::Bluetooth => tr("Bluetooth"),
            Self::Updates => tr("Updates"),
            Self::Plugins => tr("Plugins"),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Accent => tr("Color of selection and buttons"),
            Self::Animations => tr("Transitions and motion"),
            Self::Background => tr("Game artwork or an animated background"),
            Self::AnimationStyle => tr("Look of the animated background"),
            Self::AnimationColors => tr("Colors of the animated background"),
            Self::Artwork => tr("Load covers and logos; off saves memory"),
            Self::Notifications => tr("Toasts for messages from other programs and downloads"),
            Self::Sounds => tr("Sounds while navigating"),
            Self::Music => tr("Plays while the home screen is in front"),
            Self::MusicTrack => tr("One file on repeat, or all shuffled"),
            Self::GameThemes => tr("The music of the game in focus, found on YouTube"),
            Self::Glyphs => tr("Symbols in the button hints"),
            Self::Language => tr("Language of the interface"),
            Self::KeyboardLayout => tr("Layout of the connected keyboard"),
            Self::Display => tr("Which connected screen shows the console"),
            Self::Resolution => tr("Output resolution of the screen"),
            Self::Vrr => tr("The screen follows the frame rate of the game (VRR)"),
            Self::Wifi => tr("Show and join networks"),
            Self::Bluetooth => tr("Pair controllers and other devices"),
            Self::Updates => tr("Search for and install system updates"),
            Self::Plugins => tr("Games and controls from other programs"),
        }
    }
}

fn on_off(value: bool) -> &'static str {
    if value { tr("On") } else { tr("Off") }
}

/// Next (`forward`) or previous value in a list, wrapping around.
fn cycle<T: Copy + PartialEq>(all: &[T], current: T, forward: bool) -> T {
    let index = all.iter().position(|&v| v == current).unwrap_or(0);
    let len = all.len();
    all[if forward { (index + 1) % len } else { (index + len - 1) % len }]
}

/// Next or previous choice of the "Resolution" row: automatic (`None`), then the output's sizes.
pub fn cycle_resolution(sizes: &[(u32, u32)], current: Option<(u32, u32)>, forward: bool) -> Option<(u32, u32)> {
    let all: Vec<_> = std::iter::once(None).chain(sizes.iter().copied().map(Some)).collect();
    cycle(&all, current, forward)
}

/// Next or previous choice of the "Screen" row: automatic (`None`), then the connected monitors.
/// A saved monitor that is not connected right now counts as automatic.
pub fn cycle_display(names: &[String], current: Option<&str>, forward: bool) -> Option<String> {
    let all: Vec<Option<&str>> = std::iter::once(None).chain(names.iter().map(|name| Some(name.as_str()))).collect();
    let current = current.filter(|current| names.iter().any(|name| name == current));
    cycle(&all, current, forward).map(str::to_string)
}

impl Settings {
    /// The rows of the settings page: the keyboard layout only where it has an effect (in our
    /// compositor; gamescope and the desktop use the layout of their own session), the resolution
    /// only when the compositor reports modes, variable refresh rate only when the monitor can do it,
    /// the animation style unless the background is classic, the track only for "My music", the
    /// screen only when there is more than one.
    pub fn rows(&self, keyboard_layout: bool, resolution: bool, vrr: bool, display: bool) -> Vec<Setting> {
        Setting::ALL
            .into_iter()
            .filter(|&row| match row {
                Setting::KeyboardLayout => keyboard_layout,
                Setting::Resolution => resolution,
                Setting::Display => display,
                Setting::Vrr => vrr,
                Setting::AnimationStyle | Setting::AnimationColors => self.background != BackgroundChoice::Classic,
                Setting::MusicTrack => self.music == MusicChoice::Folder,
                _ => true,
            })
            .collect()
    }

    /// `$XDG_CONFIG_HOME/consoleui/settings.json`
    pub fn default_path() -> Option<PathBuf> {
        xdg::config_home().map(|dir| dir.join("consoleui/settings.json"))
    }

    /// A missing or broken file gives the defaults.
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => Self::parse(&content).unwrap_or_else(|err| {
                eprintln!("Settings {} are invalid and ignored: {err}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Also reads the files of a short-lived version that had "Game themes" as a choice of the
    /// background music: that is the ambient loop plus the own row now.
    fn parse(content: &str) -> serde_json::Result<Self> {
        let mut value: serde_json::Value = serde_json::from_str(content)?;
        if let Some(object) = value.as_object_mut()
            && object.get("music").and_then(serde_json::Value::as_str) == Some("game_themes")
        {
            object.insert("music".into(), "ambient".into());
            object.insert("theme_music".into(), "home".into());
        }
        serde_json::from_value(value)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        fs::rename(tmp, path)
    }

    /// Value of the row; for sub-pages (Wi-Fi, Bluetooth, updates) the shell shows their own state.
    pub fn value_label(&self, setting: Setting) -> &'static str {
        match setting {
            Setting::Accent => self.accent.label(),
            Setting::Animations => on_off(self.animations),
            Setting::Background => self.background.label(),
            Setting::AnimationStyle => self.shader.label(),
            Setting::AnimationColors => self.palette.label(),
            Setting::Artwork => on_off(self.artwork),
            Setting::Notifications => on_off(self.notifications),
            Setting::Sounds => on_off(self.sounds),
            Setting::Music => self.music.label(),
            // Names a file or a mode: the shell has the list
            Setting::MusicTrack | Setting::Resolution | Setting::Display => "",
            Setting::Vrr => on_off(self.vrr),
            Setting::GameThemes => self.theme_music.label(),
            Setting::Glyphs => self.glyphs.label(),
            Setting::Language => self.language.label(),
            Setting::KeyboardLayout => self.keyboard_layout.label(),
            Setting::Wifi | Setting::Bluetooth | Setting::Updates | Setting::Plugins => "",
        }
    }

    /// Changes the value (left/right or confirm).
    pub fn change(&mut self, setting: Setting, forward: bool) {
        match setting {
            Setting::Accent => self.accent = cycle(&Accent::ALL, self.accent, forward),
            Setting::Animations => self.animations = !self.animations,
            Setting::Background => self.background = cycle(&BackgroundChoice::ALL, self.background, forward),
            Setting::AnimationStyle => self.shader = cycle(&Style::ALL, self.shader, forward),
            Setting::AnimationColors => self.palette = cycle(&Palette::ALL, self.palette, forward),
            Setting::Artwork => self.artwork = !self.artwork,
            Setting::Notifications => self.notifications = !self.notifications,
            Setting::Sounds => self.sounds = !self.sounds,
            Setting::Vrr => self.vrr = !self.vrr,
            Setting::GameThemes => self.theme_music = cycle(&ThemeMusic::ALL, self.theme_music, forward),
            Setting::Music => self.music = cycle(&MusicChoice::ALL, self.music, forward),
            Setting::Glyphs => self.glyphs = cycle(&GlyphChoice::ALL, self.glyphs, forward),
            Setting::Language => self.language = cycle(&LanguageChoice::ALL, self.language, forward),
            Setting::KeyboardLayout => {
                self.keyboard_layout = cycle(&KeyboardLayout::ALL, self.keyboard_layout, forward);
            }
            // The shell steps through the music folder (`music::cycle_track`) and the output's
            // modes (`cycle_resolution`) and monitors (`cycle_display`)
            Setting::MusicTrack
            | Setting::Resolution
            | Setting::Display
            | Setting::Wifi
            | Setting::Bluetooth
            | Setting::Updates
            | Setting::Plugins => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_cycles_through_values() {
        let mut settings = Settings::default();
        settings.change(Setting::Accent, false);
        assert_eq!(settings.accent, Accent::Red);
        settings.change(Setting::Accent, true);
        assert_eq!(settings.accent, Accent::White);
        settings.change(Setting::Sounds, true);
        assert!(!settings.sounds);
        assert_eq!(settings.value_label(Setting::Sounds), "Off");
        settings.change(Setting::Glyphs, true);
        assert_eq!(settings.glyphs, GlyphChoice::PlayStation);
        settings.change(Setting::Background, true);
        assert_eq!(settings.background, BackgroundChoice::Animated);
        assert!(BackgroundChoice::Artwork.animated(false) && !BackgroundChoice::Artwork.animated(true));
        assert!(BackgroundChoice::Animated.animated(true) && !BackgroundChoice::Classic.animated(false));
        settings.change(Setting::Music, true);
        assert_eq!(settings.music, MusicChoice::Ambient);
        settings.change(Setting::Music, false);
        settings.change(Setting::Music, false);
        assert_eq!(settings.music, MusicChoice::Folder);
        // Sub-pages change no value
        settings.change(Setting::Wifi, true);
        assert_eq!(settings.glyphs, GlyphChoice::PlayStation);
        assert!(Setting::Bluetooth.opens_page() && !Setting::Sounds.opens_page());
        settings.change(Setting::Language, true);
        assert_eq!(settings.language, LanguageChoice::English);
        settings.change(Setting::Language, false);
        settings.change(Setting::Language, false);
        assert_eq!(settings.language, LanguageChoice::German);
        settings.change(Setting::KeyboardLayout, false);
        assert_eq!(settings.keyboard_layout, KeyboardLayout::Pl);
        settings.change(Setting::KeyboardLayout, true);
        settings.change(Setting::KeyboardLayout, true);
        assert_eq!(settings.keyboard_layout, KeyboardLayout::De);
    }

    #[test]
    fn game_themes_from_the_music_choice_become_their_own_row() {
        let settings = Settings::parse(r#"{"music": "game_themes", "sounds": false}"#).unwrap();
        assert_eq!(settings.music, MusicChoice::Ambient);
        assert!(settings.theme_music == ThemeMusic::Home && !settings.sounds);
        let plain = Settings::parse(r#"{"music": "folder"}"#).unwrap();
        assert_eq!(plain.theme_music, ThemeMusic::Off);
    }

    #[test]
    fn display_row_and_cycle() {
        let settings = Settings::default();
        assert!(!settings.rows(true, true, true, false).contains(&Setting::Display));
        assert!(settings.rows(true, true, true, true).contains(&Setting::Display));
        let names = ["eDP-1".to_string(), "HDMI-A-1".to_string()];
        assert_eq!(cycle_display(&names, None, true).as_deref(), Some("eDP-1"));
        assert_eq!(cycle_display(&names, Some("HDMI-A-1"), true), None);
        assert_eq!(cycle_display(&names, None, false).as_deref(), Some("HDMI-A-1"));
        // Unplugged: the next step starts from automatic
        assert_eq!(cycle_display(&names, Some("DP-2"), true).as_deref(), Some("eDP-1"));
    }

    #[test]
    fn resolution_row_and_cycle() {
        let settings = Settings::default();
        assert!(!settings.rows(true, false, false, false).contains(&Setting::Resolution));
        assert!(!settings.rows(true, true, false, false).contains(&Setting::Vrr));
        assert!(settings.rows(true, true, true, false).contains(&Setting::Vrr));
        assert!(settings.rows(false, true, false, false).contains(&Setting::Resolution));
        let sizes = [(1920, 1080), (1280, 720)];
        assert_eq!(cycle_resolution(&sizes, None, true), Some((1920, 1080)));
        assert_eq!(cycle_resolution(&sizes, Some((1280, 720)), true), None);
        assert_eq!(cycle_resolution(&sizes, None, false), Some((1280, 720)));
        // A size of another monitor counts as automatic
        assert_eq!(cycle_resolution(&sizes, Some((3840, 2160)), true), Some((1920, 1080)));
    }

    #[test]
    fn keyboard_layout_row_only_in_our_compositor() {
        let settings = Settings::default();
        assert!(!settings.rows(false, false, false, false).contains(&Setting::KeyboardLayout));
        assert_eq!(settings.rows(false, false, false, false).len(), Setting::ALL.len() - 5);
        assert!(settings.rows(true, false, false, false).contains(&Setting::KeyboardLayout));
    }

    #[test]
    fn track_and_style_rows_depend_on_their_parent() {
        let mut settings = Settings::default();
        assert!(!settings.rows(false, false, false, false).contains(&Setting::MusicTrack));
        settings.music = MusicChoice::Folder;
        assert!(settings.rows(false, false, false, false).contains(&Setting::MusicTrack));
        assert!(settings.rows(false, false, false, false).contains(&Setting::AnimationStyle));
        settings.background = BackgroundChoice::Classic;
        assert!(!settings.rows(false, false, false, false).contains(&Setting::AnimationStyle));
        settings.change(Setting::AnimationStyle, true);
        assert_eq!(settings.shader, Style::Plasma);
        settings.change(Setting::AnimationColors, false);
        assert_eq!(settings.palette, Palette::Neon);
        settings.change(Setting::Artwork, true);
        assert!(!settings.artwork);
        assert_eq!(settings.value_label(Setting::Artwork), "Off");
    }

    #[test]
    fn save_load_and_partial_files() {
        let dir = std::env::temp_dir().join(format!("consoleui-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let settings = Settings {
            accent: Accent::Green,
            animations: false,
            music_track: Some("a.ogg".into()),
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        // Missing fields get their defaults (e.g. after an update)
        fs::write(&path, r#"{"sounds": false}"#).unwrap();
        assert_eq!(Settings::load(&path), Settings { sounds: false, ..Settings::default() });
        fs::write(&path, r#"{"music": "ambient"}"#).unwrap();
        assert_eq!(Settings::load(&path).music, MusicChoice::Ambient);
        fs::write(&path, r#"{"keyboard_layout": "ch"}"#).unwrap();
        assert_eq!(Settings::load(&path).keyboard_layout, KeyboardLayout::Ch);
        fs::remove_dir_all(dir).unwrap();
    }
}
