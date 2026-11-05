//! The gamepad as a mouse (Story 7.15), for windows that are made for a mouse: the Steam client's
//! dialogs after "Show Steam", and any app through the quick menu. The shell reads the controller
//! exclusively (like in the quick menu) and sends the pointer to the compositor. ○ ends it, and so
//! does the app leaving the front or the Steam game starting.

use consoleui_core::pointer::{self, PointerCommand, StickPointer};

use super::*;

pub(super) struct MouseMode {
    pub(super) app_id: u32,
    pointer: StickPointer,
    last: Instant,
}

impl Shell {
    pub(super) fn offers_mouse_mode(&self) -> bool {
        self.compositor.as_ref().is_some_and(Compositor::virtual_pointer)
    }

    pub(super) fn start_mouse_mode(&mut self, ui: &AppWindow, app_id: u32) {
        if !self.offers_mouse_mode() || self.mouse_mode.is_some() {
            return;
        }
        self.mouse_mode = Some(MouseMode { app_id, pointer: StickPointer::default(), last: Instant::now() });
        if let Some(compositor) = &mut self.compositor {
            compositor.send(&ShellMessage::PointerMode { active: true });
        }
        self.toast(ui, "◉", tr("Gamepad mouse"), tr("Stick moves, ✕ clicks, □ right-clicks, ○ ends"));
    }

    pub(super) fn stop_mouse_mode(&mut self) {
        if self.mouse_mode.take().is_some()
            && let Some(compositor) = &mut self.compositor
        {
            compositor.send(&ShellMessage::PointerMode { active: false });
        }
    }

    /// The app it was started for is not in front any more.
    pub(super) fn check_mouse_mode(&mut self) {
        if self.mouse_mode.as_ref().is_some_and(|mode| self.foreground != Some(mode.app_id)) {
            self.stop_mouse_mode();
        }
    }

    pub(super) fn apply_mouse_mode(&mut self, action: Action) {
        let Some(command) = pointer::command(action) else { return };
        let message = match command {
            PointerCommand::Click(button) => ShellMessage::PointerClick { button },
            PointerCommand::Scroll(steps) => ShellMessage::PointerScroll { steps },
            PointerCommand::Leave => return self.stop_mouse_mode(),
        };
        if let Some(compositor) = &mut self.compositor {
            compositor.send(&message);
        }
    }

    /// The left stick, on every gamepad poll.
    pub fn mouse_mode_stick(&mut self, (x, y): (f32, f32), now: Instant) {
        if self.quick.is_some() {
            return;
        }
        let Some(mode) = &mut self.mouse_mode else { return };
        let seconds = now.duration_since(mode.last).as_secs_f64();
        mode.last = now;
        if let Some((dx, dy)) = mode.pointer.step(x, y, seconds) {
            self.move_pointer(dx, dy);
        }
    }

    /// Also the `pointer:DX:DY` step of `--actions`.
    pub fn move_pointer(&mut self, dx: i32, dy: i32) {
        if self.mouse_mode.is_some()
            && let Some(compositor) = &mut self.compositor
        {
            compositor.send(&ShellMessage::PointerMotion { dx, dy });
        }
    }
}
