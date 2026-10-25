//! Shell settings, stored in `$XDG_CONFIG_HOME/consoleui/settings.json`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub accent: Accent,
    pub animations: bool,
    pub sounds: bool,
    pub glyphs: GlyphChoice,
    pub language: LanguageChoice,
    /// Layout of the physical keyboard in our compositor (the on-screen keyboard follows the
    /// language instead)
    pub keyboard_layout: KeyboardLayout,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            accent: Accent::default(),
            animations: true,
            sounds: true,
            glyphs: GlyphChoice::default(),
            language: LanguageChoice::default(),
            keyboard_layout: KeyboardLayout::default(),
        }
    }
}

/// A row of the settings page
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Accent,
    Animations,
    Sounds,
    Glyphs,
    /// Story 6.6
    Language,
    /// Only under our compositor, which applies it to the seat keyboard
    KeyboardLayout,
    /// Opens the Wi-Fi page (Story 5.3)
    Wifi,
    /// Opens the Bluetooth page (Story 5.4)
    Bluetooth,
}

impl Setting {
    pub const ALL: [Setting; 8] = [
        Self::Accent,
        Self::Animations,
        Self::Sounds,
        Self::Glyphs,
        Self::Language,
        Self::KeyboardLayout,
        Self::Wifi,
        Self::Bluetooth,
    ];

    /// The rows of the settings page; the keyboard layout only where it has an effect (in our
    /// compositor; gamescope and the desktop use the layout of their own session).
    pub fn rows(keyboard_layout: bool) -> Vec<Setting> {
        Self::ALL.into_iter().filter(|&row| keyboard_layout || row != Self::KeyboardLayout).collect()
    }

    /// Rows that open a sub-page instead of changing a value
    pub fn opens_page(self) -> bool {
        matches!(self, Self::Wifi | Self::Bluetooth)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Accent => tr("Accent color"),
            Self::Animations => tr("Animations"),
            Self::Sounds => tr("Sounds"),
            Self::Glyphs => tr("Button symbols"),
            Self::Language => tr("Language"),
            Self::KeyboardLayout => tr("Keyboard layout"),
            Self::Wifi => tr("Wi-Fi"),
            Self::Bluetooth => tr("Bluetooth"),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Accent => tr("Color of selection and buttons"),
            Self::Animations => tr("Transitions and motion"),
            Self::Sounds => tr("Sounds while navigating"),
            Self::Glyphs => tr("Symbols in the button hints"),
            Self::Language => tr("Language of the interface"),
            Self::KeyboardLayout => tr("Layout of the connected keyboard"),
            Self::Wifi => tr("Show and join networks"),
            Self::Bluetooth => tr("Pair controllers and other devices"),
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

impl Settings {
    /// `$XDG_CONFIG_HOME/consoleui/settings.json`
    pub fn default_path() -> Option<PathBuf> {
        xdg::config_home().map(|dir| dir.join("consoleui/settings.json"))
    }

    /// A missing or broken file gives the defaults.
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|err| {
                eprintln!("Settings {} are invalid and ignored: {err}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        fs::rename(tmp, path)
    }

    /// Value of the row; for sub-pages (Wi-Fi, Bluetooth) the shell shows their own state.
    pub fn value_label(&self, setting: Setting) -> &'static str {
        match setting {
            Setting::Accent => self.accent.label(),
            Setting::Animations => on_off(self.animations),
            Setting::Sounds => on_off(self.sounds),
            Setting::Glyphs => self.glyphs.label(),
            Setting::Language => self.language.label(),
            Setting::KeyboardLayout => self.keyboard_layout.label(),
            Setting::Wifi | Setting::Bluetooth => "",
        }
    }

    /// Changes the value (left/right or confirm).
    pub fn change(&mut self, setting: Setting, forward: bool) {
        match setting {
            Setting::Accent => self.accent = cycle(&Accent::ALL, self.accent, forward),
            Setting::Animations => self.animations = !self.animations,
            Setting::Sounds => self.sounds = !self.sounds,
            Setting::Glyphs => self.glyphs = cycle(&GlyphChoice::ALL, self.glyphs, forward),
            Setting::Language => self.language = cycle(&LanguageChoice::ALL, self.language, forward),
            Setting::KeyboardLayout => {
                self.keyboard_layout = cycle(&KeyboardLayout::ALL, self.keyboard_layout, forward);
            }
            Setting::Wifi | Setting::Bluetooth => {}
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
    fn keyboard_layout_row_only_in_our_compositor() {
        assert!(!Setting::rows(false).contains(&Setting::KeyboardLayout));
        assert_eq!(Setting::rows(false).len(), Setting::ALL.len() - 1);
        assert_eq!(Setting::rows(true), Setting::ALL.to_vec());
    }

    #[test]
    fn save_load_and_partial_files() {
        let dir = std::env::temp_dir().join(format!("consoleui-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let settings = Settings { accent: Accent::Green, animations: false, ..Settings::default() };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        // Missing fields get their defaults (e.g. after an update)
        fs::write(&path, r#"{"sounds": false}"#).unwrap();
        assert_eq!(Settings::load(&path), Settings { sounds: false, ..Settings::default() });
        fs::write(&path, r#"{"keyboard_layout": "ch"}"#).unwrap();
        assert_eq!(Settings::load(&path).keyboard_layout, KeyboardLayout::Ch);
        fs::remove_dir_all(dir).unwrap();
    }
}
