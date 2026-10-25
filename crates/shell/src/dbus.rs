//! Real Wi-Fi/Bluetooth backend (Stories 5.3/5.4): NetworkManager and BlueZ over the system bus
//! (zbus, blocking API). Runs only on the network thread (`consoleui_core::network`), never on the
//! UI thread. The processing (merging, sorting, device kind) lives in `consoleui_core::wifi` and
//! `::bluetooth`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use consoleui_core::bluetooth::{self, Adapter, BluetoothState, BtDevice};
use consoleui_core::i18n::{tr, trf};
use consoleui_core::network::{Backend, Request};
use consoleui_core::wifi::{self, AccessPoint, WifiState};
use serde::de::DeserializeOwned;
use zbus::blocking::Connection;
use zbus::zvariant::{DynamicType, ObjectPath, OwnedObjectPath, OwnedValue, Value};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const NM_WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const NM_AP: &str = "org.freedesktop.NetworkManager.AccessPoint";
const NM_SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const NM_SETTINGS: &str = "org.freedesktop.NetworkManager.Settings";
const NM_CONNECTION: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const NM_ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const NM_DEVICE_TYPE_WIFI: u32 = 2;

const BLUEZ: &str = "org.bluez";
const BLUEZ_ADAPTER: &str = "org.bluez.Adapter1";
const BLUEZ_DEVICE: &str = "org.bluez.Device1";
const AGENT_PATH: &str = "/org/consoleui/agent";

/// How long "Connect" waits for NetworkManager before giving up
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(30);

type Props = HashMap<String, OwnedValue>;
type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

/// Security type of a Wi-Fi network, for the profile on the first connect
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Security {
    Open,
    Wep,
    WpaPsk,
    Sae,
    Enterprise,
}

impl Security {
    fn from_flags(flags: u32, wpa: u32, rsn: u32) -> Self {
        const KEY_MGMT_PSK: u32 = 0x100;
        const KEY_MGMT_8021X: u32 = 0x200;
        const KEY_MGMT_SAE: u32 = 0x400;
        let mgmt = wpa | rsn;
        if mgmt & KEY_MGMT_8021X != 0 {
            Self::Enterprise
        } else if mgmt & KEY_MGMT_PSK != 0 {
            Self::WpaPsk
        } else if mgmt & KEY_MGMT_SAE != 0 {
            Self::Sae
        } else if wifi::is_secured(flags, wpa, rsn) {
            Self::Wep
        } else {
            Self::Open
        }
    }
}

#[derive(Default)]
pub struct DbusBackend {
    connection: Option<Connection>,
    /// From the last read: Wi-Fi device, access point and saved profile per SSID
    wifi_device: Option<OwnedObjectPath>,
    access_points: HashMap<String, (OwnedObjectPath, Security)>,
    profiles: HashMap<String, OwnedObjectPath>,
    /// Bluetooth adapter and devices per address
    adapter: Option<OwnedObjectPath>,
    devices: HashMap<String, OwnedObjectPath>,
    agent_registered: bool,
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Short, readable message from a D-Bus error (`org.bluez.Error.AuthenticationFailed` → "AuthenticationFailed")
fn short(error: zbus::Error) -> String {
    match &error {
        zbus::Error::MethodError(name, message, _) => {
            let name = name.as_str().rsplit('.').next().unwrap_or_default().to_string();
            match message {
                Some(message) if !message.is_empty() => format!("{name}: {message}"),
                _ => name,
            }
        }
        _ => error.to_string(),
    }
}

fn get<T: TryFrom<OwnedValue>>(props: &Props, key: &str) -> Option<T> {
    T::try_from(props.get(key)?.try_clone().ok()?).ok()
}

impl DbusBackend {
    fn bus(&mut self) -> Result<&Connection, String> {
        if self.connection.is_none() {
            self.connection = Some(Connection::system().map_err(|e| trf("Can't reach the system bus: {}", &[&e]))?);
        }
        Ok(self.connection.as_ref().expect("just set"))
    }

    fn call<B, R>(&mut self, dest: &str, path: &str, iface: &str, method: &str, body: &B) -> Result<R, String>
    where
        B: serde::Serialize + DynamicType,
        R: DeserializeOwned + zbus::zvariant::Type,
    {
        let reply = self.bus()?.call_method(Some(dest), path, Some(iface), method, body).map_err(short)?;
        reply.body().deserialize::<R>().map_err(err)
    }

    fn call_unit<B>(&mut self, dest: &str, path: &str, iface: &str, method: &str, body: &B) -> Result<(), String>
    where
        B: serde::Serialize + DynamicType,
    {
        self.bus()?.call_method(Some(dest), path, Some(iface), method, body).map_err(short)?;
        Ok(())
    }

    fn get_all(&mut self, dest: &str, path: &str, iface: &str) -> Result<Props, String> {
        self.call(dest, path, "org.freedesktop.DBus.Properties", "GetAll", &(iface,))
    }

    fn set(&mut self, dest: &str, path: &str, iface: &str, name: &str, value: Value<'_>) -> Result<(), String> {
        self.call_unit(dest, path, "org.freedesktop.DBus.Properties", "Set", &(iface, name, value))
    }

    // --- NetworkManager --------------------------------------------------------------------

    /// SSIDs and paths of the saved Wi-Fi profiles
    fn read_profiles(&mut self) -> HashMap<String, OwnedObjectPath> {
        let paths: Vec<OwnedObjectPath> =
            self.call(NM, NM_SETTINGS_PATH, NM_SETTINGS, "ListConnections", &()).unwrap_or_default();
        let mut profiles = HashMap::new();
        for path in paths {
            let Ok(settings) =
                self.call::<_, HashMap<String, Props>>(NM, path.as_str(), NM_CONNECTION, "GetSettings", &())
            else {
                continue;
            };
            let ssid = settings.get("802-11-wireless").and_then(|w| get::<Vec<u8>>(w, "ssid"));
            if let Some(ssid) = ssid {
                profiles.insert(wifi::ssid_from_bytes(&ssid), path);
            }
        }
        profiles
    }

    fn read_wifi(&mut self) -> Result<WifiState, String> {
        let nm = self.get_all(NM, NM_PATH, NM)?;
        let enabled = get::<bool>(&nm, "WirelessEnabled").unwrap_or(false)
            && get::<bool>(&nm, "WirelessHardwareEnabled").unwrap_or(true);
        let devices: Vec<OwnedObjectPath> = self.call(NM, NM_PATH, NM, "GetDevices", &())?;
        self.wifi_device = None;
        for device in devices {
            let props = self.get_all(NM, device.as_str(), NM_DEVICE)?;
            if get::<u32>(&props, "DeviceType") == Some(NM_DEVICE_TYPE_WIFI) {
                self.wifi_device = Some(device);
                break;
            }
        }
        let Some(device) = self.wifi_device.clone() else {
            return Ok(WifiState { adapter: false, enabled, networks: Vec::new() });
        };
        let wireless = self.get_all(NM, device.as_str(), NM_WIRELESS)?;
        let active = get::<OwnedObjectPath>(&wireless, "ActiveAccessPoint").filter(|p| p.as_str() != "/");
        let paths: Vec<OwnedObjectPath> = self.call(NM, device.as_str(), NM_WIRELESS, "GetAllAccessPoints", &())?;
        let mut points = Vec::new();
        let mut connected = None;
        let mut best: HashMap<String, u8> = HashMap::new();
        self.access_points.clear();
        for path in paths {
            // Access points sometimes disappear between two calls
            let Ok(props) = self.get_all(NM, path.as_str(), NM_AP) else { continue };
            let ssid = wifi::ssid_from_bytes(&get::<Vec<u8>>(&props, "Ssid").unwrap_or_default());
            let (flags, wpa, rsn) = (
                get::<u32>(&props, "Flags").unwrap_or(0),
                get::<u32>(&props, "WpaFlags").unwrap_or(0),
                get::<u32>(&props, "RsnFlags").unwrap_or(0),
            );
            let strength = get::<u8>(&props, "Strength").unwrap_or(0);
            if active.as_ref() == Some(&path) {
                connected = Some(ssid.clone());
            }
            // Remember the strongest access point per SSID for connecting
            if best.get(&ssid).is_none_or(|&s| strength > s) {
                best.insert(ssid.clone(), strength);
                self.access_points.insert(ssid.clone(), (path.clone(), Security::from_flags(flags, wpa, rsn)));
            }
            points.push(AccessPoint { ssid, strength, secured: wifi::is_secured(flags, wpa, rsn) });
        }
        self.profiles = self.read_profiles();
        let known: Vec<String> = self.profiles.keys().cloned().collect();
        let networks = if enabled { wifi::build_networks(&points, connected.as_deref(), &known) } else { Vec::new() };
        Ok(WifiState { adapter: true, enabled, networks })
    }

    fn wifi_device(&self) -> Result<OwnedObjectPath, String> {
        self.wifi_device.clone().ok_or_else(|| tr("No Wi-Fi adapter").to_string())
    }

    fn connect_wifi(&mut self, ssid: &str, password: Option<&str>) -> Result<(), String> {
        let device = self.wifi_device()?;
        let root = ObjectPath::from_static_str_unchecked("/");
        let (active, created) = match (self.profiles.get(ssid).cloned(), password) {
            // Saved profile and no new password: just activate it
            (Some(profile), None) => {
                let active: OwnedObjectPath =
                    self.call(NM, NM_PATH, NM, "ActivateConnection", &(&profile, &device, &root))?;
                (active, None)
            }
            (existing, _) => {
                let (ap, security) =
                    self.access_points.get(ssid).cloned().ok_or_else(|| tr("Network no longer visible").to_string())?;
                let mut settings: HashMap<&str, HashMap<&str, Value<'_>>> = HashMap::new();
                settings.insert(
                    "connection",
                    HashMap::from([("id", Value::from(ssid)), ("type", Value::from("802-11-wireless"))]),
                );
                settings.insert("802-11-wireless", HashMap::from([("ssid", Value::from(ssid.as_bytes().to_vec()))]));
                let key_mgmt = match security {
                    Security::Open => None,
                    Security::WpaPsk => Some("wpa-psk"),
                    Security::Sae => Some("sae"),
                    Security::Wep => return Err(tr("WEP is not supported").into()),
                    Security::Enterprise => return Err(tr("WPA Enterprise is not supported").into()),
                };
                if let Some(key_mgmt) = key_mgmt {
                    let psk = password.ok_or_else(|| tr("Password required").to_string())?;
                    settings.insert(
                        "802-11-wireless-security",
                        HashMap::from([("key-mgmt", Value::from(key_mgmt)), ("psk", Value::from(psk))]),
                    );
                }
                // Replace an old profile with a wrong password
                if let Some(old) = existing {
                    let _ = self.call_unit(NM, old.as_str(), NM_CONNECTION, "Delete", &());
                }
                let (profile, active): (OwnedObjectPath, OwnedObjectPath) =
                    self.call(NM, NM_PATH, NM, "AddAndActivateConnection", &(settings, &device, &ap))?;
                (active, Some(profile))
            }
        };
        let result = self.wait_activated(&active);
        // Delete a newly created profile again if it does not work (wrong password)
        if let (Err(_), Some(profile)) = (&result, created) {
            let _ = self.call_unit(NM, profile.as_str(), NM_CONNECTION, "Delete", &());
        }
        result
    }

    /// Waits until the connection is up (state 2) or fails (state 4 or object gone).
    fn wait_activated(&mut self, active: &OwnedObjectPath) -> Result<(), String> {
        let started = Instant::now();
        while started.elapsed() < ACTIVATION_TIMEOUT {
            match self.get_all(NM, active.as_str(), NM_ACTIVE).ok().and_then(|p| get::<u32>(&p, "State")) {
                Some(2) => return Ok(()),
                Some(3 | 4) | None => return Err(tr("Connection refused (wrong password?)").into()),
                _ => std::thread::sleep(Duration::from_millis(300)),
            }
        }
        Err(tr("Timed out").into())
    }

    // --- BlueZ -----------------------------------------------------------------------------

    fn read_bluetooth(&mut self) -> Result<BluetoothState, String> {
        let objects: ManagedObjects = self
            .call(BLUEZ, "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", &())
            .map_err(|e| if e.contains("ServiceUnknown") { tr("BlueZ is not running").into() } else { e })?;
        let mut adapters: Vec<(&OwnedObjectPath, &Props)> =
            objects.iter().filter_map(|(path, ifaces)| Some((path, ifaces.get(BLUEZ_ADAPTER)?))).collect();
        adapters.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        self.devices.clear();
        let Some((adapter_path, props)) = adapters.first() else {
            self.adapter = None;
            return Ok(BluetoothState::default());
        };
        self.adapter = Some((*adapter_path).clone());
        let adapter = Adapter {
            name: get::<String>(props, "Alias").or_else(|| get(props, "Name")).unwrap_or_default(),
            address: get::<String>(props, "Address").unwrap_or_default(),
            powered: get::<bool>(props, "Powered").unwrap_or(false),
            discovering: get::<bool>(props, "Discovering").unwrap_or(false),
        };
        let mut devices = Vec::new();
        for (path, ifaces) in &objects {
            let Some(props) = ifaces.get(BLUEZ_DEVICE) else { continue };
            if get::<OwnedObjectPath>(props, "Adapter").as_ref() != Some(*adapter_path) {
                continue;
            }
            let address = get::<String>(props, "Address").unwrap_or_default();
            let real_name = get::<String>(props, "Name");
            let name = get::<String>(props, "Alias").or_else(|| real_name.clone()).unwrap_or_else(|| address.clone());
            let icon = get::<String>(props, "Icon");
            let class = get::<u32>(props, "Class");
            self.devices.insert(address.clone(), path.clone());
            devices.push(BtDevice {
                kind: bluetooth::classify(icon.as_deref(), class, &name),
                address,
                name,
                named: real_name.is_some(),
                paired: get(props, "Paired").unwrap_or(false),
                trusted: get(props, "Trusted").unwrap_or(false),
                connected: get(props, "Connected").unwrap_or(false),
                rssi: get(props, "RSSI"),
            });
        }
        Ok(BluetoothState { adapter: Some(adapter), devices: bluetooth::visible_devices(devices) })
    }

    fn adapter(&self) -> Result<String, String> {
        self.adapter.as_ref().map(|p| p.to_string()).ok_or_else(|| tr("No Bluetooth adapter").to_string())
    }

    fn device(&self, address: &str) -> Result<String, String> {
        self.devices.get(address).map(|p| p.to_string()).ok_or_else(|| tr("Device is gone").to_string())
    }

    /// Own pairing agent without input/output (controllers pair via "Just Works"). BlueZ asks the
    /// agent of the connection that calls `Pair`; without it pairing fails when no other agent
    /// (e.g. KDE's) runs in the session.
    fn register_agent(&mut self) -> Result<(), String> {
        if self.agent_registered {
            return Ok(());
        }
        self.bus()?.object_server().at(AGENT_PATH, Agent).map_err(err)?;
        let path = ObjectPath::from_static_str_unchecked(AGENT_PATH);
        self.call_unit(BLUEZ, "/org/bluez", "org.bluez.AgentManager1", "RegisterAgent", &(path, "NoInputNoOutput"))?;
        self.agent_registered = true;
        Ok(())
    }

    fn pair(&mut self, address: &str) -> Result<(), String> {
        let path = self.device(address)?;
        if let Err(err) = self.register_agent() {
            eprintln!("Cannot register the Bluetooth agent: {err}");
        }
        match self.call_unit(BLUEZ, &path, BLUEZ_DEVICE, "Pair", &()) {
            Err(e) if !e.starts_with("AlreadyExists") => return Err(e),
            _ => {}
        }
        // Trust it, so the controller may reconnect by itself later
        self.set(BLUEZ, &path, BLUEZ_DEVICE, "Trusted", Value::from(true))?;
        self.call_unit(BLUEZ, &path, BLUEZ_DEVICE, "Connect", &())
    }
}

/// `org.bluez.Agent1`: accepts every request (only asked during a pairing started by the
/// user).
struct Agent;

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {}

    fn request_pin_code(&self, _device: OwnedObjectPath) -> String {
        // Old controllers without Secure Simple Pairing
        "0000".into()
    }

    fn display_pin_code(&self, _device: OwnedObjectPath, _pincode: String) {}

    fn request_passkey(&self, _device: OwnedObjectPath) -> u32 {
        0
    }

    fn display_passkey(&self, _device: OwnedObjectPath, _passkey: u32, _entered: u16) {}

    fn request_confirmation(&self, _device: OwnedObjectPath, _passkey: u32) {}

    fn request_authorization(&self, _device: OwnedObjectPath) {}

    fn authorize_service(&self, _device: OwnedObjectPath, _uuid: String) {}

    fn cancel(&self) {}
}

impl Backend for DbusBackend {
    fn wifi(&mut self) -> Result<WifiState, String> {
        self.read_wifi()
    }

    fn bluetooth(&mut self) -> Result<BluetoothState, String> {
        self.read_bluetooth()
    }

    fn execute(&mut self, request: &Request) -> Result<(), String> {
        match request {
            Request::Refresh => Ok(()),
            Request::WifiEnable(on) => self.set(NM, NM_PATH, NM, "WirelessEnabled", Value::from(*on)),
            Request::WifiScan => {
                let device = self.wifi_device()?;
                let options: HashMap<&str, Value<'_>> = HashMap::new();
                self.call_unit(NM, device.as_str(), NM_WIRELESS, "RequestScan", &(options,))
            }
            Request::WifiConnect { ssid, password } => self.connect_wifi(ssid, password.as_deref()),
            Request::WifiDisconnect { .. } => {
                let device = self.wifi_device()?;
                self.call_unit(NM, device.as_str(), NM_DEVICE, "Disconnect", &())
            }
            Request::BtPower(on) => {
                let adapter = self.adapter()?;
                self.set(BLUEZ, &adapter, BLUEZ_ADAPTER, "Powered", Value::from(*on))
            }
            Request::BtDiscovery(on) => {
                let adapter = self.adapter()?;
                self.call_unit(
                    BLUEZ,
                    &adapter,
                    BLUEZ_ADAPTER,
                    if *on { "StartDiscovery" } else { "StopDiscovery" },
                    &(),
                )
            }
            Request::BtPair { address, .. } => self.pair(address),
            Request::BtConnect { address, .. } => {
                let path = self.device(address)?;
                self.call_unit(BLUEZ, &path, BLUEZ_DEVICE, "Connect", &())
            }
            Request::BtDisconnect { address, .. } => {
                let path = self.device(address)?;
                self.call_unit(BLUEZ, &path, BLUEZ_DEVICE, "Disconnect", &())
            }
            Request::BtRemove { address, .. } => {
                let (adapter, path) = (self.adapter()?, self.device(address)?);
                let path = ObjectPath::try_from(path).map_err(err)?;
                self.call_unit(BLUEZ, &adapter, BLUEZ_ADAPTER, "RemoveDevice", &(path,))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_from_nm_flags() {
        assert_eq!(Security::from_flags(0, 0, 0), Security::Open);
        assert_eq!(Security::from_flags(1, 0, 0), Security::Wep);
        assert_eq!(Security::from_flags(1, 0, 0x188), Security::WpaPsk);
        assert_eq!(Security::from_flags(1, 0, 0x488), Security::Sae);
        assert_eq!(Security::from_flags(1, 0, 0x288), Security::Enterprise);
    }
}
