//! Simulated Wi-Fi and Bluetooth backend (`--fake-network`), so the pages can be used and tested
//! without real hardware and without changing the system.

use std::time::Duration;

use crate::bluetooth::{Adapter, BluetoothState, BtDevice, DeviceKind, visible_devices};
use crate::i18n::{tr, trf};
use crate::network::{Backend, Request};
use crate::wifi::{WifiNetwork, WifiState, sort_networks};

pub struct FakeBackend {
    wifi_enabled: bool,
    networks: Vec<WifiNetwork>,
    scans: u32,
    powered: bool,
    discovering: bool,
    devices: Vec<BtDevice>,
    /// Simulated duration of connecting and pairing
    delay: Duration,
}

fn network(ssid: &str, strength: u8, secured: bool, known: bool, connected: bool) -> WifiNetwork {
    WifiNetwork { ssid: ssid.into(), strength, secured, connected, known }
}

fn device(address: &str, name: &str, kind: DeviceKind, paired: bool, connected: bool, rssi: Option<i16>) -> BtDevice {
    BtDevice {
        address: address.into(),
        name: if name.is_empty() { address.into() } else { name.into() },
        named: !name.is_empty(),
        kind,
        paired,
        trusted: paired,
        connected,
        rssi,
    }
}

/// These devices only show up during a search.
fn discoverable() -> Vec<BtDevice> {
    vec![
        device("98:B6:E9:12:34:56", "Xbox Wireless Controller", DeviceKind::Gamepad, false, false, Some(-52)),
        device("E4:17:D8:AA:BB:CC", "8BitDo Pro 2", DeviceKind::Gamepad, false, false, Some(-71)),
        device("B8:F6:53:01:02:03", "JBL Flip 5", DeviceKind::Audio, false, false, Some(-60)),
        device("5C:3A:45:9F:00:11", "", DeviceKind::Other, false, false, Some(-80)),
    ]
}

impl FakeBackend {
    pub fn new(delay: Duration) -> Self {
        Self {
            wifi_enabled: true,
            networks: vec![
                network("Zuhause", 82, true, true, true),
                network("Café Sonnenschein", 58, false, false, false),
                network("FRITZ!Box 7590 XY", 41, true, false, false),
                network("Nachbar-5G", 18, true, false, false),
            ],
            scans: 0,
            powered: true,
            discovering: false,
            devices: vec![
                device("A0:5A:5C:11:22:33", "DualSense Wireless Controller", DeviceKind::Gamepad, true, true, None),
                device("38:18:4C:44:55:66", "WH-1000XM4", DeviceKind::Audio, true, false, None),
            ],
            delay,
        }
    }

    fn wait(&self) {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
    }

    fn device_mut(&mut self, address: &str) -> Result<&mut BtDevice, String> {
        self.devices.iter_mut().find(|d| d.address == address).ok_or_else(|| tr("Device not found").to_string())
    }
}

impl Backend for FakeBackend {
    fn wifi(&mut self) -> Result<WifiState, String> {
        let mut networks = if self.wifi_enabled { self.networks.clone() } else { Vec::new() };
        sort_networks(&mut networks);
        Ok(WifiState { adapter: true, enabled: self.wifi_enabled, networks })
    }

    fn bluetooth(&mut self) -> Result<BluetoothState, String> {
        let adapter = Adapter {
            name: trf("{} (simulated)", &[&"hci0"]),
            address: "00:1A:7D:DA:71:13".into(),
            powered: self.powered,
            discovering: self.discovering,
        };
        let devices = if self.powered { visible_devices(self.devices.clone()) } else { Vec::new() };
        Ok(BluetoothState { adapter: Some(adapter), devices })
    }

    fn execute(&mut self, request: &Request) -> Result<(), String> {
        match request {
            Request::Refresh => {}
            Request::WifiEnable(on) => {
                self.wifi_enabled = *on;
                if !on {
                    self.networks.iter_mut().for_each(|n| n.connected = false);
                }
            }
            Request::WifiScan => {
                self.scans += 1;
                if self.scans == 1 {
                    self.networks.push(network("Gast-WLAN", 35, false, false, false));
                }
                // Signal strengths vary a bit
                for (i, n) in self.networks.iter_mut().enumerate() {
                    let delta = ((self.scans as usize + i) % 5) as i16 * 3 - 6;
                    n.strength = (n.strength as i16 + delta).clamp(5, 99) as u8;
                }
            }
            Request::WifiConnect { ssid, password } => {
                self.wait();
                let network = self.networks.iter().find(|n| &n.ssid == ssid).ok_or(tr("Network not found"))?;
                if network.secured && !network.known {
                    match password {
                        None => return Err(tr("Password required").into()),
                        Some(p) if p.chars().count() < 8 => return Err(tr("Wrong password").into()),
                        Some(_) => {}
                    }
                }
                for n in &mut self.networks {
                    n.connected = &n.ssid == ssid;
                    if n.connected {
                        n.known = true;
                    }
                }
            }
            Request::WifiDisconnect { ssid } => {
                self.networks.iter_mut().filter(|n| &n.ssid == ssid).for_each(|n| n.connected = false);
            }
            Request::BtPower(on) => {
                self.powered = *on;
                if !on {
                    self.discovering = false;
                    self.devices.iter_mut().for_each(|d| d.connected = false);
                }
            }
            Request::BtDiscovery(on) => {
                if !self.powered {
                    return Err(tr("Bluetooth is off").into());
                }
                self.discovering = *on;
                if *on {
                    for new in discoverable() {
                        if !self.devices.iter().any(|d| d.address == new.address) {
                            self.devices.push(new);
                        }
                    }
                } else {
                    self.devices.retain(|d| d.paired);
                }
            }
            Request::BtPair { address, .. } => {
                self.wait();
                let device = self.device_mut(address)?;
                // One device always fails, so the error case can be shown.
                if device.name.starts_with("8BitDo") {
                    return Err(tr("Authentication failed").into());
                }
                device.paired = true;
                device.trusted = true;
                device.connected = true;
            }
            Request::BtConnect { address, .. } => {
                self.wait();
                self.device_mut(address)?.connected = true;
            }
            Request::BtDisconnect { address, .. } => self.device_mut(address)?.connected = false,
            Request::BtRemove { address, .. } => {
                self.device_mut(address)?;
                self.devices.retain(|d| &d.address != address);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_connect_needs_password_for_unknown_networks() {
        let mut fake = FakeBackend::new(Duration::ZERO);
        let connect = |password: Option<&str>| Request::WifiConnect {
            ssid: "FRITZ!Box 7590 XY".into(),
            password: password.map(str::to_string),
        };
        assert!(fake.execute(&connect(None)).is_err());
        assert_eq!(fake.execute(&connect(Some("kurz"))), Err("Wrong password".into()));
        assert!(fake.execute(&connect(Some("lang genug"))).is_ok());
        let state = fake.wifi().unwrap();
        assert_eq!(state.connected().unwrap().ssid, "FRITZ!Box 7590 XY");
        assert_eq!(state.networks[0].ssid, "FRITZ!Box 7590 XY");
        assert!(!state.network("Zuhause").unwrap().connected);
    }

    #[test]
    fn bluetooth_discovery_and_pairing() {
        let mut fake = FakeBackend::new(Duration::ZERO);
        assert_eq!(fake.bluetooth().unwrap().devices.len(), 2);
        fake.execute(&Request::BtDiscovery(true)).unwrap();
        // The unnamed device stays hidden
        assert_eq!(fake.bluetooth().unwrap().devices.len(), 5);
        let xbox = Request::BtPair { address: "98:B6:E9:12:34:56".into(), name: "Xbox".into() };
        fake.execute(&xbox).unwrap();
        let state = fake.bluetooth().unwrap();
        let device = state.device("98:B6:E9:12:34:56").unwrap();
        assert!(device.paired && device.trusted && device.connected);
        let bitdo = Request::BtPair { address: "E4:17:D8:AA:BB:CC".into(), name: "8BitDo".into() };
        assert!(fake.execute(&bitdo).is_err());
        fake.execute(&Request::BtDiscovery(false)).unwrap();
        assert_eq!(fake.bluetooth().unwrap().devices.len(), 3);
        fake.execute(&Request::BtRemove { address: "38:18:4C:44:55:66".into(), name: String::new() }).unwrap();
        fake.execute(&Request::BtPower(false)).unwrap();
        assert_eq!(fake.bluetooth().unwrap().summary(), "Off");
    }
}
