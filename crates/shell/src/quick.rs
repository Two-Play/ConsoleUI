//! Inhalt und Zustand des Quick-Menüs (PS-/Guide-Taste).

use consoleui_core::system::{self, PowerAction, Volume};

use crate::QuickItem;

#[derive(Debug, Clone, PartialEq)]
pub enum QuickAction {
    /// Zur laufenden App zurück (gamescope-App-ID)
    Resume(u32),
    Home,
    /// Laufende App beenden (Eintrags-ID)
    Quit(String),
    Volume,
    Power(PowerAction),
    Logout,
}

impl QuickAction {
    /// Folgenschwere Aktionen brauchen einen zweiten Druck.
    fn needs_confirmation(&self) -> bool {
        matches!(self, Self::Quit(_) | Self::Power(PowerAction::Reboot | PowerAction::PowerOff) | Self::Logout)
    }
}

/// Die App, die gerade im Vordergrund läuft
pub struct ForegroundApp {
    pub app_id: u32,
    pub entry_id: String,
    pub title: String,
}

pub struct QuickMenu {
    actions: Vec<QuickAction>,
    focused: usize,
    /// Fokussierte Aktion wartet auf Bestätigung
    armed: bool,
    volume: Option<Volume>,
    app_title: Option<String>,
}

/// Was nach einer Eingabe im Quick-Menü passieren soll
pub enum QuickOutcome {
    Stay,
    Close,
    Run(QuickAction),
}

impl QuickMenu {
    pub fn new(foreground: Option<ForegroundApp>) -> Self {
        let mut actions = Vec::new();
        let app_title = foreground.as_ref().map(|app| app.title.clone());
        if let Some(app) = foreground {
            actions.extend([QuickAction::Resume(app.app_id), QuickAction::Home, QuickAction::Quit(app.entry_id)]);
        }
        actions.extend([
            QuickAction::Volume,
            QuickAction::Power(PowerAction::Suspend),
            QuickAction::Power(PowerAction::Reboot),
            QuickAction::Power(PowerAction::PowerOff),
            QuickAction::Logout,
        ]);
        Self { actions, focused: 0, armed: false, volume: system::volume(), app_title }
    }

    pub fn focused(&self) -> usize {
        self.focused
    }

    pub fn title(&self) -> String {
        match &self.app_title {
            Some(title) => format!("STEUERZENTRALE · {}", title.to_uppercase()),
            None => "STEUERZENTRALE".into(),
        }
    }

    pub fn apply(&mut self, action: consoleui_core::Action) -> QuickOutcome {
        use consoleui_core::Action;
        let current = self.actions[self.focused].clone();
        match action {
            Action::Left | Action::Right => {
                let next = if action == Action::Left {
                    self.focused.checked_sub(1)
                } else {
                    Some(self.focused + 1).filter(|&i| i < self.actions.len())
                };
                if let Some(next) = next {
                    self.focused = next;
                    self.armed = false;
                }
                QuickOutcome::Stay
            }
            Action::Up | Action::Down if current == QuickAction::Volume => {
                report(system::change_volume(action == Action::Up));
                self.volume = system::volume();
                QuickOutcome::Stay
            }
            Action::Confirm if current == QuickAction::Volume => {
                report(system::toggle_mute());
                self.volume = system::volume();
                QuickOutcome::Stay
            }
            Action::Confirm if current.needs_confirmation() && !self.armed => {
                self.armed = true;
                QuickOutcome::Stay
            }
            Action::Confirm => QuickOutcome::Run(current),
            Action::Back | Action::Guide if self.armed => {
                self.armed = false;
                QuickOutcome::Stay
            }
            Action::Back | Action::Guide => QuickOutcome::Close,
            _ => QuickOutcome::Stay,
        }
    }

    pub fn items(&self) -> Vec<QuickItem> {
        self.actions
            .iter()
            .enumerate()
            .map(|(i, action)| {
                let armed = self.armed && i == self.focused;
                let (symbol, label, value) = match action {
                    QuickAction::Resume(_) => ("▶", "Fortsetzen", self.app_title.clone().unwrap_or_default()),
                    QuickAction::Home => ("⌂", "Home", "Zur Übersicht".to_string()),
                    QuickAction::Quit(_) => ("■", "Beenden", self.app_title.clone().unwrap_or_default()),
                    QuickAction::Volume => ("♪", "Lautstärke", self.volume_text()),
                    QuickAction::Power(PowerAction::Suspend) => ("☾", "Standby", String::new()),
                    QuickAction::Power(PowerAction::Reboot) => ("↻", "Neustart", String::new()),
                    QuickAction::Power(PowerAction::PowerOff) => ("⏻", "Ausschalten", String::new()),
                    QuickAction::Logout => ("⇥", "Abmelden", "Shell beenden".to_string()),
                };
                // Wartet die Aktion auf Bestätigung, rückt ihr Name in die zweite Zeile.
                let (label, value) = if armed { ("Wirklich?", label.to_string()) } else { (label, value) };
                let level = match (action, self.volume) {
                    (QuickAction::Volume, Some(volume)) if !volume.muted => volume.percent.into(),
                    (QuickAction::Volume, Some(_)) => 0,
                    _ => -1,
                };
                QuickItem {
                    symbol: symbol.into(),
                    label: label.into(),
                    value: value.into(),
                    armed,
                    adjustable: *action == QuickAction::Volume,
                    level,
                }
            })
            .collect()
    }

    fn volume_text(&self) -> String {
        match self.volume {
            Some(Volume { muted: true, .. }) => "Stumm".into(),
            Some(volume) => format!("{} %", volume.percent),
            None => "nicht verfügbar".into(),
        }
    }
}

fn report(result: std::io::Result<()>) {
    if let Err(err) = result {
        eprintln!("Quick-Menü: {err}");
    }
}
