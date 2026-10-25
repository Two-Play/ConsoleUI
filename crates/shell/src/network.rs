//! Unterseiten „WLAN“ und „Bluetooth“ der Einstellungen (Stories 5.3/5.4): verbindet den
//! Seitenzustand aus `consoleui_core::netpages` mit dem Netzwerk-Thread und baut die Slint-Ansicht.
//! D-Bus-Aufrufe laufen nur im Thread; `main.rs` fragt die Antworten per Timer ab.

use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use consoleui_core::fakenet::FakeBackend;
use consoleui_core::netpages::NetPages;
use consoleui_core::network::{self, Backend, Notice, Request, Response};
use consoleui_core::osk::OnScreenKeyboard;
use slint::{ModelRc, VecModel};

use crate::dbus::DbusBackend;
use crate::{GlyphButton, KeyCap, NetRow, NetView};

/// So lange „dauert“ Verbinden und Koppeln in der Simulation
const FAKE_DELAY: Duration = Duration::from_millis(900);

#[derive(Default)]
pub struct Network {
    pub pages: NetPages,
    requests: Option<Sender<Request>>,
    responses: Option<Receiver<Response>>,
}

impl Network {
    /// Startet den Netzwerk-Thread mit NetworkManager/BlueZ oder der Simulation (`--fake-network`).
    pub fn start(&mut self, fake: bool) {
        let backend: Box<dyn Backend> =
            if fake { Box::new(FakeBackend::new(FAKE_DELAY)) } else { Box::new(DbusBackend::default()) };
        let (requests, responses) = network::spawn(backend);
        self.requests = Some(requests);
        self.responses = Some(responses);
    }

    pub fn send(&self, requests: Vec<Request>) {
        let Some(sender) = &self.requests else { return };
        for request in requests {
            let _ = sender.send(request);
        }
    }

    /// Holt Antworten ab und stößt bei Bedarf ein Neulesen an. Gibt zurück, ob sich etwas geändert
    /// hat, und die fälligen Hinweise.
    pub fn poll(&mut self, visible: bool) -> (bool, Vec<Notice>) {
        if self.pages.refresh_due(Instant::now(), visible) {
            self.send(vec![Request::Refresh]);
        }
        let Some(responses) = &self.responses else { return (false, Vec::new()) };
        let responses: Vec<Response> = responses.try_iter().collect();
        let changed = !responses.is_empty();
        let notices = responses.into_iter().filter_map(|r| self.pages.handle(r)).collect();
        (changed, notices)
    }

    /// Hinweise unten (Taste, Beschriftung) für die offene Unterseite
    pub fn hints(&self) -> Vec<(GlyphButton, &'static str)> {
        let mut hints = Vec::new();
        if self.pages.password_open() {
            hints.push((GlyphButton::Confirm, "Eingeben"));
            hints.push((GlyphButton::West, "Löschen"));
            hints.push((GlyphButton::North, "Leerzeichen"));
        } else {
            hints.push((GlyphButton::Updown, "Auswählen"));
            if let Some(label) = self.pages.confirm_label() {
                hints.push((GlyphButton::Confirm, label));
            }
            if let Some(label) = self.pages.secondary_label() {
                hints.push((GlyphButton::West, label));
            }
        }
        if let Some(label) = self.pages.menu_label() {
            hints.push((GlyphButton::Menu, label));
        }
        hints.push((GlyphButton::Back, if self.pages.password_open() { "Abbrechen" } else { "Zurück" }));
        hints
    }

    pub fn view(&self) -> NetView {
        let page = self.pages.view();
        let rows: Vec<NetRow> = page
            .rows
            .into_iter()
            .map(|row| NetRow {
                symbol: row.symbol.into(),
                title: row.title.into(),
                detail: row.detail.into(),
                value: row.value.into(),
                bars: row.bars.into(),
                secured: row.secured,
                gamepad: row.gamepad,
                active: row.active,
                busy: row.busy,
                armed: row.armed,
            })
            .collect();
        let password = self.pages.password_view();
        let keys: Vec<KeyCap> = match &password {
            Some(view) => OnScreenKeyboard::password()
                .keys()
                .iter()
                .zip(&view.labels)
                .map(|(cap, label)| KeyCap { label: label.into(), row: cap.row as i32, x: cap.x, width: cap.width })
                .collect(),
            None => Vec::new(),
        };
        NetView {
            open: self.pages.is_open(),
            title: page.title.into(),
            status: page.status.into(),
            rows: ModelRc::new(VecModel::from(rows)),
            focused: page.focused as i32,
            empty_title: page.empty_title.into(),
            empty_text: page.empty_text.into(),
            password_open: password.is_some(),
            password_ssid: password.as_ref().map(|p| p.ssid.clone()).unwrap_or_default().into(),
            password_text: password.as_ref().map(|p| p.text.clone()).unwrap_or_default().into(),
            keys: ModelRc::new(VecModel::from(keys)),
            key_focused: password.as_ref().map_or(0, |p| p.key_focused as i32),
        }
    }
}
