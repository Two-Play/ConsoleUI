//! Content and state of the quick menu (PS/Guide button).

use consoleui_core::i18n::{tr, trf};
use consoleui_core::protocol::WindowInfo;
use consoleui_core::switcher::{Switcher, SwitcherEntry, SwitcherOutcome};
use consoleui_core::system::{self, Backlight, PowerAction, Volume};

use crate::QuickItem;

#[derive(Debug, Clone, PartialEq)]
pub enum QuickAction {
    /// Back to the running app (gamescope app ID)
    Resume(u32),
    /// From the window switcher: to this app, and this window of it if it is known
    Switch {
        app_id: u32,
        window: Option<u64>,
    },
    Home,
    /// Quit the running app (entry ID)
    Quit(String),
    /// The gamepad as a mouse for the app in front (Story 7.15)
    Mouse(u32),
    /// Open the profile screen (Story 5.7)
    SwitchProfile,
    Volume,
    /// Only on screens with a backlight (Story 4.6)
    Brightness,
    Power(PowerAction),
    Logout,
}

impl QuickAction {
    /// Actions with serious consequences are confirmed in a dialog.
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
    volume: Option<Volume>,
    brightness: Option<Backlight>,
    app_title: Option<String>,
    app_in_front: bool,
    /// Name of the active profile
    profile: String,
    /// The row of running apps above the cards
    switcher: Switcher,
    /// The focus is in that row
    in_switcher: bool,
}

/// What should happen after an input in the quick menu
pub enum QuickOutcome {
    Stay,
    Close,
    Run(QuickAction),
    /// The action has serious consequences: the shell asks first (dialog)
    Confirm(QuickAction),
}

impl QuickMenu {
    pub fn new(foreground: Option<ForegroundApp>, profile: String, switcher: Switcher, mouse: bool) -> Self {
        let mut actions = Vec::new();
        let app_title = foreground.as_ref().map(|app| app.title.clone());
        let app_in_front = foreground.as_ref().is_some_and(|app| app.in_front);
        // In front the menu closes back into the app with ○, so "Resume" would only repeat that:
        // there the app offers the way home. With the shell in front (home) it is the way back.
        if let Some(app) = foreground {
            actions.push(if app.in_front { QuickAction::Home } else { QuickAction::Resume(app.app_id) });
            actions.push(QuickAction::Quit(app.entry_id));
            actions.extend((mouse && app.in_front).then_some(QuickAction::Mouse(app.app_id)));
        }
        let brightness = system::backlight();
        actions.extend([QuickAction::SwitchProfile, QuickAction::Volume]);
        actions.extend(brightness.is_some().then_some(QuickAction::Brightness));
        actions.extend([
            QuickAction::Power(PowerAction::Suspend),
            QuickAction::Power(PowerAction::Reboot),
            QuickAction::Power(PowerAction::PowerOff),
            QuickAction::Logout,
        ]);
        let in_switcher = switcher.is_useful();
        Self {
            actions,
            focused: 0,
            volume: system::volume(),
            brightness,
            app_title,
            app_in_front,
            profile,
            switcher,
            in_switcher,
        }
    }

    /// The focused card, -1 while the focus is in the switcher row.
    pub fn focused(&self) -> i32 {
        if self.in_switcher { -1 } else { self.focused as i32 }
    }

    /// The focused card of the switcher row, -1 while the focus is elsewhere.
    pub fn switcher_focused(&self) -> i32 {
        if self.in_switcher { self.switcher.focused() as i32 } else { -1 }
    }

    /// Changes with every move of the focus, in whichever row (for the sounds).
    pub fn focus_key(&self) -> usize {
        if self.in_switcher { self.actions.len() + self.switcher.focused() } else { self.focused }
    }

    /// Focuses card `index` (a click), if there is one.
    pub fn focus(&mut self, index: usize) {
        if index < self.actions.len() {
            self.focused = index;
            self.in_switcher = false;
        }
    }

    /// Focuses card `index` of the switcher row (a click), if there is one.
    pub fn focus_switcher(&mut self, index: usize) {
        if index < self.switcher.entries().len() {
            self.switcher.focus(index);
            self.in_switcher = true;
        }
    }

    /// The cards of the switcher row; empty when there is nothing to switch to.
    pub fn switcher_entries(&self) -> &[SwitcherEntry] {
        if self.switcher.is_useful() { self.switcher.entries() } else { &[] }
    }

    /// The compositor reported the windows. A second window of a single app may only now make the
    /// row useful, then the focus starts there like with several apps.
    pub fn set_windows(&mut self, windows: &[WindowInfo]) {
        let useful = self.switcher.is_useful();
        self.switcher.set_windows(windows);
        if self.switcher.is_useful() != useful {
            self.in_switcher = !useful;
        }
    }

    pub fn title(&self) -> String {
        match &self.app_title {
            Some(title) if self.app_in_front => trf("CONTROL CENTER · {}", &[&title.to_uppercase()]),
            _ => tr("CONTROL CENTER").into(),
        }
    }

    pub fn apply(&mut self, action: consoleui_core::Action) -> QuickOutcome {
        use consoleui_core::Action;
        // L1/R1 reach the row from every card; up/down change the volume on some of them.
        if matches!(action, Action::PrevTab | Action::NextTab) && self.switcher.is_useful() {
            self.in_switcher = !self.in_switcher;
            return QuickOutcome::Stay;
        }
        if self.in_switcher {
            return match self.switcher.apply(action) {
                SwitcherOutcome::Stay => QuickOutcome::Stay,
                SwitcherOutcome::Leave => {
                    self.in_switcher = false;
                    QuickOutcome::Stay
                }
                SwitcherOutcome::Close => QuickOutcome::Close,
                SwitcherOutcome::Switch { app_id, window } => QuickOutcome::Run(QuickAction::Switch { app_id, window }),
                SwitcherOutcome::Quit(entry_id) => QuickOutcome::Confirm(QuickAction::Quit(entry_id)),
            };
        }
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
                }
                QuickOutcome::Stay
            }
            Action::Up | Action::Down if current == QuickAction::Volume => {
                report(system::change_volume(action == Action::Up));
                self.volume = system::volume();
                QuickOutcome::Stay
            }
            Action::Up | Action::Down if current == QuickAction::Brightness => {
                report(system::change_brightness(action == Action::Up));
                self.brightness = system::backlight();
                QuickOutcome::Stay
            }
            Action::Up if self.switcher.is_useful() => {
                self.in_switcher = true;
                QuickOutcome::Stay
            }
            Action::Confirm if current == QuickAction::Brightness => QuickOutcome::Stay,
            Action::Confirm if current == QuickAction::Volume => {
                report(system::toggle_mute());
                self.volume = system::volume();
                QuickOutcome::Stay
            }
            Action::Confirm if current.needs_confirmation() => QuickOutcome::Confirm(current),
            Action::Confirm => QuickOutcome::Run(current),
            Action::Back | Action::Guide => QuickOutcome::Close,
            _ => QuickOutcome::Stay,
        }
    }

    pub fn items(&self) -> Vec<QuickItem> {
        self.actions
            .iter()
            .map(|action| {
                let (symbol, label, value) = match action {
                    QuickAction::Resume(_) => ("▶", tr("Resume"), self.app_title.clone().unwrap_or_default()),
                    // Only a card of the switcher row, never of this one
                    QuickAction::Switch { .. } => ("▶", tr("Resume"), String::new()),
                    QuickAction::Home => ("⌂", tr("Home"), tr("Go to Home").to_string()),
                    QuickAction::Quit(_) => ("■", tr("Quit"), self.app_title.clone().unwrap_or_default()),
                    QuickAction::Mouse(_) => ("◉", tr("Gamepad mouse"), tr("Use the app with the stick").to_string()),
                    QuickAction::SwitchProfile => ("☺", tr("Switch profile"), self.profile.clone()),
                    QuickAction::Volume => ("♪", tr("Volume"), self.volume_text()),
                    QuickAction::Brightness => ("☀", tr("Brightness"), self.brightness_text()),
                    QuickAction::Power(PowerAction::Suspend) => ("☾", tr("Sleep"), String::new()),
                    QuickAction::Power(PowerAction::Reboot) => ("↻", tr("Restart"), String::new()),
                    QuickAction::Power(PowerAction::PowerOff) => ("⏻", tr("Power off"), String::new()),
                    QuickAction::Logout => ("⇥", tr("Sign out"), tr("Quit shell").to_string()),
                };
                let level = match (action, self.volume) {
                    (QuickAction::Volume, Some(volume)) if !volume.muted => volume.percent.into(),
                    (QuickAction::Volume, Some(_)) => 0,
                    _ if *action == QuickAction::Brightness => {
                        self.brightness.as_ref().map_or(-1, |light| light.percent().into())
                    }
                    _ => -1,
                };
                QuickItem {
                    symbol: symbol.into(),
                    label: label.into(),
                    value: value.into(),
                    adjustable: matches!(action, QuickAction::Volume | QuickAction::Brightness),
                    can_mute: *action == QuickAction::Volume,
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

    fn brightness_text(&self) -> String {
        match &self.brightness {
            Some(light) => format!("{} %", light.percent()),
            None => tr("not available").into(),
        }
    }
}

fn report(result: std::io::Result<()>) {
    if let Err(err) = result {
        eprintln!("Quick menu: {err}");
    }
}
