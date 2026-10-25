//! Volume (PipeWire via `wpctl`) and power actions (logind via `systemctl`).

use std::io;
use std::process::Command;

const SINK: &str = "@DEFAULT_AUDIO_SINK@";
/// Step for volume up/down in percent
pub const VOLUME_STEP: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Volume {
    pub percent: u8,
    pub muted: bool,
}

pub fn volume() -> Option<Volume> {
    let output = Command::new("wpctl").args(["get-volume", SINK]).output().ok()?;
    parse_volume(&String::from_utf8_lossy(&output.stdout))
}

/// `wpctl get-volume` prints e.g. "Volume: 0.30" or "Volume: 0.45 [MUTED]".
fn parse_volume(output: &str) -> Option<Volume> {
    let rest = output.trim().strip_prefix("Volume:")?.trim();
    let value: f32 = rest.split_whitespace().next()?.parse().ok()?;
    Some(Volume { percent: (value * 100.0).round().clamp(0.0, 100.0) as u8, muted: rest.contains("[MUTED]") })
}

/// Volume up (`up = true`) or down by [`VOLUME_STEP`], at most 100 %.
pub fn change_volume(up: bool) -> io::Result<()> {
    let step = format!("{VOLUME_STEP}%{}", if up { '+' } else { '-' });
    run(Command::new("wpctl").args(["set-volume", "-l", "1.0", SINK, &step]))
}

pub fn toggle_mute() -> io::Result<()> {
    run(Command::new("wpctl").args(["set-mute", SINK, "toggle"]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Suspend,
    Reboot,
    PowerOff,
}

impl PowerAction {
    fn verb(self) -> &'static str {
        match self {
            Self::Suspend => "suspend",
            Self::Reboot => "reboot",
            Self::PowerOff => "poweroff",
        }
    }
}

/// logind allows this for the active local session without root (polkit).
pub fn power(action: PowerAction) -> io::Result<()> {
    run(Command::new("systemctl").arg(action.verb()))
}

fn run(command: &mut Command) -> io::Result<()> {
    let status = command.status()?;
    if status.success() { Ok(()) } else { Err(io::Error::other(format!("{command:?} failed ({status})"))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wpctl_output() {
        assert_eq!(parse_volume("Volume: 0.30\n"), Some(Volume { percent: 30, muted: false }));
        assert_eq!(parse_volume("Volume: 0.45 [MUTED]\n"), Some(Volume { percent: 45, muted: true }));
        assert_eq!(parse_volume("Volume: 1.20"), Some(Volume { percent: 100, muted: false }));
        assert_eq!(parse_volume("Error"), None);
    }
}
