//! Volume (PipeWire via `wpctl`), screen brightness (sysfs, set through logind) and power actions
//! (logind via `systemctl`).

use std::path::Path;
use std::process::Command;
use std::{fs, io};

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

const BACKLIGHT_DIR: &str = "/sys/class/backlight";
/// Step for brightness up/down in percent
pub const BRIGHTNESS_STEP: u32 = 5;
/// Lowest brightness in percent: some panels switch off at 0, and then nothing can be read anymore
const MIN_BRIGHTNESS: u32 = 5;

/// The screen's backlight, if it has one (desktop monitors usually do not)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlight {
    name: String,
    value: u32,
    max: u32,
}

impl Backlight {
    pub fn percent(&self) -> u8 {
        ((self.value as u64 * 100 + self.max as u64 / 2) / self.max as u64).min(100) as u8
    }

    /// The raw value one step up or down, never below [`MIN_BRIGHTNESS`].
    fn stepped(&self, up: bool) -> u32 {
        let percent = self.percent() as u32;
        let percent = if up { percent + BRIGHTNESS_STEP } else { percent.saturating_sub(BRIGHTNESS_STEP) };
        let percent = percent.clamp(MIN_BRIGHTNESS, 100);
        (self.max as u64 * percent as u64 / 100) as u32
    }
}

pub fn backlight() -> Option<Backlight> {
    read_backlight(Path::new(BACKLIGHT_DIR))
}

/// The device to use like the kernel recommends: firmware before platform before raw.
fn read_backlight(dir: &Path) -> Option<Backlight> {
    let rank = |path: &Path| match fs::read_to_string(path.join("type")).unwrap_or_default().trim() {
        "firmware" => 0,
        "platform" => 1,
        _ => 2,
    };
    let number = |path: &Path, file: &str| fs::read_to_string(path.join(file)).ok()?.trim().parse::<u32>().ok();
    let mut devices: Vec<_> = fs::read_dir(dir).ok()?.flatten().map(|entry| entry.path()).collect();
    devices.sort_by_key(|path| (rank(path), path.clone()));
    devices.iter().find_map(|path| {
        let max = number(path, "max_brightness").filter(|&max| max > 0)?;
        let value = number(path, "brightness")?;
        Some(Backlight { name: path.file_name()?.to_string_lossy().into_owned(), value, max })
    })
}

/// Brightness up (`up = true`) or down by [`BRIGHTNESS_STEP`]. logind allows this for the active
/// session; the sysfs file itself belongs to root.
pub fn change_brightness(up: bool) -> io::Result<()> {
    let Some(light) = backlight() else { return Ok(()) };
    run(Command::new("busctl").args([
        "call",
        "org.freedesktop.login1",
        "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session",
        "SetBrightness",
        "ssu",
        "backlight",
        &light.name,
        &light.stepped(up).to_string(),
    ]))
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

    fn light(value: u32, max: u32) -> Backlight {
        Backlight { name: "panel".into(), value, max }
    }

    #[test]
    fn brightness_steps_stay_in_range() {
        assert_eq!(light(500, 1000).percent(), 50);
        assert_eq!(light(500, 1000).stepped(true), 550);
        assert_eq!(light(500, 1000).stepped(false), 450);
        assert_eq!(light(980, 1000).stepped(true), 1000);
        assert_eq!(light(60, 1000).stepped(false), 50);
        assert_eq!(light(7, 7).percent(), 100);
    }

    #[test]
    fn prefers_firmware_backlights_and_skips_broken_ones() {
        let dir = std::env::temp_dir().join(format!("consoleui-backlight-{}", std::process::id()));
        let device = |name: &str, kind: &str, value: &str, max: &str| {
            let path = dir.join(name);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("type"), kind).unwrap();
            fs::write(path.join("brightness"), value).unwrap();
            fs::write(path.join("max_brightness"), max).unwrap();
        };
        device("a_raw", "raw\n", "1\n", "10\n");
        device("b_broken", "firmware\n", "1\n", "0\n");
        device("c_acpi", "firmware\n", "30\n", "60\n");
        let found = read_backlight(&dir);
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(found, Some(Backlight { name: "c_acpi".into(), value: 30, max: 60 }));
        assert_eq!(read_backlight(&dir), None);
    }
}
