//! System status for the top bar (battery, network) straight from sysfs/procfs.

use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    Offline,
    Ethernet,
    /// Signal strength in percent, if known
    Wifi(Option<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemStatus {
    pub battery: Option<Battery>,
    pub network: Network,
}

pub fn read() -> SystemStatus {
    let wireless = fs::read_to_string("/proc/net/wireless").unwrap_or_default();
    read_from(Path::new("/sys/class"), &wireless)
}

/// `class_dir` corresponds to `/sys/class`, `proc_wireless` to the content of `/proc/net/wireless`.
pub fn read_from(class_dir: &Path, proc_wireless: &str) -> SystemStatus {
    SystemStatus {
        battery: battery(&class_dir.join("power_supply")),
        network: network(&class_dir.join("net"), proc_wireless),
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn battery(dir: &Path) -> Option<Battery> {
    let mut supplies: Vec<_> = fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    supplies.sort();
    supplies.iter().find_map(|supply| {
        if read_trimmed(&supply.join("type"))? != "Battery" {
            return None;
        }
        // Batteries of mice or controllers report `scope=Device`.
        if read_trimmed(&supply.join("scope")).is_some_and(|s| s == "Device") {
            return None;
        }
        Some(Battery {
            percent: read_trimmed(&supply.join("capacity"))?.parse::<u8>().ok()?.min(100),
            charging: read_trimmed(&supply.join("status")).is_some_and(|s| s == "Charging" || s == "Full"),
        })
    })
}

fn network(dir: &Path, proc_wireless: &str) -> Network {
    let mut wired = false;
    for iface in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = iface.path();
        // Only real hardware: virtual devices (lo, bridges, VPN, Docker) have no `device`.
        if !path.join("device").exists() || read_trimmed(&path.join("operstate")).as_deref() != Some("up") {
            continue;
        }
        if path.join("wireless").exists() {
            let name = iface.file_name();
            return Network::Wifi(name.to_str().and_then(|n| wifi_signal(proc_wireless, n)));
        }
        wired = true;
    }
    if wired { Network::Ethernet } else { Network::Offline }
}

/// Link quality from `/proc/net/wireless` (scale 0–70) in percent.
fn wifi_signal(proc_wireless: &str, iface: &str) -> Option<u8> {
    let line = proc_wireless.lines().find(|l| l.trim_start().starts_with(&format!("{iface}:")))?;
    let quality: f32 = line.split_whitespace().nth(2)?.trim_end_matches('.').parse().ok()?;
    Some(((quality / 70.0) * 100.0).clamp(0.0, 100.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempClass(std::path::PathBuf);

    impl TempClass {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("consoleui-status-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("power_supply")).unwrap();
            fs::create_dir_all(dir.join("net")).unwrap();
            Self(dir)
        }

        fn file(&self, rel: &str, content: &str) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
    }

    impl Drop for TempClass {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const WIRELESS: &str = "\
Inter-| sta-|   Quality        |   Discarded packets
 face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22
wlan0: 0000   56.  -54.  -256        0      0      0      0     12        0
";

    #[test]
    fn desktop_without_battery_on_ethernet() {
        let class = TempClass::new("desktop");
        class.file("net/enp3s0/operstate", "up\n");
        class.file("net/enp3s0/device/vendor", "x");
        class.file("net/lo/operstate", "unknown\n");
        class.file("net/docker0/operstate", "up\n");
        assert_eq!(read_from(&class.0, ""), SystemStatus { battery: None, network: Network::Ethernet });
    }

    #[test]
    fn laptop_on_wifi_ignores_device_batteries() {
        let class = TempClass::new("laptop");
        class.file("power_supply/BAT0/type", "Battery\n");
        class.file("power_supply/BAT0/capacity", "81\n");
        class.file("power_supply/BAT0/status", "Charging\n");
        class.file("power_supply/hidpp_battery_0/type", "Battery\n");
        class.file("power_supply/hidpp_battery_0/scope", "Device\n");
        class.file("power_supply/hidpp_battery_0/capacity", "5\n");
        class.file("net/wlan0/operstate", "up\n");
        class.file("net/wlan0/device/vendor", "x");
        fs::create_dir_all(class.0.join("net/wlan0/wireless")).unwrap();
        assert_eq!(
            read_from(&class.0, WIRELESS),
            SystemStatus { battery: Some(Battery { percent: 81, charging: true }), network: Network::Wifi(Some(80)) }
        );
    }

    #[test]
    fn offline_when_nothing_is_up() {
        let class = TempClass::new("offline");
        class.file("net/eno1/operstate", "down\n");
        class.file("net/eno1/device/vendor", "x");
        assert_eq!(read_from(&class.0, "").network, Network::Offline);
    }
}
