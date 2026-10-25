//! Zustand und Bedienung der Unterseiten „WLAN“ und „Bluetooth“ der Einstellungen
//! (Stories 5.3/5.4): Fokus, Passwort-Eingabe mit der Bildschirmtastatur, Rückfrage vor dem
//! Entfernen, laufende Aufträge und die Anzeige als einfache Zeilen. Ohne UI und ohne D-Bus;
//! die Aufträge führt der Backend-Thread aus ([`crate::network`]).

use std::time::{Duration, Instant};

use crate::bluetooth::{BluetoothState, BtDevice, DeviceKind};
use crate::nav::Action;
use crate::network::{Notice, Request, Response};
use crate::osk::{KeyboardEvent, OnScreenKeyboard};
use crate::wifi::{WifiNetwork, WifiState, signal_bars};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Wifi,
    Bluetooth,
}

/// So oft wird neu gelesen, solange die Einstellungen zu sehen sind
const REFRESH: Duration = Duration::from_secs(3);
/// … und während einer Suche oder eines laufenden Auftrags
const REFRESH_FAST: Duration = Duration::from_secs(1);

/// Eine Zeile der Seite
#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    /// WLAN bzw. Bluetooth ein/aus
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

/// Anzeige einer Zeile
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowView {
    pub symbol: String,
    pub title: String,
    pub detail: String,
    /// Rechts in der Zeile, z. B. „An“
    pub value: String,
    /// WLAN-Balken 1–4, sonst 0
    pub bars: u8,
    pub secured: bool,
    pub gamepad: bool,
    /// Verbunden
    pub active: bool,
    pub busy: bool,
    /// Wartet auf den zweiten Druck zum Entfernen
    pub armed: bool,
}

/// Anzeige der Passwort-Eingabe
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordView {
    pub ssid: String,
    /// Maskiert, außer „anzeigen“ ist an
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
    /// Text unter den Zeilen, wenn es nichts aufzulisten gibt
    pub empty_title: String,
    pub empty_text: String,
}

#[derive(Default)]
pub struct NetPages {
    page: Option<Page>,
    focus: usize,
    /// `None`: noch nicht gelesen
    wifi: Option<Result<WifiState, String>>,
    bluetooth: Option<Result<BluetoothState, String>>,
    /// Laufende Aufträge (ohne `Refresh`)
    busy: Vec<Request>,
    password: Option<Password>,
    /// Adresse des Geräts, das beim nächsten Druck entfernt wird
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

    /// Öffnet eine Unterseite; der Zustand wird gleich neu gelesen.
    pub fn open(&mut self, page: Page) -> Vec<Request> {
        self.page = Some(page);
        self.focus = 0;
        self.armed = None;
        self.password = None;
        self.last_refresh = None;
        vec![Request::Refresh]
    }

    /// Kurzer Zustand für die Zeilen der Einstellungsseite
    pub fn summary(&self, page: Page) -> String {
        let summary = match page {
            Page::Wifi => self.wifi.as_ref().map(|s| s.as_ref().map(WifiState::summary)),
            Page::Bluetooth => self.bluetooth.as_ref().map(|s| s.as_ref().map(BluetoothState::summary)),
        };
        match summary {
            None => "…".into(),
            Some(Ok(text)) => text,
            Some(Err(_)) => "Nicht verfügbar".into(),
        }
    }

    /// Ob jetzt neu gelesen werden soll (`visible`: die Einstellungen sind zu sehen).
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

    /// Übernimmt eine Antwort des Backends; gibt einen Hinweis zurück, falls einer fällig ist.
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
        // Der Fokus bleibt auf derselben Zeile, auch wenn sich die Reihenfolge ändert.
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

    /// Merkt sich laufende Aufträge (für „Verbinde …“ und das schnellere Neulesen).
    fn send(&mut self, request: Request) -> Vec<Request> {
        if request != Request::Refresh {
            self.busy.push(request.clone());
        }
        vec![request]
    }

    /// Eingabe auf der Unterseite. „Zurück“ schließt sie (danach ist [`Self::is_open`] falsch).
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
                // Unten aus der Tastatur heraus soll nicht versehentlich verbinden.
                KeyboardEvent::Done if action == Action::Confirm && !password.text.is_empty() => {
                    let Some(password) = self.password.take() else { return Vec::new() };
                    return self.send(Request::WifiConnect { ssid: password.ssid, password: Some(password.text) });
                }
                _ => {}
            },
        }
        Vec::new()
    }

    /// Eingabe über die echte Tastatur in der Passwort-Eingabe
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

    /// Momentaufnahme für die Klangauswahl (ändert sich, wenn sich etwas Sichtbares ändert)
    pub fn feedback(&self) -> [usize; 5] {
        let page = match self.page {
            None => 0,
            Some(Page::Wifi) => 1,
            Some(Page::Bluetooth) => 2,
        };
        let (key, len, open) = self
            .password
            .as_ref()
            .map_or((0, 0, 0), |p| (p.keyboard.focused(), p.text.chars().count(), 1));
        [page, self.focus, key, len, open]
    }

    // --- Hinweise --------------------------------------------------------------------------

    /// Beschriftung von ✕ für die fokussierte Zeile
    pub fn confirm_label(&self) -> Option<&'static str> {
        if self.password.is_some() {
            return Some("Eingeben");
        }
        match self.focused_item()? {
            Item::Toggle => Some(match self.page? {
                Page::Wifi if self.wifi_state().is_some_and(|s| s.enabled) => "Ausschalten",
                Page::Bluetooth if self.bt_state().is_some_and(BluetoothState::powered) => "Ausschalten",
                _ => "Einschalten",
            }),
            Item::Network(_) => {
                Some(if self.focused_network()?.connected { "Trennen" } else { "Verbinden" })
            }
            Item::Device(_) => {
                let device = self.focused_device()?;
                Some(if device.connected {
                    "Trennen"
                } else if device.paired {
                    "Verbinden"
                } else {
                    "Koppeln"
                })
            }
        }
    }

    /// Beschriftung von □ (WLAN neu suchen, Bluetooth-Suche)
    pub fn secondary_label(&self) -> Option<&'static str> {
        if self.password.is_some() {
            return Some("Löschen");
        }
        match self.page? {
            Page::Wifi => self.wifi_state().filter(|s| s.adapter && s.enabled).map(|_| "Neu suchen"),
            Page::Bluetooth => {
                let state = self.bt_state().filter(|s| s.powered())?;
                Some(if state.discovering() { "Suche beenden" } else { "Geräte suchen" })
            }
        }
    }

    /// Beschriftung von ≡ (Gerät entfernen, Passwort anzeigen)
    pub fn menu_label(&self) -> Option<&'static str> {
        if let Some(password) = &self.password {
            return Some(if password.visible { "Verbergen" } else { "Anzeigen" });
        }
        let device = self.focused_device().filter(|d| d.paired)?;
        Some(if self.armed.as_deref() == Some(device.address.as_str()) { "Wirklich entfernen" } else { "Entfernen" })
    }

    // --- Anzeige ---------------------------------------------------------------------------

    pub fn password_view(&self) -> Option<PasswordView> {
        let password = self.password.as_ref()?;
        let text = if password.visible {
            password.text.clone()
        } else {
            "•".repeat(password.text.chars().count())
        };
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
            value: if on { "An" } else { "Aus" }.into(),
            busy: self.busy.iter().any(|r| matches!(r, Request::WifiEnable(_) | Request::BtPower(_))),
            ..RowView::default()
        }
    }

    fn wifi_view(&self) -> PageView {
        let mut view = PageView { title: "WLAN".into(), focused: self.focus, ..PageView::default() };
        let state = match &self.wifi {
            None => {
                view.empty_title = "Wird geladen …".into();
                return view;
            }
            Some(Err(err)) => {
                view.empty_title = "NetworkManager nicht erreichbar".into();
                view.empty_text = err.clone();
                return view;
            }
            Some(Ok(state)) => state,
        };
        if !state.adapter {
            view.empty_title = "Kein WLAN-Adapter".into();
            view.empty_text =
                "Dieses Gerät hat keinen WLAN-Adapter. Eine Verbindung per Kabel funktioniert weiterhin.".into();
            return view;
        }
        view.status = match state.connected() {
            Some(network) => format!("Verbunden mit {}", network.ssid),
            None if state.enabled => "Nicht verbunden".into(),
            None => "WLAN ist ausgeschaltet".into(),
        };
        if self.busy.contains(&Request::WifiScan) {
            view.status.push_str(" · Suche …");
        }
        view.rows.push(self.toggle_row("WLAN", "Funk ein- oder ausschalten".into(), state.enabled));
        for network in &state.networks {
            let busy = self.busy_for(&network.ssid);
            let detail = match busy {
                Some(request) => request.busy_label(),
                None if network.connected => "Verbunden",
                None if network.known => "Gespeichert",
                None if network.secured => "Gesichert",
                None => "Offen",
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
            view.empty_title = "WLAN ist ausgeschaltet".into();
            view.empty_text = "Schalte es oben ein, um Netzwerke zu sehen.".into();
        } else if state.networks.is_empty() {
            view.empty_title = "Keine Netzwerke gefunden".into();
            view.empty_text = "Suche neu oder geh näher an den Router.".into();
        }
        view
    }

    fn bluetooth_view(&self) -> PageView {
        let mut view = PageView { title: "Bluetooth".into(), focused: self.focus, ..PageView::default() };
        let state = match &self.bluetooth {
            None => {
                view.empty_title = "Wird geladen …".into();
                return view;
            }
            Some(Err(err)) => {
                view.empty_title = "Bluetooth nicht erreichbar".into();
                view.empty_text = err.clone();
                return view;
            }
            Some(Ok(state)) => state,
        };
        let Some(adapter) = &state.adapter else {
            view.empty_title = "Kein Bluetooth-Adapter".into();
            view.empty_text = "Es wurde kein Bluetooth-Adapter gefunden.".into();
            return view;
        };
        view.status = if !adapter.powered {
            "Bluetooth ist ausgeschaltet".into()
        } else if adapter.discovering {
            "Suche nach Geräten …".into()
        } else {
            state.summary()
        };
        view.rows.push(self.toggle_row("Bluetooth", format!("Adapter {}", adapter.name), adapter.powered));
        for device in &state.devices {
            let busy = self.busy_for(&device.address);
            let armed = self.armed.as_deref() == Some(device.address.as_str());
            let detail = match busy {
                Some(request) => request.busy_label().to_string(),
                None if armed => "Zum Entfernen nochmal drücken".into(),
                None => {
                    let state = if device.connected {
                        "Verbunden"
                    } else if device.paired {
                        "Gekoppelt"
                    } else {
                        "Nicht gekoppelt"
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
            view.empty_title = "Bluetooth ist ausgeschaltet".into();
            view.empty_text = "Schalte es oben ein, um Controller zu koppeln.".into();
        } else if state.devices.is_empty() {
            view.empty_title = "Keine Geräte".into();
            view.empty_text = "Starte die Suche und versetze den Controller in den Kopplungsmodus \
                               (DualSense: PS + Create gedrückt halten, Xbox: Pairing-Taste)."
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

    /// Seite mit dem Zustand der Simulation
    fn pages(page: Page) -> (NetPages, FakeBackend) {
        let mut fake = FakeBackend::new(Duration::ZERO);
        let mut pages = NetPages::default();
        assert_eq!(pages.open(page), [Request::Refresh]);
        pages.handle(Response::Wifi(fake.wifi()));
        pages.handle(Response::Bluetooth(fake.bluetooth()));
        (pages, fake)
    }

    /// Führt die Aufträge aus und gibt die Antworten an die Seite weiter, wie der Backend-Thread.
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
        assert_eq!(view.status, "Verbunden mit Zuhause");
        assert_eq!(view.rows[0].value, "An");
        assert_eq!(view.rows[1].title, "Zuhause");
        assert!(view.rows[1].active && view.rows[1].secured);
        assert_eq!(pages.confirm_label(), Some("Ausschalten"));
        pages.apply(Action::Down);
        assert_eq!(pages.confirm_label(), Some("Trennen"));
        let requests = pages.apply(Action::Confirm);
        assert_eq!(requests, [Request::WifiDisconnect { ssid: "Zuhause".into() }]);
        assert!(pages.view().rows[1].busy);
        let notices = run(&mut pages, &mut fake, requests);
        assert_eq!(notices[0].title, "WLAN getrennt");
        assert_eq!(pages.view().status, "Nicht verbunden");
        // Der Fokus folgt dem Netz, auch wenn es weiter nach unten rutscht
        assert_eq!(pages.confirm_label(), Some("Verbinden"));
    }

    #[test]
    fn wifi_password_entry_with_keyboard() {
        let (mut pages, mut fake) = pages(Page::Wifi);
        // „FRITZ!Box“ ist gesichert und unbekannt (Zeile 3)
        pages.apply(Action::Down);
        pages.apply(Action::Down);
        pages.apply(Action::Down);
        assert!(pages.apply(Action::Confirm).is_empty());
        assert!(pages.password_open());
        assert_eq!(pages.password_view().unwrap().ssid, "FRITZ!Box 7590 XY");
        // Leer abschicken geht nicht; unten heraus auch nicht
        for _ in 0..4 {
            pages.apply(Action::Down);
        }
        // Umschalt → Leerzeichen → ⌫ → Fertig
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
        assert_eq!(notices[0].title, "Verbunden mit FRITZ!Box 7590 XY");
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
        let notices = run(&mut pages, &mut fake, vec![Request::WifiConnect {
            ssid: "FRITZ!Box 7590 XY".into(),
            password: Some("kurz".into()),
        }]);
        assert_eq!(notices[0].title, "Verbindung fehlgeschlagen");
        pages.apply(Action::Back);
        pages.apply(Action::Back);
        assert!(!pages.is_open());
    }

    #[test]
    fn no_adapter_shows_empty_state() {
        let mut pages = NetPages::default();
        pages.open(Page::Wifi);
        assert_eq!(pages.view().empty_title, "Wird geladen …");
        assert_eq!(pages.summary(Page::Wifi), "…");
        pages.handle(Response::Wifi(Ok(WifiState { adapter: false, enabled: true, networks: Vec::new() })));
        let view = pages.view();
        assert_eq!(view.empty_title, "Kein WLAN-Adapter");
        assert!(view.rows.is_empty());
        assert_eq!(pages.confirm_label(), None);
        assert_eq!(pages.secondary_label(), None);
        assert!(pages.apply(Action::Confirm).is_empty());
        assert!(pages.apply(Action::Secondary).is_empty());
        assert_eq!(pages.summary(Page::Wifi), "Kein Adapter");
        pages.handle(Response::Bluetooth(Err("org.bluez fehlt".into())));
        assert_eq!(pages.summary(Page::Bluetooth), "Nicht verfügbar");
    }

    #[test]
    fn bluetooth_discover_pair_and_remove() {
        let (mut pages, mut fake) = pages(Page::Bluetooth);
        assert_eq!(pages.secondary_label(), Some("Geräte suchen"));
        let requests = pages.apply(Action::Secondary);
        assert_eq!(requests, [Request::BtDiscovery(true)]);
        run(&mut pages, &mut fake, requests);
        let view = pages.view();
        assert_eq!(view.status, "Suche nach Geräten …");
        // Toggle, DualSense, WH-1000XM4, Xbox, 8BitDo, JBL
        let titles: Vec<_> = view.rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles[3], "Xbox Wireless Controller");
        assert!(view.rows[3].gamepad && !view.rows[5].gamepad);
        pages.focus = 3;
        assert_eq!(pages.confirm_label(), Some("Koppeln"));
        let requests = pages.apply(Action::Confirm);
        assert_eq!(view.rows[3].detail, "Controller · Nicht gekoppelt");
        assert_eq!(pages.view().rows[3].detail, "Kopple …");
        // Zweimal drücken startet keinen zweiten Auftrag
        assert!(pages.apply(Action::Confirm).is_empty());
        let notices = run(&mut pages, &mut fake, requests);
        assert_eq!(notices[0].title, "Gekoppelt: Xbox Wireless Controller");
        // Die Zeile ist nach oben gewandert, der Fokus ist mitgekommen
        assert_eq!(pages.view().rows[pages.focus].title, "Xbox Wireless Controller");
        assert_eq!(pages.confirm_label(), Some("Trennen"));
        // Entfernen braucht zwei Drücke
        assert!(pages.apply(Action::Menu).is_empty());
        assert_eq!(pages.menu_label(), Some("Wirklich entfernen"));
        assert!(pages.view().rows[pages.focus].armed);
        let requests = pages.apply(Action::Menu);
        assert!(matches!(&requests[..], [Request::BtRemove { .. }]));
        let notices = run(&mut pages, &mut fake, requests);
        assert_eq!(notices[0].title, "Gerät entfernt");
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
        assert_eq!(notices[0].title, "Kopplung fehlgeschlagen");
        // Rückfrage verfällt bei jeder anderen Eingabe
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
