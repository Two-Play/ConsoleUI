//! Wi-Fi model (Story 5.3): networks as NetworkManager reports them, prepared for the Wi-Fi page.
//! Without D-Bus; the query itself lives in the shell (`dbus.rs`).

use crate::i18n::tr;

/// A visible Wi-Fi network (merged per SSID)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiNetwork {
    pub ssid: String,
    /// Signal strength 0–100 %
    pub strength: u8,
    /// Needs a password (WEP/WPA/WPA2/WPA3)
    pub secured: bool,
    pub connected: bool,
    /// There is a saved profile already (connect without entering a password)
    pub known: bool,
}

/// What the Wi-Fi page shows
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WifiState {
    /// Is there a Wi-Fi adapter at all?
    pub adapter: bool,
    /// Wi-Fi switched on (software and hardware)
    pub enabled: bool,
    /// Sorted, see [`sort_networks`]
    pub networks: Vec<WifiNetwork>,
}

impl WifiState {
    pub fn connected(&self) -> Option<&WifiNetwork> {
        self.networks.iter().find(|n| n.connected)
    }

    pub fn network(&self, ssid: &str) -> Option<&WifiNetwork> {
        self.networks.iter().find(|n| n.ssid == ssid)
    }

    /// Short state for the "Wi-Fi" row of the settings
    pub fn summary(&self) -> String {
        if !self.adapter {
            tr("No adapter").into()
        } else if !self.enabled {
            tr("Off").into()
        } else if let Some(network) = self.connected() {
            network.ssid.clone()
        } else {
            tr("Not connected").into()
        }
    }
}

/// NetworkManager flags of an access point: secured as soon as WPA/RSN flags are set or the
/// privacy flag (WEP) is on.
pub fn is_secured(flags: u32, wpa_flags: u32, rsn_flags: u32) -> bool {
    const NM_802_11_AP_FLAGS_PRIVACY: u32 = 0x1;
    flags & NM_802_11_AP_FLAGS_PRIVACY != 0 || wpa_flags != 0 || rsn_flags != 0
}

/// SSIDs are bytes; invalid UTF-8 is replaced, control characters are dropped.
pub fn ssid_from_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).chars().filter(|c| !c.is_control()).collect()
}

/// An access point as NetworkManager reports it (a network can have several)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessPoint {
    pub ssid: String,
    pub strength: u8,
    pub secured: bool,
}

/// Merges access points into networks (the strongest wins), leaves out hidden networks without
/// an SSID, marks the connected and the saved networks and sorts.
pub fn build_networks(points: &[AccessPoint], connected: Option<&str>, known: &[String]) -> Vec<WifiNetwork> {
    let mut networks: Vec<WifiNetwork> = Vec::new();
    for point in points.iter().filter(|p| !p.ssid.trim().is_empty()) {
        match networks.iter_mut().find(|n| n.ssid == point.ssid) {
            Some(network) => {
                network.strength = network.strength.max(point.strength);
                network.secured |= point.secured;
            }
            None => networks.push(WifiNetwork {
                ssid: point.ssid.clone(),
                strength: point.strength.min(100),
                secured: point.secured,
                connected: connected == Some(point.ssid.as_str()),
                known: known.contains(&point.ssid),
            }),
        }
    }
    sort_networks(&mut networks);
    networks
}

/// Connected network first, then by signal strength, on a tie alphabetical.
pub fn sort_networks(networks: &mut [WifiNetwork]) {
    networks.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.strength.cmp(&a.strength))
            .then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
    });
}

/// Bars for the display (0–4)
pub fn signal_bars(strength: u8) -> u8 {
    match strength {
        0..=19 => 1,
        20..=44 => 2,
        45..=69 => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ap(ssid: &str, strength: u8, secured: bool) -> AccessPoint {
        AccessPoint { ssid: ssid.into(), strength, secured }
    }

    #[test]
    fn secured_flags() {
        assert!(!is_secured(0, 0, 0));
        assert!(is_secured(1, 0, 0));
        assert!(is_secured(0, 0x100, 0));
        assert!(is_secured(0, 0, 0x188));
    }

    #[test]
    fn ssid_bytes_are_cleaned() {
        assert_eq!(ssid_from_bytes(b"Caf\xc3\xa9"), "Café");
        assert_eq!(ssid_from_bytes(b"a\x00b\xff"), "ab\u{fffd}");
    }

    #[test]
    fn networks_are_merged_marked_and_sorted() {
        let points = [
            ap("Nachbar", 30, true),
            ap("Zuhause", 40, true),
            ap("", 99, false),
            ap("Café", 70, false),
            ap("Zuhause", 80, true),
            ap("anders", 30, true),
        ];
        let networks = build_networks(&points, Some("Nachbar"), &["Zuhause".into()]);
        let names: Vec<_> = networks.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(names, ["Nachbar", "Zuhause", "Café", "anders"]);
        assert!(networks[0].connected);
        assert_eq!(networks[1].strength, 80);
        assert!(networks[1].known && !networks[2].known);
    }

    #[test]
    fn summary_and_bars() {
        let mut state = WifiState::default();
        assert_eq!(state.summary(), "No adapter");
        state.adapter = true;
        assert_eq!(state.summary(), "Off");
        state.enabled = true;
        assert_eq!(state.summary(), "Not connected");
        state.networks = build_networks(&[ap("Zuhause", 50, true)], Some("Zuhause"), &[]);
        assert_eq!(state.summary(), "Zuhause");
        assert_eq!([signal_bars(5), signal_bars(30), signal_bars(50), signal_bars(90)], [1, 2, 3, 4]);
    }
}
