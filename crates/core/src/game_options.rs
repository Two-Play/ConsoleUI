//! Per-entry option to start a game inside its own nested gamescope (Story 7.10): render
//! resolution, upscaler, FPS limit, HDR and fullscreen. Saved as
//! `$XDG_DATA_HOME/consoleui/game_options.json` next to `collection.json`.
//!
//! Only entries we start ourselves (`Launch::Command`) can be wrapped. Steam games are started by
//! the running Steam client (`steam://rungameid/<id>`), so the game process is a child of Steam,
//! not of us. Steam has no command-line switch for a per-launch wrapper; the only ways are the
//! per-game launch options (`gamescope … -- %command%`) or a compatibility tool, and both live in
//! Steam's own config files (`localconfig.vdf`/`config.vdf`), which Steam rewrites while it runs.
//! We therefore do not touch them and show the option as unavailable for Steam entries.

use std::collections::BTreeMap;

use crate::i18n::tr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::library::Launch;
use crate::nav::Action;
use crate::xdg;

/// Resolution the game renders at (the height; the width follows the output's aspect ratio).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resolution {
    /// Same as the output: no `-w`/`-h`
    #[default]
    Native,
    P720,
    P800,
    P1080,
    P1440,
}

impl Resolution {
    pub const ALL: [Resolution; 5] = [Self::Native, Self::P720, Self::P800, Self::P1080, Self::P1440];

    pub fn height(self) -> Option<u32> {
        match self {
            Self::Native => None,
            Self::P720 => Some(720),
            Self::P800 => Some(800),
            Self::P1080 => Some(1080),
            Self::P1440 => Some(1440),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Native => tr("Native"),
            Self::P720 => "720p",
            Self::P800 => "800p",
            Self::P1080 => "1080p",
            Self::P1440 => "1440p",
        }
    }
}

/// How gamescope scales the game image up to the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Upscaler {
    /// gamescope's default (linear filter)
    #[default]
    Off,
    /// AMD FidelityFX Super Resolution 1.0 (`-F fsr`)
    Fsr,
    /// NVIDIA Image Scaling (`-F nis`)
    Nis,
    /// Whole-number factors with sharp pixels (`-S integer -F nearest`)
    Integer,
}

impl Upscaler {
    pub const ALL: [Upscaler; 4] = [Self::Off, Self::Fsr, Self::Nis, Self::Integer];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => tr("Off"),
            Self::Fsr => "FSR",
            Self::Nis => "NIS",
            Self::Integer => tr("Integer"),
        }
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Off => &[],
            Self::Fsr => &["-F", "fsr"],
            Self::Nis => &["-F", "nis"],
            Self::Integer => &["-S", "integer", "-F", "nearest"],
        }
    }
}

/// Frame limit via gamescope's nested refresh rate (`-r`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FpsLimit {
    #[default]
    Off,
    Fps30,
    Fps40,
    Fps60,
    Fps90,
}

impl FpsLimit {
    pub const ALL: [FpsLimit; 5] = [Self::Off, Self::Fps30, Self::Fps40, Self::Fps60, Self::Fps90];

    pub fn fps(self) -> Option<u32> {
        match self {
            Self::Off => None,
            Self::Fps30 => Some(30),
            Self::Fps40 => Some(40),
            Self::Fps60 => Some(60),
            Self::Fps90 => Some(90),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => tr("Off"),
            Self::Fps30 => "30 FPS",
            Self::Fps40 => "40 FPS",
            Self::Fps60 => "60 FPS",
            Self::Fps90 => "90 FPS",
        }
    }
}

/// Options of one entry. The default starts the entry directly, without gamescope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameOptions {
    /// Start inside a nested gamescope at all
    pub nested: bool,
    pub resolution: Resolution,
    pub upscaler: Upscaler,
    pub fps_limit: FpsLimit,
    pub hdr: bool,
    /// gamescope's window fullscreen (`-f`)
    pub fullscreen: bool,
}

impl Default for GameOptions {
    fn default() -> Self {
        Self {
            nested: false,
            resolution: Resolution::default(),
            upscaler: Upscaler::default(),
            fps_limit: FpsLimit::default(),
            hdr: false,
            fullscreen: true,
        }
    }
}

/// gamescope's executable
pub const GAMESCOPE: &str = "gamescope";

/// Only entries we start ourselves can run inside a nested gamescope (see the module docs).
pub fn supports(launch: &Launch) -> bool {
    matches!(launch, Launch::Command(_) | Launch::None)
}

impl GameOptions {
    /// gamescope arguments up to (excluding) `--`. `output` is the size of the window/screen the
    /// nested gamescope shows its image in; unknown, gamescope picks its own and the game
    /// resolution assumes 16:9.
    pub fn gamescope_args(&self, output: Option<(u32, u32)>) -> Vec<String> {
        let output = output.filter(|&(w, h)| w > 0 && h > 0);
        let mut args: Vec<String> = Vec::new();
        if let Some((width, height)) = output {
            args.extend(["-W".into(), width.to_string(), "-H".into(), height.to_string()]);
        }
        if let Some(game_height) = self.resolution.height() {
            let (out_w, out_h) = output.unwrap_or((16, 9));
            // Width from the output's aspect ratio, rounded to an even number
            let game_width = ((u64::from(game_height) * u64::from(out_w) + u64::from(out_h)) / (2 * u64::from(out_h))) * 2;
            args.extend(["-w".into(), game_width.to_string(), "-h".into(), game_height.to_string()]);
        }
        args.extend(self.upscaler.args().iter().map(|a| a.to_string()));
        if let Some(fps) = self.fps_limit.fps() {
            args.extend(["-r".into(), fps.to_string()]);
        }
        if self.hdr {
            args.push("--hdr-enabled".into());
        }
        if self.fullscreen {
            args.push("-f".into());
        }
        args
    }

    /// The launch with gamescope put in front: `gamescope <args> -- <command>`. Unchanged when the
    /// option is off or the entry cannot be wrapped (Steam, dummy entries without a command).
    pub fn wrap(&self, launch: &Launch, output: Option<(u32, u32)>) -> Launch {
        match launch {
            Launch::Command(argv) if self.nested && !argv.is_empty() => {
                let mut wrapped = vec![GAMESCOPE.to_string()];
                wrapped.extend(self.gamescope_args(output));
                wrapped.push("--".into());
                wrapped.extend(argv.iter().cloned());
                Launch::Command(wrapped)
            }
            other => other.clone(),
        }
    }

    /// Short preview of the prefix for the options panel, e.g. `gamescope -w 1280 -h 720 -F fsr -f --`.
    pub fn preview(&self, output: Option<(u32, u32)>) -> String {
        if !self.nested {
            return tr("Starts directly, without gamescope").into();
        }
        let mut parts = vec![GAMESCOPE.to_string()];
        parts.extend(self.gamescope_args(output));
        parts.push("-- …".into());
        parts.join(" ")
    }
}

/// One row of the options panel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameOption {
    Nested,
    Resolution,
    Upscaler,
    FpsLimit,
    Hdr,
    Fullscreen,
}

impl GameOption {
    pub const ALL: [GameOption; 6] =
        [Self::Nested, Self::Resolution, Self::Upscaler, Self::FpsLimit, Self::Hdr, Self::Fullscreen];

    pub fn label(self) -> &'static str {
        match self {
            Self::Nested => tr("Start in gamescope"),
            Self::Resolution => tr("Resolution"),
            Self::Upscaler => tr("Upscaler"),
            Self::FpsLimit => tr("FPS limit"),
            Self::Hdr => "HDR",
            Self::Fullscreen => tr("Fullscreen"),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Nested => tr("Own nested gamescope for this entry"),
            Self::Resolution => tr("The game renders at this size"),
            Self::Upscaler => tr("Scaling to the screen size"),
            Self::FpsLimit => tr("Frame rate of the nested display"),
            Self::Hdr => tr("HDR output (needs gamescope's WSI layer)"),
            Self::Fullscreen => tr("gamescope window covers the screen"),
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

impl GameOptions {
    pub fn value_label(&self, option: GameOption) -> &'static str {
        match option {
            GameOption::Nested => on_off(self.nested),
            GameOption::Resolution => self.resolution.label(),
            GameOption::Upscaler => self.upscaler.label(),
            GameOption::FpsLimit => self.fps_limit.label(),
            GameOption::Hdr => on_off(self.hdr),
            GameOption::Fullscreen => on_off(self.fullscreen),
        }
    }

    pub fn change(&mut self, option: GameOption, forward: bool) {
        match option {
            GameOption::Nested => self.nested = !self.nested,
            GameOption::Resolution => self.resolution = cycle(&Resolution::ALL, self.resolution, forward),
            GameOption::Upscaler => self.upscaler = cycle(&Upscaler::ALL, self.upscaler, forward),
            GameOption::FpsLimit => self.fps_limit = cycle(&FpsLimit::ALL, self.fps_limit, forward),
            GameOption::Hdr => self.hdr = !self.hdr,
            GameOption::Fullscreen => self.fullscreen = !self.fullscreen,
        }
    }

    /// Rows shown in the panel: only the switch while the option is off.
    pub fn visible_rows(&self) -> &'static [GameOption] {
        if self.nested { &GameOption::ALL } else { &GameOption::ALL[..1] }
    }
}

/// Options of all entries by entry ID. Entries with default options are not stored.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GameOptionsStore {
    entries: BTreeMap<String, GameOptions>,
}

impl GameOptionsStore {
    /// `$XDG_DATA_HOME/consoleui/game_options.json`
    pub fn default_path() -> Option<PathBuf> {
        xdg::data_home().map(|dir| dir.join("consoleui/game_options.json"))
    }

    /// A missing or broken file results in no options.
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|err| {
                eprintln!("Game options {} are invalid and ignored: {err}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Writes to a temporary file first so that a crash cannot destroy the file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        fs::rename(tmp, path)
    }

    pub fn get(&self, id: &str) -> GameOptions {
        self.entries.get(id).copied().unwrap_or_default()
    }

    pub fn set(&mut self, id: &str, options: GameOptions) {
        if options == GameOptions::default() {
            self.entries.remove(id);
        } else {
            self.entries.insert(id.to_string(), options);
        }
    }
}

/// State of the options panel for one entry (opened from the context menu).
#[derive(Debug, Clone)]
pub struct GameOptionsPanel {
    entry_id: String,
    title: String,
    /// `false` for Steam entries (see the module docs): the panel only explains why.
    available: bool,
    options: GameOptions,
    focused: usize,
}

/// What should happen after an input in the panel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelOutcome {
    Stay,
    /// The options changed and should be saved.
    Changed,
    Close,
}

impl GameOptionsPanel {
    pub fn new(entry_id: &str, title: &str, launch: &Launch, options: GameOptions) -> Self {
        Self { entry_id: entry_id.into(), title: title.into(), available: supports(launch), options, focused: 0 }
    }

    pub fn entry_id(&self) -> &str {
        &self.entry_id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn is_available(&self) -> bool {
        self.available
    }

    pub fn options(&self) -> GameOptions {
        self.options
    }

    pub fn focused(&self) -> usize {
        self.focused
    }

    /// Rows as (label, description, value).
    pub fn rows(&self) -> Vec<(&'static str, &'static str, &'static str)> {
        if !self.available {
            return vec![(
                tr("Not available for Steam games"),
                tr("Steam starts the game itself. Use gamescope … -- %command% in its launch options."),
                "",
            )];
        }
        self.options.visible_rows().iter().map(|&o| (o.label(), o.description(), self.options.value_label(o))).collect()
    }

    pub fn apply(&mut self, action: Action) -> PanelOutcome {
        if matches!(action, Action::Back | Action::Menu) {
            return PanelOutcome::Close;
        }
        if !self.available {
            return PanelOutcome::Stay;
        }
        let rows = self.options.visible_rows();
        match action {
            Action::Up => self.focused = self.focused.saturating_sub(1),
            Action::Down => self.focused = (self.focused + 1).min(rows.len() - 1),
            Action::Left | Action::Right | Action::Confirm => {
                self.options.change(rows[self.focused], action != Action::Left);
                self.focused = self.focused.min(self.options.visible_rows().len() - 1);
                return PanelOutcome::Changed;
            }
            _ => {}
        }
        PanelOutcome::Stay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(options: &GameOptions, output: Option<(u32, u32)>) -> String {
        options.gamescope_args(output).join(" ")
    }

    #[test]
    fn builds_gamescope_arguments() {
        let mut options = GameOptions { nested: true, ..GameOptions::default() };
        assert_eq!(args(&options, Some((1920, 1080))), "-W 1920 -H 1080 -f");
        options.fullscreen = false;
        assert_eq!(args(&options, None), "");

        options.resolution = Resolution::P720;
        options.upscaler = Upscaler::Fsr;
        options.fps_limit = FpsLimit::Fps40;
        options.hdr = true;
        assert_eq!(args(&options, Some((1920, 1080))), "-W 1920 -H 1080 -w 1280 -h 720 -F fsr -r 40 --hdr-enabled");
        // Width follows the aspect ratio of the output (16:10, 21:9), rounded to even
        options.resolution = Resolution::P800;
        options.upscaler = Upscaler::Integer;
        options.fps_limit = FpsLimit::Off;
        options.hdr = false;
        assert_eq!(args(&options, Some((2560, 1600))), "-W 2560 -H 1600 -w 1280 -h 800 -S integer -F nearest");
        options.resolution = Resolution::P720;
        options.upscaler = Upscaler::Nis;
        assert_eq!(args(&options, Some((3440, 1440))), "-W 3440 -H 1440 -w 1720 -h 720 -F nis");
        // Unknown (or empty) output: 16:9
        assert_eq!(args(&options, Some((0, 0))), "-w 1280 -h 720 -F nis");
    }

    #[test]
    fn wraps_only_commands_when_enabled() {
        let command = Launch::Command(vec!["glxgears".into(), "-info".into()]);
        let off = GameOptions::default();
        assert_eq!(off.wrap(&command, Some((1280, 720))), command);
        let on = GameOptions { nested: true, fps_limit: FpsLimit::Fps60, ..GameOptions::default() };
        assert_eq!(
            on.wrap(&command, Some((1280, 720))),
            Launch::Command(
                ["gamescope", "-W", "1280", "-H", "720", "-r", "60", "-f", "--", "glxgears", "-info"]
                    .map(String::from)
                    .to_vec()
            )
        );
        assert_eq!(on.wrap(&Launch::Steam(730), None), Launch::Steam(730));
        assert_eq!(on.wrap(&Launch::None, None), Launch::None);
        assert_eq!(on.wrap(&Launch::Command(vec![]), None), Launch::Command(vec![]));
        assert!(!supports(&Launch::Steam(730)) && supports(&command));
        assert!(on.preview(None).starts_with("gamescope -r 60 -f --"));
    }

    #[test]
    fn change_cycles_values() {
        let mut options = GameOptions::default();
        assert_eq!(options.visible_rows(), [GameOption::Nested]);
        options.change(GameOption::Nested, true);
        assert_eq!(options.visible_rows().len(), GameOption::ALL.len());
        options.change(GameOption::Resolution, false);
        assert_eq!(options.resolution, Resolution::P1440);
        options.change(GameOption::Resolution, true);
        assert_eq!(options.resolution, Resolution::Native);
        options.change(GameOption::Upscaler, true);
        options.change(GameOption::FpsLimit, true);
        options.change(GameOption::Hdr, true);
        options.change(GameOption::Fullscreen, true);
        assert_eq!(options.value_label(GameOption::Upscaler), "FSR");
        assert_eq!(options.value_label(GameOption::FpsLimit), "30 FPS");
        assert_eq!(options.value_label(GameOption::Hdr), "On");
        assert_eq!(options.value_label(GameOption::Fullscreen), "Off");
    }

    #[test]
    fn panel_navigation() {
        let launch = Launch::Command(vec!["glxgears".into()]);
        let mut panel = GameOptionsPanel::new("dummy:0", "Gears", &launch, GameOptions::default());
        assert_eq!(panel.rows().len(), 1);
        assert_eq!(panel.apply(Action::Down), PanelOutcome::Stay);
        assert_eq!(panel.focused(), 0);
        assert_eq!(panel.apply(Action::Confirm), PanelOutcome::Changed);
        assert!(panel.options().nested);
        assert_eq!(panel.rows().len(), 6);
        panel.apply(Action::Down);
        assert_eq!(panel.apply(Action::Right), PanelOutcome::Changed);
        assert_eq!(panel.options().resolution, Resolution::P720);
        for _ in 0..10 {
            panel.apply(Action::Down);
        }
        assert_eq!(panel.focused(), 5);
        // Switching off again hides the rows; the focus moves back to the switch.
        for _ in 0..10 {
            panel.apply(Action::Up);
        }
        panel.apply(Action::Left);
        assert!(!panel.options().nested && panel.focused() == 0);
        assert_eq!(panel.apply(Action::Back), PanelOutcome::Close);

        let mut steam = GameOptionsPanel::new("steam:730", "CS2", &Launch::Steam(730), GameOptions::default());
        assert!(!steam.is_available());
        assert_eq!(steam.rows().len(), 1);
        assert_eq!(steam.apply(Action::Confirm), PanelOutcome::Stay);
        assert!(!steam.options().nested);
        assert_eq!(steam.apply(Action::Menu), PanelOutcome::Close);
    }

    #[test]
    fn store_drops_defaults_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!("consoleui-game-options-{}", std::process::id()));
        let path = dir.join("nested/game_options.json");
        let mut store = GameOptionsStore::default();
        let options = GameOptions { nested: true, upscaler: Upscaler::Fsr, ..GameOptions::default() };
        store.set("desktop:gears.desktop", options);
        store.set("desktop:other.desktop", GameOptions::default());
        assert_eq!(store.entries.len(), 1);
        assert_eq!(store.get("desktop:gears.desktop"), options);
        assert_eq!(store.get("unknown"), GameOptions::default());
        store.save(&path).unwrap();
        assert_eq!(GameOptionsStore::load(&path), store);
        store.set("desktop:gears.desktop", GameOptions::default());
        assert_eq!(store, GameOptionsStore::default());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_partial_or_broken_file() {
        let dir = std::env::temp_dir().join(format!("consoleui-game-options-broken-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game_options.json");
        assert_eq!(GameOptionsStore::load(&path), GameOptionsStore::default());
        // Missing fields get their defaults (e.g. after an update)
        fs::write(&path, r#"{"desktop:a": {"nested": true, "fps_limit": "fps60"}}"#).unwrap();
        let loaded = GameOptionsStore::load(&path).get("desktop:a");
        assert_eq!(loaded, GameOptions { nested: true, fps_limit: FpsLimit::Fps60, ..GameOptions::default() });
        fs::write(&path, "{broken").unwrap();
        assert_eq!(GameOptionsStore::load(&path), GameOptionsStore::default());
        fs::remove_dir_all(dir).unwrap();
    }
}
