//! "Wi-Fi" and "Bluetooth" sub-pages of the settings (Stories 5.3/5.4): connects the page state
//! from `consoleui_core::netpages` with the network thread and builds the Slint view. D-Bus calls
//! only run on the thread; `main.rs` polls the responses with a timer.

use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use consoleui_core::fakenet::FakeBackend;
use consoleui_core::i18n::tr;
use consoleui_core::netpages::NetPages;
use consoleui_core::network::{self, Backend, Notice, Request, Response};
use consoleui_core::osk::OnScreenKeyboard;
use slint::{ModelRc, VecModel};

use crate::dbus::DbusBackend;
use crate::{GlyphButton, KeyCap, NetRow, NetView};

/// How long connecting and pairing "take" in the simulation
const FAKE_DELAY: Duration = Duration::from_millis(900);

#[derive(Default)]
pub struct Network {
    pub pages: NetPages,
    requests: Option<Sender<Request>>,
    responses: Option<Receiver<Response>>,
}

impl Network {
    /// Starts the network thread with NetworkManager/BlueZ or the simulation (`--fake-network`).
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

    /// Fetches responses and triggers a refresh when due. Returns whether something changed, and
    /// the notices that are due.
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

    /// Hints at the bottom (button, label) for the open sub-page
    pub fn hints(&self) -> Vec<(GlyphButton, &'static str)> {
        let mut hints = Vec::new();
        if self.pages.password_open() {
            hints.push((GlyphButton::Confirm, tr("Type")));
            hints.push((GlyphButton::West, tr("Delete")));
            hints.push((GlyphButton::North, tr("Space")));
        } else {
            hints.push((GlyphButton::Updown, tr("Select")));
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
        hints.push((GlyphButton::Back, if self.pages.password_open() { tr("Cancel") } else { tr("Back") }));
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
