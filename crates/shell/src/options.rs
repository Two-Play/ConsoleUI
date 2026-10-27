use std::path::PathBuf;

use consoleui_core::Action;

use crate::shell::Click;

pub const USAGE: &str = "\
Usage: consoleui [OPTIONS]

  --windowed      Start in a window instead of fullscreen
  --dummy         Show made-up entries instead of real Steam and desktop apps
  --tiles <N>     Number of dummy tiles (default: 12, only with --dummy)
  --profile <NAME>  Play as this profile (ID or name) without asking who is playing
  --fps           Show the FPS and render-time overlay
  --bench         Scroll through all tiles automatically, print measurements and exit
  -h, --help      Show this help

For development:
  --start <PAGE>      Start with home, library, settings, search or profiles
  --query <TEXT>      Prefill this text in the search (with --start search)
  --snapshot <FILE>   Save a PNG of the window after a short time and exit
  --actions <LIST>    Simulate input, e.g. \"right,confirm,wait:3000,guide\"
                      (up, down, left, right, confirm, back, menu, search, secondary,
                      prev-tab, next-tab, guide, wait:<ms>, toast = test notice)
  --dummy-launch <COMMAND>  Dummy entries launch this command (e.g. glxgears)
  --fake-network      Simulate Wi-Fi and Bluetooth instead of asking NetworkManager/BlueZ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartScreen {
    #[default]
    Home,
    Library,
    Settings,
    Search,
    /// The "Who's playing?" screen, even with only one profile
    Profiles,
}

/// One step of `--actions`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptStep {
    Action(Action),
    Wait(u64),
    /// A pointer click, e.g. `click-tab:1`, `click-tile:3`, `click-library:2`, `click-row:4`
    Click(Click),
    /// Development aid: shows a test notice (e.g. to check toasts over games)
    Toast,
}

#[derive(Debug, PartialEq)]
pub struct Options {
    pub windowed: bool,
    pub dummy: bool,
    pub tiles: usize,
    pub show_fps: bool,
    pub bench: bool,
    pub help: bool,
    pub start: StartScreen,
    pub query: Option<String>,
    pub snapshot: Option<PathBuf>,
    pub actions: Vec<ScriptStep>,
    pub dummy_launch: Option<String>,
    pub fake_network: bool,
    pub profile: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            windowed: false,
            dummy: false,
            tiles: 12,
            show_fps: false,
            bench: false,
            help: false,
            start: StartScreen::Home,
            query: None,
            snapshot: None,
            actions: Vec::new(),
            dummy_launch: None,
            fake_network: false,
            profile: None,
        }
    }
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--windowed" => options.windowed = true,
                "--dummy" => options.dummy = true,
                "--fps" => options.show_fps = true,
                "--bench" => options.bench = true,
                "--fake-network" => options.fake_network = true,
                "-h" | "--help" => options.help = true,
                "--tiles" => {
                    let value = value(&mut args, "--tiles")?;
                    options.tiles = value.parse().map_err(|_| format!("Invalid tile count: {value}"))?;
                }
                "--start" => {
                    options.start = match value(&mut args, "--start")?.as_str() {
                        "home" => StartScreen::Home,
                        "library" => StartScreen::Library,
                        "settings" => StartScreen::Settings,
                        "search" => StartScreen::Search,
                        "profiles" => StartScreen::Profiles,
                        other => return Err(format!("Unknown page: {other}")),
                    }
                }
                "--profile" => options.profile = Some(value(&mut args, "--profile")?),
                "--query" => options.query = Some(value(&mut args, "--query")?),
                "--snapshot" => options.snapshot = Some(value(&mut args, "--snapshot")?.into()),
                "--actions" => options.actions = parse_script(&value(&mut args, "--actions")?)?,
                "--dummy-launch" => options.dummy_launch = Some(value(&mut args, "--dummy-launch")?),
                other => return Err(format!("Unknown option: {other}")),
            }
        }
        Ok(options)
    }
}

fn parse_script(list: &str) -> Result<Vec<ScriptStep>, String> {
    list.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|step| {
            if let Some(ms) = step.strip_prefix("wait:") {
                return ms.parse().map(ScriptStep::Wait).map_err(|_| format!("Invalid wait time: {step}"));
            }
            if let Some((kind, index)) = step.strip_prefix("click-").and_then(|rest| rest.split_once(':')) {
                let index: usize = index.parse().map_err(|_| format!("Invalid click: {step}"))?;
                return match kind {
                    "tab" => Ok(Click::Tab(index)),
                    "tile" => Ok(Click::HomeTile(index)),
                    "library" => Ok(Click::LibraryTile(index)),
                    "row" => Ok(Click::SettingsRow(index)),
                    "dialog" => Ok(Click::DialogButton(index)),
                    "quick" => Ok(Click::QuickCard(index)),
                    _ => Err(format!("Unknown click: {step}")),
                }
                .map(ScriptStep::Click);
            }
            if step == "toast" {
                return Ok(ScriptStep::Toast);
            }
            let action = match step {
                "up" => Action::Up,
                "down" => Action::Down,
                "left" => Action::Left,
                "right" => Action::Right,
                "confirm" => Action::Confirm,
                "back" => Action::Back,
                "menu" => Action::Menu,
                "search" => Action::Search,
                "secondary" => Action::Secondary,
                "prev-tab" => Action::PrevTab,
                "next-tab" => Action::NextTab,
                "guide" => Action::Guide,
                other => return Err(format!("Unknown action: {other}")),
            };
            Ok(ScriptStep::Action(action))
        })
        .collect()
}

fn value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{option} expects a value"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults_without_arguments() {
        assert_eq!(parse(&[]), Ok(Options::default()));
    }

    #[test]
    fn parses_all_flags() {
        let options = parse(&["--windowed", "--dummy", "--tiles", "50", "--fps", "--bench", "--fake-network"]).unwrap();
        assert!(options.windowed && options.dummy && options.show_fps && options.bench && options.fake_network);
        assert_eq!(options.tiles, 50);
    }

    #[test]
    fn parses_development_options() {
        let options = parse(&["--start", "search", "--query", "red", "--snapshot", "/tmp/x.png"]).unwrap();
        assert_eq!(options.start, StartScreen::Search);
        assert_eq!(options.query.as_deref(), Some("red"));
        assert_eq!(options.snapshot, Some(PathBuf::from("/tmp/x.png")));
        assert!(parse(&["--start", "nirgendwo"]).is_err());
        let options = parse(&["--start", "profiles", "--profile", "Anna"]).unwrap();
        assert_eq!(options.start, StartScreen::Profiles);
        assert_eq!(options.profile.as_deref(), Some("Anna"));
        assert!(parse(&["--profile"]).is_err());
    }

    #[test]
    fn parses_action_scripts() {
        let options =
            parse(&["--actions", "right, confirm,wait:500,guide,toast", "--dummy-launch", "glxgears"]).unwrap();
        assert_eq!(
            options.actions,
            [
                ScriptStep::Action(Action::Right),
                ScriptStep::Action(Action::Confirm),
                ScriptStep::Wait(500),
                ScriptStep::Action(Action::Guide),
                ScriptStep::Toast
            ]
        );
        assert_eq!(options.dummy_launch.as_deref(), Some("glxgears"));
        assert!(parse(&["--actions", "jump"]).is_err());
        assert!(parse(&["--actions", "wait:bald"]).is_err());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&["--tiles"]).is_err());
        assert!(parse(&["--tiles", "viele"]).is_err());
        assert!(parse(&["--unbekannt"]).is_err());
    }
}
