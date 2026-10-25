//! Keyboard layout of our compositor's seat (Epic 7): the system layout, detected like other
//! compositors do it, and the layout the user picks in the settings ([`KeyboardLayout`]).
//!
//! Detection order ([`detect`]): the `XKB_DEFAULT_*` environment variables, then the X11 keymap
//! systemd-localed reports (`localectl status`), then the files localed writes:
//! `XKBLAYOUT=` lines in `/etc/vconsole.conf`, `/etc/X11/xorg.conf.d/00-keyboard.conf`, and as a
//! last resort the console keymap (`KEYMAP=de-nodeadkeys`) mapped to an X11 layout. Nothing found
//! gives xkbcommon's default (`us`).
//!
//! The shell's own on-screen keyboard (QWERTZ/QWERTY by language) is independent of this.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::i18n::tr;

/// An xkb RMLVO keymap description. Empty fields mean "xkbcommon's default".
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Keymap {
    pub rules: String,
    pub model: String,
    /// Comma-separated layouts, e.g. `de` or `de,us`
    pub layout: String,
    /// Comma-separated variants, one per layout, e.g. `nodeadkeys`
    pub variant: String,
    /// Comma-separated options, e.g. `compose:ralt`
    pub options: String,
}

/// Where [`detect`] found the keymap (for the log).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Environment,
    Localectl,
    Vconsole,
    XorgConf,
    /// Console keymap (`KEYMAP=` in `/etc/vconsole.conf`) mapped to an X11 layout
    ConsoleKeymap,
    Default,
}

const VCONSOLE: &str = "/etc/vconsole.conf";
const XORG_KEYBOARD: &str = "/etc/X11/xorg.conf.d/00-keyboard.conf";

impl Keymap {
    pub fn is_empty(&self) -> bool {
        self.layout.is_empty() && self.model.is_empty() && self.variant.is_empty() && self.options.is_empty()
    }

    /// Short description for logs and the settings row, e.g. `de (nodeadkeys)`.
    pub fn describe(&self) -> String {
        let layout = if self.layout.is_empty() { "us" } else { &self.layout };
        if self.variant.is_empty() { layout.to_owned() } else { format!("{layout} ({})", self.variant) }
    }

    /// `XKB_DEFAULT_RULES/MODEL/LAYOUT/VARIANT/OPTIONS`; `None` if none of them is set.
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let get = |name: &str| var(&format!("XKB_DEFAULT_{name}")).map(|v| v.trim().to_owned()).unwrap_or_default();
        let keymap = Self {
            rules: get("RULES"),
            model: get("MODEL"),
            layout: get("LAYOUT"),
            variant: get("VARIANT"),
            options: get("OPTIONS"),
        };
        (!keymap.is_empty()).then_some(keymap)
    }

    /// Output of `localectl status` (`X11 Layout: de`, `X11 Model: pc105`, …).
    pub fn from_localectl(output: &str) -> Option<Self> {
        let mut keymap = Self::default();
        for line in output.lines() {
            let Some((key, value)) = line.split_once(':') else { continue };
            let value = value.trim().to_owned();
            match key.trim() {
                "X11 Layout" => keymap.layout = value,
                "X11 Model" => keymap.model = value,
                "X11 Variant" => keymap.variant = value,
                "X11 Options" => keymap.options = value,
                _ => {}
            }
        }
        (!keymap.layout.is_empty()).then_some(keymap)
    }

    /// `XKBLAYOUT=`/`XKBMODEL=`/`XKBVARIANT=`/`XKBOPTIONS=` in `/etc/vconsole.conf` (written by
    /// newer systemd-localed next to `KEYMAP=`).
    pub fn from_vconsole(content: &str) -> Option<Self> {
        let mut keymap = Self::default();
        for (key, value) in shell_assignments(content) {
            match key {
                "XKBLAYOUT" => keymap.layout = value,
                "XKBMODEL" => keymap.model = value,
                "XKBVARIANT" => keymap.variant = value,
                "XKBOPTIONS" => keymap.options = value,
                _ => {}
            }
        }
        (!keymap.layout.is_empty()).then_some(keymap)
    }

    /// `Option "XkbLayout" "de"` lines of an xorg.conf `InputClass` section.
    pub fn from_xorg_conf(content: &str) -> Option<Self> {
        let mut keymap = Self::default();
        for line in content.lines() {
            let line = line.split('#').next().unwrap_or_default();
            let mut quoted = line.split('"').skip(1).step_by(2);
            if line.trim_start().starts_with("Option")
                && let (Some(key), Some(value)) = (quoted.next(), quoted.next())
            {
                let value = value.trim().to_owned();
                match key.to_ascii_lowercase().as_str() {
                    "xkblayout" => keymap.layout = value,
                    "xkbmodel" => keymap.model = value,
                    "xkbvariant" => keymap.variant = value,
                    "xkboptions" => keymap.options = value,
                    _ => {}
                }
            }
        }
        (!keymap.layout.is_empty()).then_some(keymap)
    }

    /// The console keymap (`KEYMAP=de-latin1-nodeadkeys`) as an X11 layout. Only the common
    /// names are mapped (`localectl` does this with the full `kbd-model-map`).
    pub fn from_console_keymap(content: &str) -> Option<Self> {
        let (_, keymap) = shell_assignments(content).find(|(key, _)| *key == "KEYMAP")?;
        let mut parts = keymap.split('-');
        let name = parts.next()?.to_ascii_lowercase();
        let layout = match name.as_str() {
            "uk" => "gb",
            "sg" | "de_ch" => "ch",
            "fr_ch" => "ch",
            "sv" => "se",
            "dk" | "de" | "us" | "gb" | "fr" | "es" | "it" | "at" | "be" | "nl" | "pt" | "se" | "no" | "fi" | "pl"
            | "ch" | "cz" | "hu" | "ru" => name.as_str(),
            _ => return None,
        };
        let variant = match name.as_str() {
            "fr_ch" => "fr",
            _ if parts.any(|p| p == "nodeadkeys") => "nodeadkeys",
            _ => "",
        };
        Some(Self { layout: layout.into(), variant: variant.into(), ..Self::default() })
    }

    /// This keymap with the layout the user picked; model and options (e.g. a compose key) stay.
    /// [`KeyboardLayout::Auto`] keeps the keymap as it is.
    pub fn with_choice(&self, choice: KeyboardLayout) -> Self {
        self.with_layout(choice.code())
    }

    /// Like [`Keymap::with_choice`] with an xkb layout name (from the control protocol); `None`
    /// or an empty name keeps the keymap.
    pub fn with_layout(&self, layout: Option<&str>) -> Self {
        match layout.map(str::trim).filter(|l| !l.is_empty()) {
            Some(layout) => Self { layout: layout.into(), variant: String::new(), ..self.clone() },
            None => self.clone(),
        }
    }
}

/// `KEY=value` / `KEY="value"` lines of a shell-style config file.
fn shell_assignments(content: &str) -> impl Iterator<Item = (&str, String)> {
    content.lines().filter_map(|line| {
        let line = line.trim();
        if line.starts_with('#') {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        Some((key.trim(), value.trim().trim_matches(['"', '\'']).to_owned()))
    })
}

/// Detects the system keymap with the given environment, `localectl` and file reader.
pub fn detect_with(
    var: impl Fn(&str) -> Option<String>,
    localectl: impl FnOnce() -> Option<String>,
    read: impl Fn(&Path) -> Option<String>,
) -> (Keymap, Source) {
    if let Some(keymap) = Keymap::from_env(var) {
        return (keymap, Source::Environment);
    }
    if let Some(keymap) = localectl().as_deref().and_then(Keymap::from_localectl) {
        return (keymap, Source::Localectl);
    }
    let vconsole = read(Path::new(VCONSOLE));
    if let Some(keymap) = vconsole.as_deref().and_then(Keymap::from_vconsole) {
        return (keymap, Source::Vconsole);
    }
    if let Some(keymap) = read(Path::new(XORG_KEYBOARD)).as_deref().and_then(Keymap::from_xorg_conf) {
        return (keymap, Source::XorgConf);
    }
    if let Some(keymap) = vconsole.as_deref().and_then(Keymap::from_console_keymap) {
        return (keymap, Source::ConsoleKeymap);
    }
    (Keymap::default(), Source::Default)
}

/// The system keymap of this machine.
pub fn detect() -> (Keymap, Source) {
    detect_with(
        |name| std::env::var(name).ok(),
        || {
            let output = Command::new("localectl").arg("status").output().ok()?;
            output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
        },
        |path| std::fs::read_to_string(path).ok(),
    )
}

/// Keyboard layout picked in the settings (only shown under our compositor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyboardLayout {
    /// The system layout ([`detect`])
    #[default]
    Auto,
    De,
    At,
    Ch,
    Us,
    Gb,
    Fr,
    Be,
    Nl,
    Es,
    Pt,
    It,
    Se,
    No,
    Dk,
    Fi,
    Pl,
}

impl KeyboardLayout {
    pub const ALL: [KeyboardLayout; 17] = [
        Self::Auto,
        Self::De,
        Self::At,
        Self::Ch,
        Self::Us,
        Self::Gb,
        Self::Fr,
        Self::Be,
        Self::Nl,
        Self::Es,
        Self::Pt,
        Self::It,
        Self::Se,
        Self::No,
        Self::Dk,
        Self::Fi,
        Self::Pl,
    ];

    /// The xkb layout name; `None` for [`KeyboardLayout::Auto`].
    pub fn code(self) -> Option<&'static str> {
        Some(match self {
            Self::Auto => return None,
            Self::De => "de",
            Self::At => "at",
            Self::Ch => "ch",
            Self::Us => "us",
            Self::Gb => "gb",
            Self::Fr => "fr",
            Self::Be => "be",
            Self::Nl => "nl",
            Self::Es => "es",
            Self::Pt => "pt",
            Self::It => "it",
            Self::Se => "se",
            Self::No => "no",
            Self::Dk => "dk",
            Self::Fi => "fi",
            Self::Pl => "pl",
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => tr("Automatic"),
            Self::De => tr("German"),
            Self::At => tr("German (Austria)"),
            Self::Ch => tr("German (Switzerland)"),
            Self::Us => tr("English (US)"),
            Self::Gb => tr("English (UK)"),
            Self::Fr => tr("French"),
            Self::Be => tr("Belgian"),
            Self::Nl => tr("Dutch"),
            Self::Es => tr("Spanish"),
            Self::Pt => tr("Portuguese"),
            Self::It => tr("Italian"),
            Self::Se => tr("Swedish"),
            Self::No => tr("Norwegian"),
            Self::Dk => tr("Danish"),
            Self::Fi => tr("Finnish"),
            Self::Pl => tr("Polish"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keymap(layout: &str, model: &str, variant: &str, options: &str) -> Keymap {
        Keymap {
            layout: layout.into(),
            model: model.into(),
            variant: variant.into(),
            options: options.into(),
            ..Keymap::default()
        }
    }

    #[test]
    fn parses_localectl_status() {
        let output = "   System Locale: LANG=de_DE.UTF-8\n       VC Keymap: de-nodeadkeys\n      X11 Layout: de\n       \
                      X11 Model: pc105\n     X11 Variant: nodeadkeys\n     X11 Options: compose:ralt\n";
        assert_eq!(Keymap::from_localectl(output), Some(keymap("de", "pc105", "nodeadkeys", "compose:ralt")));
        assert_eq!(Keymap::from_localectl("System Locale: LANG=C\n VC Keymap: (unset)\n"), None);
    }

    #[test]
    fn parses_vconsole_and_its_console_keymap() {
        let content = "KEYMAP=\"de-nodeadkeys\"\nFONT=\"eurlatgr\"\n";
        assert_eq!(Keymap::from_vconsole(content), None);
        assert_eq!(Keymap::from_console_keymap(content), Some(keymap("de", "", "nodeadkeys", "")));
        let content = "# localed\nKEYMAP=uk\nXKBLAYOUT=gb\nXKBMODEL=pc105\nXKBOPTIONS='ctrl:nocaps'\n";
        assert_eq!(Keymap::from_vconsole(content), Some(keymap("gb", "pc105", "", "ctrl:nocaps")));
        assert_eq!(Keymap::from_console_keymap("KEYMAP=uk"), Some(keymap("gb", "", "", "")));
        assert_eq!(Keymap::from_console_keymap("KEYMAP=sg-latin1"), Some(keymap("ch", "", "", "")));
        assert_eq!(Keymap::from_console_keymap("KEYMAP=fr_CH"), Some(keymap("ch", "", "fr", "")));
        assert_eq!(Keymap::from_console_keymap("KEYMAP=dvorak-programmer"), None);
        assert_eq!(Keymap::from_console_keymap("FONT=x"), None);
    }

    #[test]
    fn parses_xorg_keyboard_conf() {
        let content = "# Written by systemd-localed(8)\nSection \"InputClass\"\n        Identifier \"system-keyboard\"\n        \
                       MatchIsKeyboard \"on\"\n        Option \"XkbLayout\" \"de,us\"\n        Option \"XkbModel\" \
                       \"pc105\"\n        Option \"XkbVariant\" \"nodeadkeys,\"\n#       Option \"XkbOptions\" \
                       \"x\"\nEndSection\n";
        assert_eq!(Keymap::from_xorg_conf(content), Some(keymap("de,us", "pc105", "nodeadkeys,", "")));
        assert_eq!(Keymap::from_xorg_conf("Section \"InputClass\"\nEndSection\n"), None);
    }

    #[test]
    fn detection_order() {
        let no_env = |_: &str| None;
        let files = |path: &Path| match path.to_str() {
            Some(VCONSOLE) => Some("KEYMAP=fr-latin9\n".to_owned()),
            Some(XORG_KEYBOARD) => Some("Option \"XkbLayout\" \"it\"\n".to_owned()),
            _ => None,
        };
        let env = |name: &str| (name == "XKB_DEFAULT_LAYOUT").then(|| "se".to_owned());
        assert_eq!(detect_with(env, || panic!("not asked"), files), (keymap("se", "", "", ""), Source::Environment));
        let localectl = || Some("X11 Layout: es\n".to_owned());
        assert_eq!(detect_with(no_env, localectl, files), (keymap("es", "", "", ""), Source::Localectl));
        assert_eq!(detect_with(no_env, || None, files), (keymap("it", "", "", ""), Source::XorgConf));
        let only_console = |path: &Path| (path == Path::new(VCONSOLE)).then(|| "KEYMAP=fr-latin9\n".to_owned());
        assert_eq!(detect_with(no_env, || None, only_console), (keymap("fr", "", "", ""), Source::ConsoleKeymap));
        assert_eq!(detect_with(no_env, || None, |_| None), (Keymap::default(), Source::Default));
        // An empty variable does not count
        let empty = |name: &str| (name == "XKB_DEFAULT_LAYOUT").then(String::new);
        assert_eq!(detect_with(empty, || None, |_| None).1, Source::Default);
    }

    #[test]
    fn choice_overrides_layout_but_keeps_model_and_options() {
        let system = keymap("de", "pc105", "nodeadkeys", "compose:ralt");
        assert_eq!(system.with_choice(KeyboardLayout::Auto), system);
        assert_eq!(system.with_choice(KeyboardLayout::Us), keymap("us", "pc105", "", "compose:ralt"));
        assert_eq!(system.with_layout(Some(" ")), system);
        assert_eq!(system.with_layout(None), system);
        assert_eq!(system.with_layout(Some("fr")).layout, "fr");
        assert_eq!(system.describe(), "de (nodeadkeys)");
        assert_eq!(Keymap::default().describe(), "us");
    }

    #[test]
    fn choices_serialize_as_layout_codes() {
        assert_eq!(serde_json::to_string(&KeyboardLayout::Gb).unwrap(), "\"gb\"");
        assert_eq!(serde_json::to_string(&KeyboardLayout::Auto).unwrap(), "\"auto\"");
        for choice in KeyboardLayout::ALL {
            let json = serde_json::to_string(&choice).unwrap();
            assert_eq!(serde_json::from_str::<KeyboardLayout>(&json).unwrap(), choice);
            if let Some(code) = choice.code() {
                assert_eq!(json, format!("\"{code}\""));
            }
        }
    }
}
