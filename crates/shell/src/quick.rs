//! Content and state of the quick menu (PS/Guide button).

use consoleui_core::i18n::{tr, trf};
use consoleui_core::system::{self, PowerAction, Volume};

use crate::QuickItem;

#[derive(Debug, Clone, PartialEq)]
pub enum QuickAction {
    /// Back to the running app (gamescope app ID)
    Resume(u32),
    Home,
    /// Quit the running app (entry ID)
    Quit(String),
    /// Open the profile screen (Story 5.7)
    SwitchProfile,
    Volume,
    Power(PowerAction),
    Logout,
}

impl QuickAction {
    /// Actions with serious consequences need a second press.
    fn needs_confirmation(&self) -> bool {
        matches!(self, Self::Quit(_) | Self::Power(PowerAction::Reboot | PowerAction::PowerOff) | Self::Logout)
    }
}

/// A running app the quick menu offers actions for: the one in front, or (with the shell in front)
/// the one that was started last
pub struct ForegroundApp {
    pub app_id: u32,
    pub entry_id: String,
    pub title: String,
    /// The app is in front; otherwise the shell is, and the menu offers the way back
    pub in_front: bool,
}

pub struct QuickMenu {
    actions: Vec<QuickAction>,
    focused: usize,
    /// The focused action waits for confirmation
    armed: bool,
    volume: Option<Volume>,
    app_title: Option<String>,
    app_in_front: bool,
    /// Name of the active profile
    profile: String,
}

/// What should happen after an input in the quick menu
pub enum QuickOutcome {
    Stay,
    Close,
    Run(QuickAction),
}

impl QuickMenu {
    pub fn new(foreground: Option<ForegroundApp>, profile: String) -> Self {
        let mut actions = Vec::new();
        let app_title = foreground.as_ref().map(|app| app.title.clone());
        let app_in_front = foreground.as_ref().is_some_and(|app| app.in_front);
        // In front the menu closes back into the app with ○, so "Resume" would only repeat that:
        // there the app offers the way home. With the shell in front (home) it is the way back.
        if let Some(app) = foreground {
            actions.push(if app.in_front { QuickAction::Home } else { QuickAction::Resume(app.app_id) });
            actions.push(QuickAction::Quit(app.entry_id));
        }
        actions.extend([
            QuickAction::SwitchProfile,
            QuickAction::Volume,
            QuickAction::Power(PowerAction::Suspend),
            QuickAction::Power(PowerAction::Reboot),
            QuickAction::Power(PowerAction::PowerOff),
            QuickAction::Logout,
        ]);
        Self { actions, focused: 0, armed: false, volume: system::volume(), app_title, app_in_front, profile }
    }

    pub fn focused(&self) -> usize {
        self.focused
    }

    pub fn title(&self) -> String {
        match &self.app_title {
            Some(title) if self.app_in_front => trf("CONTROL CENTER · {}", &[&title.to_uppercase()]),
            _ => tr("CONTROL CENTER").into(),
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
                    QuickAction::Resume(_) => ("▶", tr("Resume"), self.app_title.clone().unwrap_or_default()),
                    QuickAction::Home => ("⌂", tr("Home"), tr("Go to Home").to_string()),
                    QuickAction::Quit(_) => ("■", tr("Quit"), self.app_title.clone().unwrap_or_default()),
                    QuickAction::SwitchProfile => ("☺", tr("Switch profile"), self.profile.clone()),
                    QuickAction::Volume => ("♪", tr("Volume"), self.volume_text()),
                    QuickAction::Power(PowerAction::Suspend) => ("☾", tr("Sleep"), String::new()),
                    QuickAction::Power(PowerAction::Reboot) => ("↻", tr("Restart"), String::new()),
                    QuickAction::Power(PowerAction::PowerOff) => ("⏻", tr("Power off"), String::new()),
                    QuickAction::Logout => ("⇥", tr("Sign out"), tr("Quit shell").to_string()),
                };
                // While the action waits for confirmation, its name moves to the second line.
                let (label, value) = if armed { (tr("Really?"), label.to_string()) } else { (label, value) };
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
            Some(Volume { muted: true, .. }) => tr("Muted").into(),
            Some(volume) => format!("{} %", volume.percent),
            None => tr("not available").into(),
        }
    }
}

fn report(result: std::io::Result<()>) {
    if let Err(err) = result {
        eprintln!("Quick menu: {err}");
    }
}
