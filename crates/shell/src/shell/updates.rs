//! The "Updates" page of the settings (Story 5.8): connects the page state from
//! `consoleui_core::updates` with the update thread and shows it in the Wi-Fi page's layer. What
//! changed in an update opens as a dialog. The real backend is `crate::updates`.

use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use consoleui_core::updates::{Backend, FakeBackend, Notice, Outcome, Request, Response, UpdatePage, spawn};
use slint::{ModelRc, VecModel};

use super::*;
use crate::updates::SystemBackend;
use crate::{NetRow, NetView};

/// How long a step "takes" in the simulation
const FAKE_DELAY: Duration = Duration::from_millis(700);

#[derive(Default)]
pub(super) struct Updates {
    pub page: UpdatePage,
    requests: Option<Sender<Request>>,
    responses: Option<Receiver<Response>>,
}

impl Updates {
    fn send(&self, requests: Vec<Request>) {
        let Some(sender) = &self.requests else { return };
        for request in requests {
            let _ = sender.send(request);
        }
    }
}

impl Shell {
    /// Starts the update thread with the system's sources or the simulation (`--fake-updates`).
    pub fn start_updates(&mut self, fake: bool) {
        let backend: Box<dyn Backend> =
            if fake { Box::new(FakeBackend::new(FAKE_DELAY)) } else { Box::new(SystemBackend::default()) };
        let (requests, responses) = spawn(backend);
        self.updates.requests = Some(requests);
        self.updates.responses = Some(responses);
    }

    /// Runs on a timer: searches when due (never next to a game) and takes over the responses.
    pub fn poll_updates(&mut self, ui: &AppWindow) {
        if self.running.is_empty() {
            let requests = self.updates.page.check_due(Instant::now());
            self.updates.send(requests);
        }
        let Some(responses) = &self.updates.responses else { return };
        let responses: Vec<Response> = responses.try_iter().collect();
        if responses.is_empty() {
            return;
        }
        for response in responses {
            match self.updates.page.handle(response) {
                Some(Outcome::Notice(Notice { symbol, title, text })) => {
                    if self.settings.notifications {
                        self.toast(ui, symbol, &title, &text);
                    }
                }
                Some(Outcome::ShowDetails(title, text)) => self.show_alert(ui, &title, &text),
                None => {}
            }
        }
        self.update_settings_rows(ui);
        if self.updates.page.is_open() {
            ui.set_net_view(self.updates_view());
            self.update_hints(ui);
        }
    }

    pub(super) fn open_updates(&mut self, ui: &AppWindow) {
        let requests = self.updates.page.open();
        self.updates.send(requests);
        ui.set_net_view(self.updates_view());
    }

    pub(super) fn apply_updates(&mut self, ui: &AppWindow, action: Action) {
        if action == Action::Secondary {
            self.confirm_install(ui);
            return;
        }
        let requests = self.updates.page.apply(action);
        self.updates.send(requests);
        ui.set_net_view(self.updates_view());
    }

    /// Asks before installing; next to a running game nothing is installed.
    fn confirm_install(&mut self, ui: &AppWindow) {
        if self.updates.page.updates().is_empty() || self.updates.page.installing() {
            return;
        }
        if !self.running.is_empty() {
            self.toast(ui, "⚠", tr("Updates have to wait"), tr("Quit the running apps first"));
            return;
        }
        let buttons = vec![
            DialogButton { label: tr("Cancel").into(), tag: DialogTag::Close },
            DialogButton { label: tr("Install").into(), tag: DialogTag::InstallUpdates },
        ];
        let text = tr("Apps update right away, the system at the next restart.");
        self.show_dialog(ui, Dialog::new(tr("Install all updates?"), text, buttons, 0));
    }

    pub(super) fn install_updates(&mut self, ui: &AppWindow) {
        let requests = self.updates.page.install();
        self.updates.send(requests);
        ui.set_net_view(self.updates_view());
        self.update_hints(ui);
    }

    pub(super) fn updates_hints(&self) -> Vec<(GlyphButton, &'static str)> {
        let page = &self.updates.page;
        let mut hints = Vec::new();
        if !page.updates().is_empty() {
            hints.push((GlyphButton::Updown, tr("Select")));
            hints.push((GlyphButton::Confirm, tr("Changes")));
            if !page.installing() {
                hints.push((GlyphButton::West, tr("Install all")));
            }
        }
        if !page.installing() {
            hints.push((GlyphButton::Menu, tr("Search")));
        }
        hints.push((GlyphButton::Back, tr("Back")));
        hints
    }

    pub(super) fn updates_view(&self) -> NetView {
        let page = &self.updates.page;
        let empty = page.updates().is_empty();
        let rows: Vec<NetRow> = page
            .rows()
            .into_iter()
            .map(|row| NetRow {
                symbol: "↓".into(),
                title: row.title.into(),
                detail: row.detail.into(),
                value: if row.security { tr("Security").into() } else { Default::default() },
                busy: row.busy,
                ..Default::default()
            })
            .collect();
        NetView {
            open: page.is_open(),
            title: tr("Updates").into(),
            status: page.status().into(),
            rows: ModelRc::new(VecModel::from(rows)),
            focused: page.focused() as i32,
            empty_title: if empty { page.summary().into() } else { Default::default() },
            empty_text: if empty { page.empty_text().into() } else { Default::default() },
            ..Default::default()
        }
    }
}
