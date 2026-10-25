//! State and input of the "Wi-Fi" and "Bluetooth" sub-pages of the settings (Stories 5.3/5.4):
//! focus, password entry with the on-screen keyboard, confirmation before removing, running
//! requests and the display as plain rows. Without UI and without D-Bus; the backend thread runs
//! the requests ([`crate::network`]).

use std::time::{Duration, Instant};

use crate::bluetooth::{BluetoothState, BtDevice, DeviceKind};
use crate::i18n::{tr, trf};
use crate::nav::Action;
use crate::network::{Notice, Request, Response};
use crate::osk::{KeyboardEvent, OnScreenKeyboard};
use crate::wifi::{WifiNetwork, WifiState, signal_bars};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Wifi,
    Bluetooth,
}

/// How often the state is read again while the settings are visible
const REFRESH: Duration = Duration::from_secs(3);
/// … and during a search or a running request
const REFRESH_FAST: Duration = Duration::from_secs(1);

/// A row of the page
#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    /// Wi-Fi or Bluetooth on/off
    Toggle,
    Network(String),
    Device(String),
}

struct Password {
    ssid: String,
    text: String,
    keyboard: OnScreenKeyboard,
    visible: bool,
}

/// Display of a row
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowView {
    pub symbol: String,
    pub title: String,
    pub detail: String,
    /// On the right of the row, e.g. "On"
    pub value: String,
    /// Wi-Fi bars 1–4, otherwise 0
    pub bars: u8,
    pub secured: bool,
    pub gamepad: bool,
    /// Connected
    pub active: bool,
    pub busy: bool,
    /// Waits for the second press to remove
    pub armed: bool,
}

/// Display of the password entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordView {
    pub ssid: String,
    /// Masked unless "show" is on
    pub text: String,
    pub visible: bool,
    pub labels: Vec<String>,
    pub key_focused: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageView {
    pub title: String,
    pub status: String,
    pub rows: Vec<RowView>,
    pub focused: usize,
    /// Text below the rows when there is nothing to list
    pub empty_title: String,
    pub empty_text: String,
}

#[derive(Default)]
pub struct NetPages {
    page: Option<Page>,
    focus: usize,
    /// `None`: not read yet
    wifi: Option<Result<WifiState, String>>,
    bluetooth: Option<Result<BluetoothState, String>>,
    /// Running requests (without `Refresh`)
    busy: Vec<Request>,
    password: Option<Password>,
    /// Address of the device that the next press removes
    armed: Option<String>,
    last_refresh: Option<Instant>,
}

impl NetPages {
    pub fn page(&self) -> Option<Page> {
        self.page
    }

    pub fn is_open(&self) -> bool {
        self.page.is_some()
    }

    pub fn password_open(&self) -> bool {
        self.password.is_some()
    }

    /// Opens a sub-page; the state is read again right away.
    pub fn open(&mut self, page: Page) -> Vec<Request> {
        self.page = Some(page);
        self.focus = 0;
        self.armed = None;
        self.password = None;
        self.last_refresh = None;
        vec![Request::Refresh]
    }

    /// Short state for the rows of the settings page
    pub fn summary(&self, page: Page) -> String {
        let summary = match page {
            Page::Wifi => self.wifi.as_ref().map(|s| s.as_ref().map(WifiState::summary)),
            Page::Bluetooth => self.bluetooth.as_ref().map(|s| s.as_ref().map(BluetoothState::summary)),
        };
        match summary {
            None => "…".into(),
            Some(Ok(text)) => text,
            Some(Err(_)) => tr("Unavailable").into(),
        }
    }

    /// Whether to read the state again now (`visible`: the settings are visible).
    pub fn refresh_due(&mut self, now: Instant, visible: bool) -> bool {
        if !visible {
            return false;
        }
        let discovering = self.bt_state().is_some_and(BluetoothState::discovering);
        let interval = if !self.busy.is_empty() || discovering { REFRESH_FAST } else { REFRESH };
        if self.last_refresh.is_some_and(|last| now.duration_since(last) < interval) {
            return false;
        }
        self.last_refresh = Some(now);
        true
    }

    /// Takes a response of the backend; returns a notice if one is due.
    pub fn handle(&mut self, response: Response) -> Option<Notice> {
        let focused = self.items().get(self.focus).cloned();
        let notice = match response {
            Response::Wifi(state) => {
                self.wifi = Some(state);
                None
            }
            Response::Bluetooth(state) => {
                self.bluetooth = Some(state);
                None
            }
            Response::Done(request, result) => {
                if let Some(pos) = self.busy.iter().position(|r| *r == request) {
                    self.busy.remove(pos);
                }
                request.notice(&result)
            }
        };
        // The focus stays on the same row, even when the order changes.
        let items = self.items();
        if let Some(pos) = focused.and_then(|item| items.iter().position(|i| *i == item)) {
            self.focus = pos;
        }
        self.focus = self.focus.min(items.len().saturating_sub(1));
        notice
    }

    fn wifi_state(&self) -> Option<&WifiState> {
        self.wifi.as_ref().and_then(|s| s.as_ref().ok())
    }

    fn bt_state(&self) -> Option<&BluetoothState> {
        self.bluetooth.as_ref().and_then(|s| s.as_ref().ok())
    }

    fn items(&self) -> Vec<Item> {
        match self.page {
            Some(Page::Wifi) => match self.wifi_state() {
                Some(state) if state.adapter => std::iter::once(Item::Toggle)
                    .chain(state.networks.iter().map(|n| Item::Network(n.ssid.clone())))
                    .collect(),
                _ => Vec::new(),
            },
            Some(Page::Bluetooth) => match self.bt_state() {
                Some(state) if state.adapter.is_some() => std::iter::once(Item::Toggle)
                    .chain(state.devices.iter().map(|d| Item::Device(d.address.clone())))
                    .collect(),
                _ => Vec::new(),
            },
            None => Vec::new(),
        }
    }

    fn focused_item(&self) -> Option<Item> {
        self.items().get(self.focus).cloned()
    }

    fn busy_for(&self, target: &str) -> Option<&Request> {
        self.busy.iter().find(|r| r.target() == Some(target))
    }

    /// Remembers running requests (for "Connecting …" and the faster refresh).
    fn send(&mut self, request: Request) -> Vec<Request> {
        if request != Request::Refresh {
            self.busy.push(request.clone());
        }
        vec![request]
    }

    /// Input on the sub-page. "Back" closes it (afterwards [`Self::is_open`] is false).
    pub fn apply(&mut self, action: Action) -> Vec<Request> {
        if self.password.is_some() {
            return self.apply_password(action);
        }
        let armed = self.armed.take();
        let len = self.items().len();
        match action {
            Action::Up => self.focus = self.focus.saturating_sub(1),
            Action::Down => self.focus = (self.focus + 1).min(len.saturating_sub(1)),
            Action::Back => self.page = None,
            Action::Confirm => return self.confirm(),
            Action::Secondary => match self.page {
                Some(Page::Wifi) if self.wifi_state().is_some_and(|s| s.adapter && s.enabled) => {
                    return self.send(Request::WifiScan);
                }
                Some(Page::Bluetooth) => {
                    if let Some(state) = self.bt_state().filter(|s| s.powered()) {
                        let on = !state.discovering();
                        return self.send(Request::BtDiscovery(on));
                    }
                }
                _ => {}
            },
            Action::Menu => {
                if let Some(device) = self.focused_device().filter(|d| d.paired).cloned() {
                    if armed.as_deref() == Some(device.address.as_str()) {
                        return self.send(Request::BtRemove { address: device.address, name: device.name });
                    }
                    self.armed = Some(device.address);
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn focused_network(&self) -> Option<&WifiNetwork> {
        match self.focused_item()? {
            Item::Network(ssid) => self.wifi_state()?.network(&ssid),
            _ => None,
        }
    }

    fn focused_device(&self) -> Option<&BtDevice> {
        match self.focused_item()? {
            Item::Device(address) => self.bt_state()?.device(&address),
            _ => None,
        }
    }

    fn confirm(&mut self) -> Vec<Request> {
        match (self.page, self.focused_item()) {
            (Some(Page::Wifi), Some(Item::Toggle)) => {
                let on = !self.wifi_state().is_some_and(|s| s.enabled);
                self.send(Request::WifiEnable(on))
            }
            (Some(Page::Bluetooth), Some(Item::Toggle)) => {
                let on = !self.bt_state().is_some_and(BluetoothState::powered);
                self.send(Request::BtPower(on))
            }
            (_, Some(Item::Network(ssid))) => {
                let Some(network) = self.focused_network().cloned() else { return Vec::new() };
                if self.busy_for(&ssid).is_some() {
                    return Vec::new();
                }
                if network.connected {
                    self.send(Request::WifiDisconnect { ssid })
                } else if network.secured && !network.known {
                    self.password = Some(Password {
                        ssid,
                        text: String::new(),
                        keyboard: OnScreenKeyboard::password(),
                        visible: false,
                    });
                    Vec::new()
                } else {
                    self.send(Request::WifiConnect { ssid, password: None })
                }
            }
            (_, Some(Item::Device(address))) => {
                let Some(device) = self.focused_device().cloned() else { return Vec::new() };
                if self.busy_for(&address).is_some() {
                    return Vec::new();
                }
                let name = device.name;
                if device.connected {
                    self.send(Request::BtDisconnect { address, name })
                } else if device.paired {
                    self.send(Request::BtConnect { address, name })
                } else {
                    self.send(Request::BtPair { address, name })
                }
            }
            _ => Vec::new(),
        }
    }

    fn apply_password(&mut self, action: Action) -> Vec<Request> {
        let Some(password) = self.password.as_mut() else { return Vec::new() };
        match action {
            Action::Back => self.password = None,
            Action::Secondary => {
                password.text.pop();
            }
            Action::Search => password.text.push(' '),
            Action::Menu => password.visible = !password.visible,
            _ => match password.keyboard.apply(action) {
                KeyboardEvent::Type(c) => password.text.push(c),
                KeyboardEvent::Space => password.text.push(' '),
                KeyboardEvent::Backspace => {
                    password.text.pop();
                }
                // Leaving the keyboard at the bottom must not connect by accident.
                KeyboardEvent::Done if action == Action::Confirm && !password.text.is_empty() => {
                    let Some(password) = self.password.take() else { return Vec::new() };
                    return self.send(Request::WifiConnect { ssid: password.ssid, password: Some(password.text) });
                }
                _ => {}
            },
        }
        Vec::new()
    }

    /// Input from the real keyboard in the password entry
    pub fn type_text(&mut self, text: &str) -> bool {
        let Some(password) = self.password.as_mut() else { return false };
        password.text.push_str(text);
        true
    }

    pub fn erase(&mut self) -> bool {
        let Some(password) = self.password.as_mut() else { return false };
        password.text.pop();
        true
    }

    /// Snapshot for picking the sound (changes when something visible changes)
    pub fn feedback(&self) -> [usize; 5] {
        let page = match self.page {
            None => 0,
            Some(Page::Wifi) => 1,
            Some(Page::Bluetooth) => 2,
        };
        let (key, len, open) =
            self.password.as_ref().map_or((0, 0, 0), |p| (p.keyboard.focused(), p.text.chars().count(), 1));
        [page, self.focus, key, len, open]
    }

    // --- Hints -----------------------------------------------------------------------------

    /// Label of ✕ for the focused row
    pub fn confirm_label(&self) -> Option<&'static str> {
        if self.password.is_some() {
            return Some(tr("Type"));
        }
        match self.focused_item()? {
            Item::Toggle => Some(match self.page? {
                Page::Wifi if self.wifi_state().is_some_and(|s| s.enabled) => tr("Turn off"),
                Page::Bluetooth if self.bt_state().is_some_and(BluetoothState::powered) => tr("Turn off"),
                _ => tr("Turn on"),
            }),
            Item::Network(_) => Some(if self.focused_network()?.connected { tr("Disconnect") } else { tr("Connect") }),
            Item::Device(_) => {
                let device = self.focused_device()?;
                Some(if device.connected {
                    tr("Disconnect")
                } else if device.paired {
                    tr("Connect")
                } else {
                    tr("Pair")
                })
            }
        }
    }

    /// Label of □ (scan Wi-Fi again, Bluetooth search)
    pub fn secondary_label(&self) -> Option<&'static str> {
        if self.password.is_some() {
            return Some(tr("Delete"));
        }
        match self.page? {
            Page::Wifi => self.wifi_state().filter(|s| s.adapter && s.enabled).map(|_| tr("Scan again")),
            Page::Bluetooth => {
                let state = self.bt_state().filter(|s| s.powered())?;
                Some(if state.discovering() { tr("Stop search") } else { tr("Find devices") })
            }
        }
    }

    /// Label of ≡ (remove device, show password)
    pub fn menu_label(&self) -> Option<&'static str> {
        if let Some(password) = &self.password {
            return Some(if password.visible { tr("Hide password") } else { tr("Show password") });
        }
        let device = self.focused_device().filter(|d| d.paired)?;
        let armed = self.armed.as_deref() == Some(device.address.as_str());
        Some(if armed { tr("Really remove") } else { tr("Remove") })
    }

    // --- Display ---------------------------------------------------------------------------

    pub fn password_view(&self) -> Option<PasswordView> {
        let password = self.password.as_ref()?;
        let text = if password.visible { password.text.clone() } else { "•".repeat(password.text.chars().count()) };
        Some(PasswordView {
            ssid: password.ssid.clone(),
            text,
            visible: password.visible,
            labels: (0..password.keyboard.keys().len()).map(|i| password.keyboard.label(i)).collect(),
            key_focused: password.keyboard.focused(),
        })
    }

    pub fn view(&self) -> PageView {
        match self.page {
            Some(Page::Wifi) => self.wifi_view(),
            Some(Page::Bluetooth) => self.bluetooth_view(),
            None => PageView::default(),
        }
    }

    fn toggle_row(&self, title: &str, detail: String, on: bool) -> RowView {
        RowView {
            symbol: "⏻".into(),
            title: title.into(),
            detail,
            value: if on { tr("On") } else { tr("Off") }.into(),
            busy: self.busy.iter().any(|r| matches!(r, Request::WifiEnable(_) | Request::BtPower(_))),
            ..RowView::default()
        }
    }

    fn wifi_view(&self) -> PageView {
        let mut view = PageView { title: tr("Wi-Fi").into(), focused: self.focus, ..PageView::default() };
        let state = match &self.wifi {
            None => {
                view.empty_title = tr("Loading …").into();
                return view;
            }
            Some(Err(err)) => {
                view.empty_title = tr("Can't reach NetworkManager").into();
                view.empty_text = err.clone();
                return view;
            }
            Some(Ok(state)) => state,
        };
        if !state.adapter {
            view.empty_title = tr("No Wi-Fi adapter").into();
            view.empty_text = tr("This device has no Wi-Fi adapter. A wired connection still works.").into();
            return view;
        }
        view.status = match state.connected() {
            Some(network) => trf("Connected to {}", &[&network.ssid]),
            None if state.enabled => tr("Not connected").into(),
            None => tr("Wi-Fi is off").into(),
        };
        if self.busy.contains(&Request::WifiScan) {
            view.status = trf("{} · Searching …", &[&view.status]);
        }
        view.rows.push(self.toggle_row(tr("Wi-Fi"), tr("Turn the radio on or off").into(), state.enabled));
        for network in &state.networks {
            let busy = self.busy_for(&network.ssid);
            let detail = match busy {
                Some(request) => request.busy_label(),
                None if network.connected => tr("Connected"),
                None if network.known => tr("Saved"),
                None if network.secured => tr("Secured"),
                None => tr("Unsecured"),
            };
            view.rows.push(RowView {
                title: network.ssid.clone(),
                detail: format!("{detail} · {} %", network.strength),
                bars: signal_bars(network.strength),
                secured: network.secured,
                active: network.connected,
                busy: busy.is_some(),
                ..RowView::default()
            });
        }
        if !state.enabled {
            view.empty_title = tr("Wi-Fi is off").into();
            view.empty_text = tr("Turn it on above to see networks.").into();
        } else if state.networks.is_empty() {
            view.empty_title = tr("No networks found").into();
            view.empty_text = tr("Scan again or move closer to the router.").into();
        }
        view
    }

    fn bluetooth_view(&self) -> PageView {
        let mut view = PageView { title: tr("Bluetooth").into(), focused: self.focus, ..PageView::default() };
        let state = match &self.bluetooth {
            None => {
                view.empty_title = tr("Loading …").into();
                return view;
            }
            Some(Err(err)) => {
                view.empty_title = tr("Can't reach Bluetooth").into();
                view.empty_text = err.clone();
                return view;
            }
            Some(Ok(state)) => state,
        };
        let Some(adapter) = &state.adapter else {
            view.empty_title = tr("No Bluetooth adapter").into();
            view.empty_text = tr("No Bluetooth adapter was found.").into();
            return view;
        };
        view.status = if !adapter.powered {
            tr("Bluetooth is off").into()
        } else if adapter.discovering {
            tr("Searching for devices …").into()
        } else {
            state.summary()
        };
        view.rows.push(self.toggle_row(tr("Bluetooth"), trf("Adapter {}", &[&adapter.name]), adapter.powered));
        for device in &state.devices {
            let busy = self.busy_for(&device.address);
            let armed = self.armed.as_deref() == Some(device.address.as_str());
            let detail = match busy {
                Some(request) => request.busy_label().to_string(),
                None if armed => tr("Press again to remove").into(),
                None => {
                    let state = if device.connected {
                        tr("Connected")
                    } else if device.paired {
                        tr("Paired")
                    } else {
                        tr("Not paired")
                    };
                    format!("{} · {state}", device.kind.label())
                }
            };
            view.rows.push(RowView {
                symbol: device.kind.symbol().into(),
                title: device.name.clone(),
                detail,
                gamepad: device.kind == DeviceKind::Gamepad,
                active: device.connected,
                busy: busy.is_some(),
                armed,
                ..RowView::default()
            });
        }
        if !adapter.powered {
            view.empty_title = tr("Bluetooth is off").into();
            view.empty_text = tr("Turn it on above to pair controllers.").into();
        } else if state.devices.is_empty() {
            view.empty_title = tr("No devices").into();
            view.empty_text = tr("Start a search and put the controller in pairing mode \
                                  (DualSense: hold PS + Create, Xbox: pairing button).")
            .into();
        }
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakenet::FakeBackend;
    use crate::network::Backend;

    /// Page with the state of the simulation
    fn pages(page: Page) -> (NetPages, FakeBackend) {
        let mut fake = FakeBackend::new(Duration::ZERO);
        let mut pages = NetPages::default();
        assert_eq!(pages.open(page), [Request::Refresh]);
        pages.handle(Response::Wifi(fake.wifi()));
        pages.handle(Response::Bluetooth(fake.bluetooth()));
        (pages, fake)
    }

    /// Runs the requests and hands the responses to the page, like the backend thread.
    fn run(pages: &mut NetPages, fake: &mut FakeBackend, requests: Vec<Request>) -> Vec<Notice> {
        let mut notices = Vec::new();
        for request in requests {
            let result = fake.execute(&request);
            notices.extend(pages.handle(Response::Done(request, result)));
        }
        pages.handle(Response::Wifi(fake.wifi()));
        pages.handle(Response::Bluetooth(fake.bluetooth()));
        notices
    }

    #[test]
    fn wifi_page_lists_networks_and_disconnects() {
        let (mut pages, mut fake) = pages(Page::Wifi);
        let view = pages.view();
        assert_eq!(view.status, "Connected to Zuhause");
        assert_eq!(view.rows[0].value, "On");
        assert_eq!(view.rows[1].title, "Zuhause");
        assert!(view.rows[1].active && view.rows[1].secured);
        assert_eq!(pages.confirm_label(), Some("Turn off"));
        pages.apply(Action::Down);
        assert_eq!(pages.confirm_label(), Some("Disconnect"));
        let requests = pages.apply(Action::Confirm);
        assert_eq!(requests, [Request::WifiDisconnect { ssid: "Zuhause".into() }]);
        assert!(pages.view().rows[1].busy);
        let notices = run(&mut pages, &mut fake, requests);
        assert_eq!(notices[0].title, "Wi-Fi disconnected");
        assert_eq!(pages.view().status, "Not connected");
        // The focus follows the network, even when it moves further down
        assert_eq!(pages.confirm_label(), Some("Connect"));
    }

    #[test]
    fn wifi_password_entry_with_keyboard() {
        let (mut pages, mut fake) = pages(Page::Wifi);
        // "FRITZ!Box" is secured and unknown (row 3)
        pages.apply(Action::Down);
        pages.apply(Action::Down);
        pages.apply(Action::Down);
        assert!(pages.apply(Action::Confirm).is_empty());
        assert!(pages.password_open());
        assert_eq!(pages.password_view().unwrap().ssid, "FRITZ!Box 7590 XY");
        // Sending it empty does not work; leaving at the bottom neither
        for _ in 0..4 {
            pages.apply(Action::Down);
        }
        // Shift → Space → ⌫ → Done
        for _ in 0..3 {
            pages.apply(Action::Right);
        }
        assert!(pages.apply(Action::Confirm).is_empty());
        assert!(pages.apply(Action::Down).is_empty());
        assert!(pages.type_text("geheim12"));
        assert_eq!(pages.password_view().unwrap().text, "••••••••");
        pages.apply(Action::Menu);
        assert_eq!(pages.password_view().unwrap().text, "geheim12");
        let requests = pages.apply(Action::Confirm);
        assert_eq!(
            requests,
            [Request::WifiConnect { ssid: "FRITZ!Box 7590 XY".into(), password: Some("geheim12".into()) }]
        );
        assert!(!pages.password_open());
        let notices = run(&mut pages, &mut fake, requests);
        assert_eq!(notices[0].title, "Connected to FRITZ!Box 7590 XY");
        assert_eq!(pages.view().rows[1].title, "FRITZ!Box 7590 XY");
    }

    #[test]
    fn wifi_wrong_password_and_back() {
        let (mut pages, mut fake) = pages(Page::Wifi);
        pages.focus = 3;
        pages.apply(Action::Confirm);
        pages.type_text("abc");
        pages.apply(Action::Secondary);
        assert_eq!(pages.password_view().unwrap().text.chars().count(), 2);
        pages.apply(Action::Back);
        assert!(!pages.password_open() && pages.is_open());
        pages.apply(Action::Confirm);
        pages.type_text("kurz");
        let notices = run(
            &mut pages,
            &mut fake,
            vec![Request::WifiConnect { ssid: "FRITZ!Box 7590 XY".into(), password: Some("kurz".into()) }],
        );
        assert_eq!(notices[0].title, "Connection failed");
        pages.apply(Action::Back);
        pages.apply(Action::Back);
        assert!(!pages.is_open());
    }

    #[test]
    fn no_adapter_shows_empty_state() {
        let mut pages = NetPages::default();
        pages.open(Page::Wifi);
        assert_eq!(pages.view().empty_title, "Loading …");
        assert_eq!(pages.summary(Page::Wifi), "…");
        pages.handle(Response::Wifi(Ok(WifiState { adapter: false, enabled: true, networks: Vec::new() })));
        let view = pages.view();
        assert_eq!(view.empty_title, "No Wi-Fi adapter");
        assert!(view.rows.is_empty());
        assert_eq!(pages.confirm_label(), None);
        assert_eq!(pages.secondary_label(), None);
        assert!(pages.apply(Action::Confirm).is_empty());
        assert!(pages.apply(Action::Secondary).is_empty());
        assert_eq!(pages.summary(Page::Wifi), "No adapter");
        pages.handle(Response::Bluetooth(Err("org.bluez missing".into())));
        assert_eq!(pages.summary(Page::Bluetooth), "Unavailable");
    }

    #[test]
    fn bluetooth_discover_pair_and_remove() {
        let (mut pages, mut fake) = pages(Page::Bluetooth);
        assert_eq!(pages.secondary_label(), Some("Find devices"));
        let requests = pages.apply(Action::Secondary);
        assert_eq!(requests, [Request::BtDiscovery(true)]);
        run(&mut pages, &mut fake, requests);
        let view = pages.view();
        assert_eq!(view.status, "Searching for devices …");
        // Toggle, DualSense, WH-1000XM4, Xbox, 8BitDo, JBL
        let titles: Vec<_> = view.rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles[3], "Xbox Wireless Controller");
        assert!(view.rows[3].gamepad && !view.rows[5].gamepad);
        pages.focus = 3;
        assert_eq!(pages.confirm_label(), Some("Pair"));
        let requests = pages.apply(Action::Confirm);
        assert_eq!(view.rows[3].detail, "Controller · Not paired");
        assert_eq!(pages.view().rows[3].detail, "Pairing …");
        // Pressing twice starts no second request
        assert!(pages.apply(Action::Confirm).is_empty());
        let notices = run(&mut pages, &mut fake, requests);
        assert_eq!(notices[0].title, "Paired: Xbox Wireless Controller");
        // The row moved up and the focus came along
        assert_eq!(pages.view().rows[pages.focus].title, "Xbox Wireless Controller");
        assert_eq!(pages.confirm_label(), Some("Disconnect"));
        // Removing needs two presses
        assert!(pages.apply(Action::Menu).is_empty());
        assert_eq!(pages.menu_label(), Some("Really remove"));
        assert!(pages.view().rows[pages.focus].armed);
        let requests = pages.apply(Action::Menu);
        assert!(matches!(&requests[..], [Request::BtRemove { .. }]));
        let notices = run(&mut pages, &mut fake, requests);
        assert_eq!(notices[0].title, "Device removed");
    }

    #[test]
    fn bluetooth_pair_failure_and_disarm() {
        let (mut pages, mut fake) = pages(Page::Bluetooth);
        let requests = pages.apply(Action::Secondary);
        run(&mut pages, &mut fake, requests);
        let index = pages.view().rows.iter().position(|r| r.title == "8BitDo Pro 2").unwrap();
        pages.focus = index;
        assert_eq!(pages.menu_label(), None);
        let requests = pages.apply(Action::Confirm);
        let notices = run(&mut pages, &mut fake, requests);
        assert_eq!(notices[0].title, "Pairing failed");
        // The confirmation expires on any other input
        pages.focus = 1;
        pages.apply(Action::Menu);
        pages.apply(Action::Down);
        pages.apply(Action::Up);
        assert!(pages.apply(Action::Menu).is_empty());
    }

    #[test]
    fn refresh_interval() {
        let mut pages = NetPages::default();
        let now = Instant::now();
        assert!(!pages.refresh_due(now, false));
        assert!(pages.refresh_due(now, true));
        assert!(!pages.refresh_due(now + Duration::from_secs(1), true));
        assert!(pages.refresh_due(now + Duration::from_secs(4), true));
        pages.busy.push(Request::WifiScan);
        assert!(pages.refresh_due(now + Duration::from_millis(5100), true));
    }
}
