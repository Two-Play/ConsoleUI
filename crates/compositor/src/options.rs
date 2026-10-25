//! Command line of `consoleui-compositor` (hand-rolled like the shell's).

use std::path::PathBuf;
use std::time::Duration;

pub const USAGE: &str = "\
Usage: consoleui-compositor [OPTIONS] [-- SHELL [ARGS...]]

Runs the ConsoleUI compositor and starts the shell inside it.

Options:
  --winit                      nested in a window on the current desktop (development)
  --drm                        on a TTY: DRM/KMS, libinput, libseat (the console session)
                               (default: --winit when WAYLAND_DISPLAY or DISPLAY is set, else --drm)
  --size WxH                   window size for --winit (default 1280x720)
  --shell CMD [ARGS...]        the shell to start (same as after --; default: consoleui)
  --no-shell                   start no shell (clients can connect to the printed socket)
  --no-xwayland                do not start XWayland (no DISPLAY for the shell and its apps)
  --screenshot-after MS PNG    save the composed output as PNG MS milliseconds after the start
                               (development; may be given several times)
  -h, --help                   this help";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendChoice {
    Winit,
    Drm,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub backend: BackendChoice,
    pub size: (u32, u32),
    /// `None`: start no shell
    pub shell: Option<Vec<String>>,
    pub xwayland: bool,
    pub screenshots: Vec<(Duration, PathBuf)>,
    pub help: bool,
}

impl Options {
    /// `nested` says whether a desktop session is around (for the default backend).
    pub fn parse(args: impl IntoIterator<Item = String>, nested: bool) -> Result<Self, String> {
        let mut options = Options {
            backend: if nested { BackendChoice::Winit } else { BackendChoice::Drm },
            size: (1280, 720),
            shell: Some(vec!["consoleui".into()]),
            xwayland: true,
            screenshots: Vec::new(),
            help: false,
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--winit" => options.backend = BackendChoice::Winit,
                "--drm" => options.backend = BackendChoice::Drm,
                "--size" => {
                    let value = args.next().ok_or("--size needs WxH")?;
                    options.size = parse_size(&value).ok_or_else(|| format!("invalid size: {value}"))?;
                }
                "--no-shell" => options.shell = None,
                "--no-xwayland" => options.xwayland = false,
                "--screenshot-after" => {
                    let ms = args.next().ok_or("--screenshot-after needs MS and PNG")?;
                    let ms: u64 = ms.parse().map_err(|_| format!("invalid milliseconds: {ms}"))?;
                    let path = args.next().ok_or("--screenshot-after needs MS and PNG")?;
                    options.screenshots.push((Duration::from_millis(ms), path.into()));
                }
                "--shell" | "--" => {
                    let command: Vec<String> = args.by_ref().collect();
                    if command.is_empty() {
                        return Err(format!("{arg} needs a command"));
                    }
                    options.shell = Some(command);
                }
                "-h" | "--help" => options.help = true,
                other => return Err(format!("unknown option: {other}")),
            }
        }
        Ok(options)
    }
}

fn parse_size(value: &str) -> Option<(u32, u32)> {
    let (w, h) = value.split_once('x')?;
    let size = (w.parse().ok()?, h.parse().ok()?);
    (size.0 > 0 && size.1 > 0).then_some(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|s| s.to_string()), true)
    }

    #[test]
    fn defaults_follow_the_environment() {
        assert_eq!(parse(&[]).unwrap().backend, BackendChoice::Winit);
        assert_eq!(Options::parse(Vec::new(), false).unwrap().backend, BackendChoice::Drm);
        assert_eq!(parse(&[]).unwrap().shell, Some(vec!["consoleui".to_string()]));
    }

    #[test]
    fn shell_command_takes_the_rest() {
        let options = parse(&["--drm", "--", "consoleui", "--dummy", "--size"]).unwrap();
        assert_eq!(options.backend, BackendChoice::Drm);
        assert_eq!(options.shell.unwrap(), ["consoleui", "--dummy", "--size"]);
        let options = parse(&["--shell", "sh", "-c", "true"]).unwrap();
        assert_eq!(options.shell.unwrap(), ["sh", "-c", "true"]);
        assert!(parse(&["--"]).is_err());
    }

    #[test]
    fn parses_size_and_screenshots() {
        let options =
            parse(&["--size", "960x540", "--screenshot-after", "1500", "/tmp/a.png", "--no-xwayland"]).unwrap();
        assert_eq!(options.size, (960, 540));
        assert_eq!(options.screenshots, vec![(Duration::from_millis(1500), PathBuf::from("/tmp/a.png"))]);
        assert!(!options.xwayland);
        assert!(parse(&["--size", "0x5"]).is_err());
        assert!(parse(&["--screenshot-after", "x", "a.png"]).is_err());
        assert!(parse(&["--bogus"]).is_err());
        assert_eq!(parse(&["--no-shell"]).unwrap().shell, None);
    }
}
