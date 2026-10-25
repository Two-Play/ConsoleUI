//! Bluetooth model (Story 5.4): adapter and devices as BlueZ reports them, prepared for the
//! Bluetooth page. Without D-Bus; the query itself lives in the shell (`dbus.rs`).

use crate::i18n::{tr, trn};

/// Kind of device, mainly to highlight controllers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Gamepad,
    Audio,
    Keyboard,
    Mouse,
    Phone,
    Computer,
    Other,
}

impl DeviceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gamepad => tr("Controller"),
            Self::Audio => tr("Audio"),
            Self::Keyboard => tr("Keyboard"),
            Self::Mouse => tr("Mouse"),
            Self::Phone => tr("Phone"),
            Self::Computer => tr("Computer"),
            Self::Other => tr("Device"),
        }
    }

    /// Monochrome symbol as text (the shell shows controllers as a graphic)
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Gamepad => "◉",
            Self::Audio => "♪",
            Self::Keyboard => "⌨",
            Self::Mouse => "◎",
            Self::Phone => "☏",
            Self::Computer => "▭",
            Self::Other => "✱",
        }
    }
}

/// Known controller names, for when BlueZ reports neither `Icon` nor `Class` (e.g. BLE devices)
const GAMEPAD_NAMES: [&str; 8] =
    ["controller", "gamepad", "dualsense", "dualshock", "joy-con", "8bitdo", "xbox", "joystick"];

/// Kind of device from the BlueZ `Icon` (freedesktop icon name), Class of Device and name.
pub fn classify(icon: Option<&str>, class: Option<u32>, name: &str) -> DeviceKind {
    match icon {
        Some("input-gaming") => return DeviceKind::Gamepad,
        Some(i) if i.starts_with("audio-") => return DeviceKind::Audio,
        Some("input-keyboard") => return DeviceKind::Keyboard,
        Some("input-mouse" | "input-tablet") => return DeviceKind::Mouse,
        Some("phone") => return DeviceKind::Phone,
        Some("computer") => return DeviceKind::Computer,
        _ => {}
    }
    if let Some(class) = class {
        let major = (class >> 8) & 0x1f;
        let minor = (class >> 2) & 0x3f;
        match major {
            // Peripheral: bits 2–5 = 1 joystick, 2 gamepad; bit 6 keyboard, bit 7 pointing device
            0x05 if matches!(minor & 0x0f, 1 | 2) => return DeviceKind::Gamepad,
            0x05 if minor & 0x10 != 0 => return DeviceKind::Keyboard,
            0x05 if minor & 0x20 != 0 => return DeviceKind::Mouse,
            0x04 => return DeviceKind::Audio,
            0x02 => return DeviceKind::Phone,
            0x01 => return DeviceKind::Computer,
            _ => {}
        }
    }
    let lower = name.to_lowercase();
    if GAMEPAD_NAMES.iter().any(|n| lower.contains(n)) { DeviceKind::Gamepad } else { DeviceKind::Other }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtDevice {
    /// MAC address, e.g. `AA:BB:CC:DD:EE:FF`; used as the key
    pub address: String,
    /// Alias or name; without a name the address
    pub name: String,
    /// Did BlueZ report a real name? (Unnamed, unpaired devices are hidden.)
    pub named: bool,
    pub kind: DeviceKind,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub rssi: Option<i16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub address: String,
    pub powered: bool,
    pub discovering: bool,
}

/// What the Bluetooth page shows
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BluetoothState {
    /// `None`: no Bluetooth adapter
    pub adapter: Option<Adapter>,
    /// Filtered and sorted, see [`visible_devices`]
    pub devices: Vec<BtDevice>,
}

impl BluetoothState {
    pub fn device(&self, address: &str) -> Option<&BtDevice> {
        self.devices.iter().find(|d| d.address == address)
    }

    pub fn powered(&self) -> bool {
        self.adapter.as_ref().is_some_and(|a| a.powered)
    }

    pub fn discovering(&self) -> bool {
        self.adapter.as_ref().is_some_and(|a| a.discovering)
    }

    /// Short state for the "Bluetooth" row of the settings
    pub fn summary(&self) -> String {
        match &self.adapter {
            None => tr("No adapter").into(),
            Some(adapter) if !adapter.powered => tr("Off").into(),
            Some(_) => match self.devices.iter().filter(|d| d.connected).count() {
                0 => tr("On").into(),
                n => trn("{n} device connected", "{n} devices connected", n, &[]),
            },
        }
    }
}

/// Hides unpaired devices without a name (a search turns up many of them) and sorts: connected,
/// paired, controllers, signal strength, name.
pub fn visible_devices(mut devices: Vec<BtDevice>) -> Vec<BtDevice> {
    devices.retain(|d| d.paired || d.named);
    devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.paired.cmp(&a.paired))
            .then((b.kind == DeviceKind::Gamepad).cmp(&(a.kind == DeviceKind::Gamepad)))
            .then(b.rssi.unwrap_or(i16::MIN).cmp(&a.rssi.unwrap_or(i16::MIN)))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    devices
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, kind: DeviceKind, paired: bool, connected: bool, rssi: Option<i16>) -> BtDevice {
        BtDevice {
            address: format!("00:00:00:00:00:{:02X}", name.len()),
            name: name.into(),
            named: !name.is_empty(),
            kind,
            paired,
            trusted: paired,
            connected,
            rssi,
        }
    }

    #[test]
    fn classifies_by_icon_class_and_name() {
        assert_eq!(classify(Some("input-gaming"), None, "x"), DeviceKind::Gamepad);
        assert_eq!(classify(Some("audio-headset"), None, "x"), DeviceKind::Audio);
        // DualSense reports class 0x002508 (peripheral, gamepad)
        assert_eq!(classify(None, Some(0x002508), "x"), DeviceKind::Gamepad);
        assert_eq!(classify(None, Some(0x002540), "x"), DeviceKind::Keyboard);
        assert_eq!(classify(None, Some(0x240404), "x"), DeviceKind::Audio);
        assert_eq!(classify(None, None, "Xbox Wireless Controller"), DeviceKind::Gamepad);
        assert_eq!(classify(None, None, "Lautsprecher"), DeviceKind::Other);
    }

    #[test]
    fn hides_unnamed_and_sorts() {
        let devices = vec![
            device("Kopfhörer", DeviceKind::Audio, true, false, None),
            device("", DeviceKind::Other, false, false, Some(-40)),
            device("Fremder Controller", DeviceKind::Gamepad, false, false, Some(-70)),
            device("Lautsprecher", DeviceKind::Audio, false, false, Some(-50)),
            device("DualSense", DeviceKind::Gamepad, true, true, None),
            device("Alter Controller", DeviceKind::Gamepad, true, false, None),
        ];
        let names: Vec<_> = visible_devices(devices).into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["DualSense", "Alter Controller", "Kopfhörer", "Fremder Controller", "Lautsprecher"]);
    }

    #[test]
    fn summary() {
        let mut state = BluetoothState::default();
        assert_eq!(state.summary(), "No adapter");
        state.adapter =
            Some(Adapter { name: "hci0".into(), address: String::new(), powered: false, discovering: false });
        assert_eq!(state.summary(), "Off");
        state.adapter.as_mut().unwrap().powered = true;
        assert_eq!(state.summary(), "On");
        state.devices = vec![device("DualSense", DeviceKind::Gamepad, true, true, None)];
        assert_eq!(state.summary(), "1 device connected");
        state.devices.push(device("Xbox", DeviceKind::Gamepad, true, true, None));
        assert_eq!(state.summary(), "2 devices connected");
    }
}
