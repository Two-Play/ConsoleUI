//! The window switcher in the quick menu: a row with every running app, most recently used first.
//! An app with several windows gets one card per window. The compositor adds the windows and their
//! previews; without it (gamescope, desktop) there is one card per app.

use crate::Action;
use crate::protocol::WindowInfo;

/// A running app, as the shell knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitcherApp {
    pub app_id: u32,
    pub entry_id: String,
    pub title: String,
    pub in_front: bool,
}

/// A card of the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitcherEntry {
    pub app_id: u32,
    pub entry_id: String,
    /// `None` while the compositor has not reported the app's windows
    pub window: Option<u64>,
    pub title: String,
    /// Title of the window, for apps with several windows
    pub detail: String,
    /// PNG from the compositor
    pub preview: Option<String>,
    /// What is visible right now
    pub in_front: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitcherOutcome {
    Stay,
    /// Down: to the quick menu's other cards
    Leave,
    Close,
    Switch {
        app_id: u32,
        window: Option<u64>,
    },
    /// Quit the app with this entry ID
    Quit(String),
}

#[derive(Debug, Clone, Default)]
pub struct Switcher {
    apps: Vec<SwitcherApp>,
    entries: Vec<SwitcherEntry>,
    focused: usize,
    /// The user moved the focus, so a new window list must not move it to the default
    moved: bool,
}

impl Switcher {
    /// `apps` in the order of the row, most recently used first.
    pub fn new(apps: Vec<SwitcherApp>) -> Self {
        let mut switcher = Self { apps, ..Self::default() };
        switcher.set_windows(&[]);
        switcher
    }

    /// Windows from the compositor, the top one first. Windows of apps the shell does not know are
    /// left out; an app without a window keeps its card (it is still starting).
    pub fn set_windows(&mut self, windows: &[WindowInfo]) {
        let current = self.entries.get(self.focused).map(|entry| (entry.app_id, entry.window));
        self.entries = self.apps.iter().flat_map(|app| Self::app_entries(app, windows)).collect();
        self.focused = match current.filter(|_| self.moved) {
            Some((app_id, window)) => self
                .entries
                .iter()
                .position(|entry| entry.app_id == app_id && entry.window == window)
                .or_else(|| self.entries.iter().position(|entry| entry.app_id == app_id))
                .unwrap_or(0),
            // Like Alt+Tab: ✕ right after opening goes to the previous app.
            None => self.entries.iter().position(|entry| !entry.in_front).unwrap_or(0),
        };
    }

    fn app_entries(app: &SwitcherApp, windows: &[WindowInfo]) -> Vec<SwitcherEntry> {
        let entry = |window: Option<&WindowInfo>, detail: String, in_front: bool| SwitcherEntry {
            app_id: app.app_id,
            entry_id: app.entry_id.clone(),
            window: window.map(|window| window.id),
            title: app.title.clone(),
            detail,
            preview: window.and_then(|window| window.preview.clone()),
            in_front,
        };
        let own: Vec<&WindowInfo> = windows.iter().filter(|window| window.app_id == app.app_id).collect();
        if own.is_empty() {
            return vec![entry(None, String::new(), app.in_front)];
        }
        let several = own.len() > 1;
        own.iter()
            .enumerate()
            .map(|(index, window)| {
                let detail = if several { window.title.clone() } else { String::new() };
                entry(Some(window), detail, app.in_front && index == 0)
            })
            .collect()
    }

    /// With a single card there is nothing to switch to: the quick menu's "Resume" does it.
    pub fn is_useful(&self) -> bool {
        self.entries.len() > 1
    }

    pub fn entries(&self) -> &[SwitcherEntry] {
        &self.entries
    }

    pub fn focused(&self) -> usize {
        self.focused
    }

    /// Focuses card `index` (a click), if there is one.
    pub fn focus(&mut self, index: usize) {
        if index < self.entries.len() {
            self.focused = index;
            self.moved = true;
        }
    }

    pub fn apply(&mut self, action: Action) -> SwitcherOutcome {
        let Some(current) = self.entries.get(self.focused) else { return SwitcherOutcome::Close };
        match action {
            Action::Left | Action::Right => {
                let next = match action {
                    Action::Left => self.focused.checked_sub(1),
                    _ => Some(self.focused + 1).filter(|&next| next < self.entries.len()),
                };
                if let Some(next) = next {
                    self.focused = next;
                    self.moved = true;
                }
                SwitcherOutcome::Stay
            }
            Action::Down => SwitcherOutcome::Leave,
            Action::Confirm => SwitcherOutcome::Switch { app_id: current.app_id, window: current.window },
            Action::Secondary => SwitcherOutcome::Quit(current.entry_id.clone()),
            Action::Back | Action::Guide => SwitcherOutcome::Close,
            _ => SwitcherOutcome::Stay,
        }
    }
}

/// Most recently used first; `last_used` gives the higher number to the app that was in front
/// later. Apps with the same number keep their order, newest launch first.
pub fn most_recent_first<T>(mut apps: Vec<T>, last_used: impl Fn(&T) -> u64) -> Vec<T> {
    apps.reverse();
    apps.sort_by_key(|app| std::cmp::Reverse(last_used(app)));
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(app_id: u32, in_front: bool) -> SwitcherApp {
        SwitcherApp { app_id, entry_id: format!("entry:{app_id}"), title: format!("App {app_id}"), in_front }
    }

    fn window(id: u64, app_id: u32) -> WindowInfo {
        WindowInfo { id, app_id, title: format!("Window {id}"), preview: Some(format!("/run/{id}.png")) }
    }

    fn cards(switcher: &Switcher) -> Vec<(u32, Option<u64>)> {
        switcher.entries().iter().map(|entry| (entry.app_id, entry.window)).collect()
    }

    #[test]
    fn opens_on_the_previous_app_and_switches_to_it() {
        let mut switcher = Switcher::new(vec![app(1, true), app(2, false), app(3, false)]);
        assert_eq!(switcher.focused(), 1);
        assert_eq!(switcher.apply(Action::Confirm), SwitcherOutcome::Switch { app_id: 2, window: None });
    }

    #[test]
    fn opens_on_the_first_app_with_the_shell_in_front() {
        assert_eq!(Switcher::new(vec![app(1, false), app(2, false)]).focused(), 0);
    }

    #[test]
    fn one_app_with_one_window_has_nothing_to_switch_to() {
        let mut switcher = Switcher::new(vec![app(1, true)]);
        assert!(!switcher.is_useful());
        switcher.set_windows(&[window(10, 1)]);
        assert!(!switcher.is_useful());
        switcher.set_windows(&[window(10, 1), window(11, 1)]);
        assert!(switcher.is_useful());
    }

    #[test]
    fn several_windows_of_an_app_get_their_own_cards() {
        let mut switcher = Switcher::new(vec![app(1, true), app(2, false)]);
        switcher.set_windows(&[window(11, 1), window(10, 1), window(20, 2), window(30, 99)]);
        assert_eq!(cards(&switcher), [(1, Some(11)), (1, Some(10)), (2, Some(20))]);
        // Only the top window is what is visible; the other one is a place to go to.
        assert!(switcher.entries()[0].in_front);
        assert!(!switcher.entries()[1].in_front);
        assert_eq!(switcher.focused(), 1);
        assert_eq!(switcher.entries()[1].detail, "Window 10");
        assert_eq!(switcher.entries()[2].detail, "");
        assert_eq!(switcher.entries()[2].preview.as_deref(), Some("/run/20.png"));
    }

    #[test]
    fn an_app_without_a_window_keeps_its_card() {
        let mut switcher = Switcher::new(vec![app(1, true), app(2, false)]);
        switcher.set_windows(&[window(10, 1)]);
        assert_eq!(cards(&switcher), [(1, Some(10)), (2, None)]);
    }

    #[test]
    fn the_window_list_keeps_a_focus_the_user_chose() {
        let mut switcher = Switcher::new(vec![app(1, true), app(2, false), app(3, false)]);
        switcher.apply(Action::Right);
        assert_eq!(switcher.focused(), 2);
        switcher.set_windows(&[window(10, 1), window(11, 1), window(20, 2), window(30, 3)]);
        assert_eq!(switcher.entries()[switcher.focused()].app_id, 3);
    }

    #[test]
    fn stops_at_the_edges_and_leaves_downwards() {
        let mut switcher = Switcher::new(vec![app(1, false), app(2, false)]);
        switcher.apply(Action::Left);
        assert_eq!(switcher.focused(), 0);
        switcher.apply(Action::Right);
        switcher.apply(Action::Right);
        assert_eq!(switcher.focused(), 1);
        assert_eq!(switcher.apply(Action::Down), SwitcherOutcome::Leave);
        assert_eq!(switcher.apply(Action::Up), SwitcherOutcome::Stay);
        assert_eq!(switcher.apply(Action::Back), SwitcherOutcome::Close);
    }

    #[test]
    fn square_quits_the_focused_app() {
        let mut switcher = Switcher::new(vec![app(1, true), app(2, false)]);
        assert_eq!(switcher.apply(Action::Secondary), SwitcherOutcome::Quit("entry:2".into()));
    }

    #[test]
    fn recently_used_apps_come_first_then_the_newest_launch() {
        let order = most_recent_first(vec![(1, 0), (2, 5), (3, 0), (4, 9)], |app| app.1);
        assert_eq!(order, [(4, 9), (2, 5), (3, 0), (1, 0)]);
    }
}
