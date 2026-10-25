//! WLAN-Modell (Story 5.3): Netzwerke, wie NetworkManager sie meldet, aufbereitet für die
//! WLAN-Seite. Ohne D-Bus; die Abfrage selbst steckt in der Shell (`dbus.rs`).

/// Ein sichtbares WLAN (pro SSID zusammengefasst)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiNetwork {
    pub ssid: String,
    /// Signalstärke 0–100 %
    pub strength: u8,
    /// Braucht ein Passwort (WEP/WPA/WPA2/WPA3)
    pub secured: bool,
    pub connected: bool,
    /// Es gibt schon ein gespeichertes Profil (Verbinden ohne Passwort-Eingabe)
    pub known: bool,
}

/// Was die WLAN-Seite anzeigt
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WifiState {
    /// Gibt es überhaupt einen WLAN-Adapter?
    pub adapter: bool,
    /// WLAN eingeschaltet (Software und Hardware)
    pub enabled: bool,
    /// Sortiert, siehe [`sort_networks`]
    pub networks: Vec<WifiNetwork>,
}

impl WifiState {
    pub fn connected(&self) -> Option<&WifiNetwork> {
        self.networks.iter().find(|n| n.connected)
    }

    pub fn network(&self, ssid: &str) -> Option<&WifiNetwork> {
        self.networks.iter().find(|n| n.ssid == ssid)
    }

    /// Kurzer Zustand für die Zeile „WLAN“ in den Einstellungen
    pub fn summary(&self) -> String {
        if !self.adapter {
            "Kein Adapter".into()
        } else if !self.enabled {
            "Aus".into()
        } else if let Some(network) = self.connected() {
            network.ssid.clone()
        } else {
            "Nicht verbunden".into()
        }
    }
}

/// NetworkManager-Flags eines Access Points: gesichert, sobald WPA/RSN-Flags gesetzt sind oder
/// das Privacy-Flag (WEP) an ist.
pub fn is_secured(flags: u32, wpa_flags: u32, rsn_flags: u32) -> bool {
    const NM_802_11_AP_FLAGS_PRIVACY: u32 = 0x1;
    flags & NM_802_11_AP_FLAGS_PRIVACY != 0 || wpa_flags != 0 || rsn_flags != 0
}

/// SSIDs sind Bytes; ungültiges UTF-8 wird ersetzt, Steuerzeichen fallen weg.
pub fn ssid_from_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).chars().filter(|c| !c.is_control()).collect()
}

/// Ein Access Point, wie ihn NetworkManager liefert (ein Netz kann mehrere haben)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessPoint {
    pub ssid: String,
    pub strength: u8,
    pub secured: bool,
}

/// Fasst Access Points zu Netzen zusammen (stärkster gewinnt), lässt versteckte Netze ohne SSID
/// weg, markiert das verbundene und die gespeicherten Netze und sortiert.
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

/// Verbundenes Netz zuerst, dann nach Signalstärke, bei Gleichstand alphabetisch.
pub fn sort_networks(networks: &mut [WifiNetwork]) {
    networks.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.strength.cmp(&a.strength))
            .then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
    });
}

/// Balken für die Anzeige (0–4)
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
        assert_eq!(state.summary(), "Kein Adapter");
        state.adapter = true;
        assert_eq!(state.summary(), "Aus");
        state.enabled = true;
        assert_eq!(state.summary(), "Nicht verbunden");
        state.networks = build_networks(&[ap("Zuhause", 50, true)], Some("Zuhause"), &[]);
        assert_eq!(state.summary(), "Zuhause");
        assert_eq!([signal_bars(5), signal_bars(30), signal_bars(50), signal_bars(90)], [1, 2, 3, 4]);
    }
}
