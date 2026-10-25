use std::path::PathBuf;

use consoleui_core::Action;

pub const USAGE: &str = "\
Verwendung: consoleui [OPTIONEN]

  --windowed      Im Fenster statt im Vollbild starten
  --dummy         Erfundene Einträge statt echter Steam- und Desktop-Apps anzeigen
  --tiles <N>     Anzahl der Dummy-Kacheln (Standard: 12, nur mit --dummy)
  --fps           FPS- und Renderzeit-Anzeige einblenden
  --bench         Automatisch durch alle Kacheln scrollen, Messwerte ausgeben und beenden
  -h, --help      Diese Hilfe anzeigen

Für die Entwicklung:
  --start <SEITE>     Mit home, library oder search starten
  --query <TEXT>      In der Suche diesen Text vorbelegen (mit --start search)
  --snapshot <DATEI>  Nach kurzer Zeit ein PNG des Fensters speichern und beenden
  --actions <LISTE>   Eingaben simulieren, z. B. \"right,confirm,wait:3000,guide\"
                      (up, down, left, right, confirm, back, menu, search, secondary,
                      prev-tab, next-tab, guide, wait:<ms>)
  --dummy-launch <BEFEHL>  Dummy-Einträge starten diesen Befehl (z. B. glxgears)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartScreen {
    #[default]
    Home,
    Library,
    Search,
}

/// Ein Schritt aus `--actions`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptStep {
    Action(Action),
    Wait(u64),
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
                "-h" | "--help" => options.help = true,
                "--tiles" => {
                    let value = value(&mut args, "--tiles")?;
                    options.tiles =
                        value.parse().map_err(|_| format!("Ungültige Kachelanzahl: {value}"))?;
                }
                "--start" => {
                    options.start = match value(&mut args, "--start")?.as_str() {
                        "home" => StartScreen::Home,
                        "library" => StartScreen::Library,
                        "search" => StartScreen::Search,
                        other => return Err(format!("Unbekannte Seite: {other}")),
                    }
                }
                "--query" => options.query = Some(value(&mut args, "--query")?),
                "--snapshot" => options.snapshot = Some(value(&mut args, "--snapshot")?.into()),
                "--actions" => options.actions = parse_script(&value(&mut args, "--actions")?)?,
                "--dummy-launch" => options.dummy_launch = Some(value(&mut args, "--dummy-launch")?),
                other => return Err(format!("Unbekannte Option: {other}")),
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
                return ms.parse().map(ScriptStep::Wait).map_err(|_| format!("Ungültige Wartezeit: {step}"));
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
                other => return Err(format!("Unbekannte Aktion: {other}")),
            };
            Ok(ScriptStep::Action(action))
        })
        .collect()
}

fn value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{option} erwartet einen Wert"))
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
        let options = parse(&["--windowed", "--dummy", "--tiles", "50", "--fps", "--bench"]).unwrap();
        assert!(options.windowed && options.dummy && options.show_fps && options.bench);
        assert_eq!(options.tiles, 50);
    }

    #[test]
    fn parses_development_options() {
        let options = parse(&["--start", "search", "--query", "red", "--snapshot", "/tmp/x.png"]).unwrap();
        assert_eq!(options.start, StartScreen::Search);
        assert_eq!(options.query.as_deref(), Some("red"));
        assert_eq!(options.snapshot, Some(PathBuf::from("/tmp/x.png")));
        assert!(parse(&["--start", "nirgendwo"]).is_err());
    }

    #[test]
    fn parses_action_scripts() {
        let options = parse(&["--actions", "right, confirm,wait:500,guide", "--dummy-launch", "glxgears"]).unwrap();
        assert_eq!(
            options.actions,
            [
                ScriptStep::Action(Action::Right),
                ScriptStep::Action(Action::Confirm),
                ScriptStep::Wait(500),
                ScriptStep::Action(Action::Guide)
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
