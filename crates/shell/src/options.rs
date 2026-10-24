pub const USAGE: &str = "\
Verwendung: consoleui [OPTIONEN]

  --windowed      Im Fenster statt im Vollbild starten
  --tiles <N>     Anzahl der Dummy-Kacheln (Standard: 12)
  --fps           FPS- und Renderzeit-Anzeige einblenden
  --bench         Automatisch durch alle Kacheln scrollen, Messwerte ausgeben und beenden
  -h, --help      Diese Hilfe anzeigen";

#[derive(Debug, PartialEq)]
pub struct Options {
    pub windowed: bool,
    pub tiles: usize,
    pub show_fps: bool,
    pub bench: bool,
    pub help: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { windowed: false, tiles: 12, show_fps: false, bench: false, help: false }
    }
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--windowed" => options.windowed = true,
                "--fps" => options.show_fps = true,
                "--bench" => options.bench = true,
                "-h" | "--help" => options.help = true,
                "--tiles" => {
                    let value = args.next().ok_or_else(|| "--tiles erwartet eine Zahl".to_string())?;
                    options.tiles =
                        value.parse().map_err(|_| format!("Ungültige Kachelanzahl: {value}"))?;
                }
                other => return Err(format!("Unbekannte Option: {other}")),
            }
        }
        Ok(options)
    }
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
        let options = parse(&["--windowed", "--tiles", "50", "--fps", "--bench"]).unwrap();
        assert!(options.windowed && options.show_fps && options.bench);
        assert_eq!(options.tiles, 50);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&["--tiles"]).is_err());
        assert!(parse(&["--tiles", "viele"]).is_err());
        assert!(parse(&["--unbekannt"]).is_err());
    }
}
