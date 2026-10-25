//! Window policy without Smithay types: who owns a window, what is in front, the stacking order
//! and the keyboard focus. The compositor feeds it window and control-socket events and applies
//! the result to Smithay's `Space` (see `State::apply_policy`).
//!
//! Rules (a console, not a desktop):
//! - Every window belongs to the shell, to a launched app, or is unknown.
//! - The shell asks for a [`Target`]. An app comes to the front only once one of its windows
//!   shows content; until then the shell (with its launch screen) stays visible.
//! - While the overlay (quick menu) is open, the shell is on top and has the keyboard.
//! - Popups (X11 override-redirect windows) are drawn directly above their owner's windows and
//!   never get the keyboard focus.

use consoleui_core::ownership::{self, Owner, ProcessInfo};
use consoleui_core::protocol::{CompositorMessage, Target};

pub type WindowId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Shell,
    App(u32),
    Unknown,
}

#[derive(Debug, Clone)]
struct Window {
    id: WindowId,
    pid: Option<u32>,
    role: Role,
    popup: bool,
    /// Has shown content (a buffer was committed)
    visible: bool,
    title: String,
}

#[derive(Debug, Clone)]
struct App {
    app_id: u32,
    owner: Option<Owner>,
}

#[derive(Debug, Default)]
pub struct Policy {
    /// In map order (oldest first)
    windows: Vec<Window>,
    /// In launch order (oldest first)
    apps: Vec<App>,
    /// The process that runs the shell UI (from `Hello`, else the spawned child)
    shell_pid: Option<u32>,
    target: Option<Target>,
    overlay: bool,
}

impl Policy {
    pub fn set_shell_pid(&mut self, pid: u32, lookup: &impl Fn(u32) -> Option<ProcessInfo>) -> Vec<CompositorMessage> {
        self.change(|policy| {
            policy.shell_pid = Some(pid);
            policy.reclassify(lookup);
        })
    }

    pub fn launched(
        &mut self,
        app_id: u32,
        owner: Option<Owner>,
        lookup: &impl Fn(u32) -> Option<ProcessInfo>,
    ) -> Vec<CompositorMessage> {
        self.change(|policy| {
            policy.apps.retain(|app| app.app_id != app_id);
            policy.apps.push(App { app_id, owner });
            policy.reclassify(lookup);
        })
    }

    pub fn exited(&mut self, app_id: u32) -> Vec<CompositorMessage> {
        self.change(|policy| policy.apps.retain(|app| app.app_id != app_id))
    }

    pub fn set_target(&mut self, target: Target) -> Vec<CompositorMessage> {
        self.change(|policy| policy.target = Some(target))
    }

    pub fn set_overlay(&mut self, open: bool) -> Vec<CompositorMessage> {
        self.change(|policy| policy.overlay = open)
    }

    /// A new window (not visible until [`Policy::show`]). `pid` is the process that owns it.
    pub fn add_window(
        &mut self,
        id: WindowId,
        pid: Option<u32>,
        popup: bool,
        title: &str,
        lookup: &impl Fn(u32) -> Option<ProcessInfo>,
    ) -> Vec<CompositorMessage> {
        self.change(|policy| {
            let role = policy.classify(pid, lookup);
            policy.windows.retain(|w| w.id != id);
            policy.windows.push(Window { id, pid, role, popup, visible: false, title: title.to_owned() });
            policy.assign_fallback();
        })
    }

    /// The window committed its first buffer.
    pub fn show(&mut self, id: WindowId) -> Vec<CompositorMessage> {
        let Some(window) = self.windows.iter().find(|w| w.id == id) else { return Vec::new() };
        if window.visible {
            return Vec::new();
        }
        tracing::info!("window {id} ({:?}, {:?}) shows content", window.role, window.title);
        let mut events = Vec::new();
        if let (Role::App(app_id), false) = (window.role, window.popup) {
            events.push(CompositorMessage::WindowMapped { app_id, title: window.title.clone() });
        }
        events.extend(self.change(|policy| {
            if let Some(window) = policy.window_mut(id) {
                window.visible = true;
            }
        }));
        events
    }

    pub fn set_title(&mut self, id: WindowId, title: &str) {
        if let Some(window) = self.window_mut(id) {
            title.clone_into(&mut window.title);
        }
    }

    pub fn remove_window(&mut self, id: WindowId) -> Vec<CompositorMessage> {
        let Some(pos) = self.windows.iter().position(|w| w.id == id) else { return Vec::new() };
        let window = self.windows.remove(pos);
        let mut events = Vec::new();
        if let (Role::App(app_id), false, true) = (window.role, window.popup, window.visible)
            && !self.windows.iter().any(|w| w.role == Role::App(app_id) && !w.popup && w.visible)
        {
            events.push(CompositorMessage::AppGone { app_id });
        }
        // `change` compares the front before and after; the window is already gone, so compute
        // the old front with it still present.
        self.windows.insert(pos, window);
        events.extend(self.change(|policy| {
            policy.windows.remove(pos);
        }));
        events
    }

    pub fn role(&self, id: WindowId) -> Option<Role> {
        self.windows.iter().find(|w| w.id == id).map(|w| w.role)
    }

    /// What is actually visible in front.
    pub fn front(&self) -> Target {
        if self.overlay {
            return Target::Shell;
        }
        match self.target {
            Some(Target::App { app_id }) if self.has_content(Role::App(app_id)) => Target::App { app_id },
            _ => Target::Shell,
        }
    }

    /// Window IDs from bottom to top.
    pub fn stacking(&self) -> Vec<WindowId> {
        let front = self.front();
        let mut groups: Vec<Role> = vec![Role::Unknown];
        groups.extend(self.apps.iter().map(|a| Role::App(a.app_id)).filter(|r| target_of(*r) != Some(front)));
        // Windows of apps the shell no longer knows (e.g. still closing after the exit)
        let mut stale: Vec<Role> = Vec::new();
        for window in &self.windows {
            if let Role::App(app_id) = window.role
                && !self.apps.iter().any(|a| a.app_id == app_id)
                && !stale.contains(&window.role)
                && target_of(window.role) != Some(front)
            {
                stale.push(window.role);
            }
        }
        groups.splice(1..1, stale);
        groups.push(Role::Shell);
        if let Target::App { app_id } = front {
            groups.push(Role::App(app_id));
        }

        let mut order = Vec::new();
        for role in &groups {
            let windows = self.windows.iter().filter(|w| w.role == *role);
            // Unknown popups are handled below; everything else: normal windows, then popups.
            let popups = |w: &&Window| w.popup;
            if *role == Role::Unknown {
                order.extend(windows.filter(|w| !w.popup).map(|w| w.id));
                continue;
            }
            let (popup, normal): (Vec<&Window>, Vec<&Window>) = windows.partition(popups);
            order.extend(normal.iter().map(|w| w.id));
            order.extend(popup.iter().map(|w| w.id));
        }
        // Popups nobody claims (e.g. a game's menu without _NET_WM_PID): above an app in front,
        // but below the shell while the shell is in front.
        let unknown_popups = self.windows.iter().filter(|w| w.role == Role::Unknown && w.popup).map(|w| w.id);
        if front == Target::Shell {
            let shell_start = order.iter().position(|id| self.role(*id) == Some(Role::Shell)).unwrap_or(order.len());
            let popups: Vec<WindowId> = unknown_popups.collect();
            order.splice(shell_start..shell_start, popups);
        } else {
            order.extend(unknown_popups);
        }
        order
    }

    /// The window that should have the keyboard: the newest visible normal window in front.
    pub fn keyboard_focus(&self) -> Option<WindowId> {
        let role = match self.front() {
            Target::Shell => Role::Shell,
            Target::App { app_id } => Role::App(app_id),
        };
        self.windows.iter().rev().find(|w| w.role == role && !w.popup && w.visible).map(|w| w.id)
    }

    fn has_content(&self, role: Role) -> bool {
        self.windows.iter().any(|w| w.role == role && !w.popup && w.visible)
    }

    fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// Runs a mutation and reports a changed front as `FocusChanged`, plus `WindowMapped` for
    /// visible windows that only now got an app.
    fn change(&mut self, mutate: impl FnOnce(&mut Self)) -> Vec<CompositorMessage> {
        let front = self.front();
        let roles: Vec<(WindowId, Role)> = self.windows.iter().map(|w| (w.id, w.role)).collect();
        mutate(self);
        let mut events = Vec::new();
        for window in &self.windows {
            let was = roles.iter().find(|(id, _)| *id == window.id).map(|(_, role)| *role);
            if let (Role::App(app_id), true, false) = (window.role, window.visible, window.popup)
                && was.is_some()
                && was != Some(window.role)
            {
                events.push(CompositorMessage::WindowMapped { app_id, title: window.title.clone() });
            }
        }
        let now = self.front();
        if now != front {
            events.push(CompositorMessage::FocusChanged { target: now });
        }
        events
    }

    fn classify(&self, pid: Option<u32>, lookup: &impl Fn(u32) -> Option<ProcessInfo>) -> Role {
        let Some(pid) = pid.filter(|&pid| pid != 0) else { return Role::Unknown };
        if Some(pid) == self.shell_pid {
            return Role::Shell;
        }
        let apps: Vec<(u32, Owner)> =
            self.apps.iter().filter_map(|a| a.owner.clone().map(|owner| (a.app_id, owner))).collect();
        match ownership::owner_of(pid, &apps, lookup) {
            Some(app_id) => Role::App(app_id),
            None => Role::Unknown,
        }
    }

    /// Re-checks unknown windows after the shell PID or a new app became known.
    fn reclassify(&mut self, lookup: &impl Fn(u32) -> Option<ProcessInfo>) {
        let updates: Vec<(WindowId, Role)> = self
            .windows
            .iter()
            .filter(|w| w.role == Role::Unknown)
            .map(|w| (w.id, self.classify(w.pid, lookup)))
            .collect();
        for (id, role) in updates {
            if let Some(window) = self.window_mut(id) {
                window.role = role;
            }
        }
        self.assign_fallback();
    }

    /// Normal windows nobody claims (no PID, or a process outside every launched app, e.g. Steam
    /// on its first start) belong to the newest launched app that has no window yet, like in the
    /// gamescope session.
    fn assign_fallback(&mut self) {
        loop {
            let Some(app_id) = self
                .apps
                .iter()
                .rev()
                .find(|a| !self.windows.iter().any(|w| w.role == Role::App(a.app_id)))
                .map(|a| a.app_id)
            else {
                return;
            };
            let Some(window) = self.windows.iter_mut().find(|w| w.role == Role::Unknown && !w.popup) else { return };
            tracing::info!(
                "window {} ({:?}) has no known owner, assigning it to app {app_id}",
                window.id,
                window.title
            );
            window.role = Role::App(app_id);
        }
    }
}

fn target_of(role: Role) -> Option<Target> {
    match role {
        Role::Shell => Some(Target::Shell),
        Role::App(app_id) => Some(Target::App { app_id }),
        Role::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const SHELL: u32 = 100;

    fn processes() -> impl Fn(u32) -> Option<ProcessInfo> {
        // 100: shell, 200: launched app (child of the shell), 201: its child, 300: unrelated
        let table: HashMap<u32, ProcessInfo> = [
            (100, ProcessInfo { parent: None, ..Default::default() }),
            (200, ProcessInfo { parent: Some(100), ..Default::default() }),
            (201, ProcessInfo { parent: Some(200), ..Default::default() }),
            (300, ProcessInfo { parent: None, ..Default::default() }),
        ]
        .into_iter()
        .collect();
        move |pid| table.get(&pid).cloned()
    }

    fn policy() -> Policy {
        let mut policy = Policy::default();
        policy.set_shell_pid(SHELL, &processes());
        policy.add_window(1, Some(SHELL), false, "ConsoleUI", &processes());
        policy.show(1);
        policy
    }

    #[test]
    fn app_comes_to_front_only_with_content() {
        let lookup = processes();
        let mut policy = policy();
        assert!(policy.launched(7, Some(Owner::Process(200)), &lookup).is_empty());
        assert!(policy.set_target(Target::App { app_id: 7 }).is_empty(), "no window yet: shell stays");
        assert_eq!(policy.front(), Target::Shell);

        assert!(policy.add_window(2, Some(201), false, "game", &lookup).is_empty());
        assert_eq!(policy.role(2), Some(Role::App(7)));
        assert_eq!(policy.keyboard_focus(), Some(1));

        let events = policy.show(2);
        assert_eq!(
            events,
            vec![
                CompositorMessage::WindowMapped { app_id: 7, title: "game".into() },
                CompositorMessage::FocusChanged { target: Target::App { app_id: 7 } },
            ]
        );
        assert_eq!(policy.stacking(), vec![1, 2]);
        assert_eq!(policy.keyboard_focus(), Some(2));
    }

    #[test]
    fn back_to_shell_when_the_window_closes() {
        let lookup = processes();
        let mut policy = policy();
        policy.launched(7, Some(Owner::Process(200)), &lookup);
        policy.set_target(Target::App { app_id: 7 });
        policy.add_window(2, Some(200), false, "game", &lookup);
        policy.show(2);

        let events = policy.remove_window(2);
        assert_eq!(
            events,
            vec![CompositorMessage::AppGone { app_id: 7 }, CompositorMessage::FocusChanged { target: Target::Shell }]
        );
        assert_eq!(policy.keyboard_focus(), Some(1));
    }

    #[test]
    fn overlay_puts_the_shell_on_top() {
        let lookup = processes();
        let mut policy = policy();
        policy.launched(7, Some(Owner::Process(200)), &lookup);
        policy.set_target(Target::App { app_id: 7 });
        policy.add_window(2, Some(200), false, "game", &lookup);
        policy.show(2);

        assert_eq!(policy.set_overlay(true), vec![CompositorMessage::FocusChanged { target: Target::Shell }]);
        assert_eq!(policy.stacking(), vec![2, 1]);
        assert_eq!(policy.keyboard_focus(), Some(1));
        policy.set_overlay(false);
        assert_eq!(policy.stacking(), vec![1, 2]);

        // "Home" from the quick menu
        assert_eq!(policy.set_target(Target::Shell), vec![CompositorMessage::FocusChanged { target: Target::Shell }]);
        assert_eq!(policy.stacking(), vec![2, 1]);
    }

    #[test]
    fn windows_without_pid_go_to_the_newest_app_without_window() {
        let lookup = processes();
        let mut policy = policy();
        policy.launched(7, Some(Owner::Process(200)), &lookup);
        policy.add_window(2, None, false, "glxgears", &lookup);
        assert_eq!(policy.role(2), Some(Role::App(7)));
        // A second unknown window stays unknown: app 7 already has one.
        policy.add_window(3, Some(300), false, "stranger", &lookup);
        assert_eq!(policy.role(3), Some(Role::Unknown));
        // Unknown windows are at the bottom.
        policy.show(3);
        assert_eq!(policy.stacking()[0], 3);
    }

    #[test]
    fn early_windows_are_claimed_once_the_app_is_known() {
        let lookup = processes();
        let mut policy = Policy::default();
        // The shell's window maps before its Hello arrives.
        policy.add_window(1, Some(SHELL), false, "ConsoleUI", &lookup);
        policy.show(1);
        assert_eq!(policy.role(1), Some(Role::Unknown));
        policy.set_shell_pid(SHELL, &lookup);
        assert_eq!(policy.role(1), Some(Role::Shell));

        policy.add_window(2, Some(201), false, "game", &lookup);
        policy.show(2);
        let events = policy.launched(7, Some(Owner::Process(200)), &lookup);
        assert_eq!(events, vec![CompositorMessage::WindowMapped { app_id: 7, title: "game".into() }]);
        assert_eq!(policy.role(2), Some(Role::App(7)));
    }

    #[test]
    fn popups_stay_with_their_owner_and_never_take_the_keyboard() {
        let lookup = processes();
        let mut policy = policy();
        policy.launched(7, Some(Owner::Process(200)), &lookup);
        policy.set_target(Target::App { app_id: 7 });
        policy.add_window(2, Some(200), false, "game", &lookup);
        policy.show(2);
        policy.add_window(3, Some(200), true, "menu", &lookup);
        policy.show(3);
        policy.add_window(4, None, true, "tooltip", &lookup);
        policy.show(4);
        assert_eq!(policy.role(4), Some(Role::Unknown), "popups are not assigned by the fallback");
        assert_eq!(policy.stacking(), vec![1, 2, 3, 4]);
        assert_eq!(policy.keyboard_focus(), Some(2));

        policy.set_target(Target::Shell);
        assert_eq!(policy.stacking(), vec![2, 3, 4, 1]);
    }

    #[test]
    fn exited_apps_lose_the_front() {
        let lookup = processes();
        let mut policy = policy();
        policy.launched(7, Some(Owner::Process(200)), &lookup);
        policy.launched(8, Some(Owner::Process(300)), &lookup);
        policy.set_target(Target::App { app_id: 8 });
        policy.add_window(2, Some(200), false, "first", &lookup);
        policy.add_window(3, Some(300), false, "second", &lookup);
        policy.show(2);
        policy.show(3);
        assert_eq!(policy.stacking(), vec![2, 1, 3]);
        policy.exited(7);
        // Window 2 still exists until it unmaps, behind the shell.
        assert_eq!(policy.stacking(), vec![2, 1, 3]);
    }
}
