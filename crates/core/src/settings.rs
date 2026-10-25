//! Einstellungen der Shell, gespeichert in `$XDG_CONFIG_HOME/consoleui/settings.json`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::xdg;

/// Farbe für Fokusrahmen, Buttons und ausgewählte Karten
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
            Self::White => "Weiß",
            Self::Blue => "Blau",
            Self::Violet => "Violett",
            Self::Green => "Grün",
            Self::Orange => "Orange",
            Self::Red => "Rot",
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

/// Welche Tastensymbole angezeigt werden
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GlyphChoice {
    /// Nach dem verbundenen Controller
    #[default]
    Auto,
    PlayStation,
    Xbox,
}

impl GlyphChoice {
    pub const ALL: [GlyphChoice; 3] = [Self::Auto, Self::PlayStation, Self::Xbox];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatisch",
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
}

impl Default for Settings {
    fn default() -> Self {
        Self { accent: Accent::default(), animations: true, sounds: true, glyphs: GlyphChoice::default() }
    }
}

/// Eine Zeile der Einstellungsseite
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Accent,
    Animations,
    Sounds,
    Glyphs,
    /// Öffnet die WLAN-Seite (Story 5.3)
    Wifi,
    /// Öffnet die Bluetooth-Seite (Story 5.4)
    Bluetooth,
}

impl Setting {
    pub const ALL: [Setting; 6] =
        [Self::Accent, Self::Animations, Self::Sounds, Self::Glyphs, Self::Wifi, Self::Bluetooth];

    /// Zeilen, die eine Unterseite öffnen, statt einen Wert zu ändern
    pub fn opens_page(self) -> bool {
        matches!(self, Self::Wifi | Self::Bluetooth)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Accent => "Akzentfarbe",
            Self::Animations => "Animationen",
            Self::Sounds => "Klänge",
            Self::Glyphs => "Tastensymbole",
            Self::Wifi => "WLAN",
            Self::Bluetooth => "Bluetooth",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Accent => "Farbe für Auswahl und Buttons",
            Self::Animations => "Übergänge und Bewegungen",
            Self::Sounds => "Klänge beim Navigieren",
            Self::Glyphs => "Symbole in den Hinweisen",
            Self::Wifi => "Netzwerke anzeigen und verbinden",
            Self::Bluetooth => "Controller und andere Geräte koppeln",
        }
    }
}

fn on_off(value: bool) -> &'static str {
    if value { "An" } else { "Aus" }
}

/// Nächster (`forward`) bzw. vorheriger Wert in einer Liste, mit Umlauf.
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

    /// Eine fehlende oder kaputte Datei ergibt die Standardwerte.
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|err| {
                eprintln!("Einstellungen {} sind ungültig und werden ignoriert: {err}", path.display());
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

    /// Wert der Zeile; Unterseiten (WLAN, Bluetooth) zeigt die Shell mit ihrem eigenen Zustand.
    pub fn value_label(&self, setting: Setting) -> &'static str {
        match setting {
            Setting::Accent => self.accent.label(),
            Setting::Animations => on_off(self.animations),
            Setting::Sounds => on_off(self.sounds),
            Setting::Glyphs => self.glyphs.label(),
            Setting::Wifi | Setting::Bluetooth => "",
        }
    }

    /// Wert ändern (Links/Rechts bzw. Bestätigen).
    pub fn change(&mut self, setting: Setting, forward: bool) {
        match setting {
            Setting::Accent => self.accent = cycle(&Accent::ALL, self.accent, forward),
            Setting::Animations => self.animations = !self.animations,
            Setting::Sounds => self.sounds = !self.sounds,
            Setting::Glyphs => self.glyphs = cycle(&GlyphChoice::ALL, self.glyphs, forward),
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
        assert_eq!(settings.value_label(Setting::Sounds), "Aus");
        settings.change(Setting::Glyphs, true);
        assert_eq!(settings.glyphs, GlyphChoice::PlayStation);
        // Unterseiten ändern keinen Wert
        settings.change(Setting::Wifi, true);
        assert_eq!(settings.glyphs, GlyphChoice::PlayStation);
        assert!(Setting::Bluetooth.opens_page() && !Setting::Sounds.opens_page());
    }

    #[test]
    fn save_load_and_partial_files() {
        let dir = std::env::temp_dir().join(format!("consoleui-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let settings = Settings { accent: Accent::Green, animations: false, ..Settings::default() };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        // Fehlende Felder bekommen Standardwerte (z. B. nach einem Update)
        fs::write(&path, r#"{"sounds": false}"#).unwrap();
        assert_eq!(Settings::load(&path), Settings { sounds: false, ..Settings::default() });
        fs::remove_dir_all(dir).unwrap();
    }
}
