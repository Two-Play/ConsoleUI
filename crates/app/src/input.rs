//! Input of an app: controller events and keyboard presses end up as the same `Action`s the shell
//! uses, and the button glyphs of the hints follow the device used last.
//!
//! The keyboard is read by the app's own Slint `InputScope` (`@consoleui/input.slint`), which
//! emits `NavAction`s; the app maps them with `slint_bridge!`. Controllers are read here with
//! gilrs, but only while the app's window has the focus: the shell keeps reading them too (for the
//! Guide button and its quick menu), so an app must not react while the quick menu is open.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use consoleui_core::Action;
use consoleui_core::input::{GlyphStyle, InputDevice, hint_glyphs};
use consoleui_core::settings::GlyphChoice;
use slint::{Timer, TimerMode};

use crate::gamepad::{Connection, Gamepads};

/// Poll interval for controllers (gilrs delivers events only on request).
const POLL: Duration = Duration::from_millis(8);
/// How often the controller type is checked (it changes rarely).
const STATUS_EVERY: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub enum Event {
    /// A controller press (or repeat) for the focused window; never `Guide`
    Action(Action),
    /// A controller was plugged in or removed, e.g. for a toast
    Controller(Connection),
    /// The glyphs of the hints changed: `None` shows keyboard keys
    Glyphs(Option<GlyphStyle>),
}

struct State {
    gamepads: Option<Gamepads>,
    controller: Option<GlyphStyle>,
    choice: GlyphChoice,
    last: InputDevice,
    shown: Option<Option<GlyphStyle>>,
    last_status: Option<Instant>,
    handler: Box<dyn FnMut(Event)>,
}

impl State {
    fn glyphs(&mut self) {
        let glyphs = hint_glyphs(self.choice, self.controller, self.last);
        if self.shown != Some(glyphs) {
            self.shown = Some(glyphs);
            (self.handler)(Event::Glyphs(glyphs));
        }
    }
}

/// Polls the controllers on a Slint timer and reports `Event`s. Keep it alive as long as the app
/// runs; dropping it stops the polling.
pub struct Input {
    state: Rc<RefCell<State>>,
    _timer: Timer,
}

impl Input {
    /// `active` says whether the app's window has the focus (`InputScope.active`); controller
    /// presses are dropped while it is false.
    pub fn start(active: impl Fn() -> bool + 'static, handler: impl FnMut(Event) + 'static) -> Self {
        let state = Rc::new(RefCell::new(State {
            gamepads: Gamepads::new(),
            controller: None,
            choice: GlyphChoice::Auto,
            last: InputDevice::Keyboard,
            shown: None,
            last_status: None,
            handler: Box::new(handler),
        }));
        let timer = Timer::default();
        let weak = Rc::downgrade(&state);
        timer.start(TimerMode::Repeated, POLL, move || {
            let Some(state) = weak.upgrade() else { return };
            let now = Instant::now();
            let active = active();
            let mut state = state.borrow_mut();
            let Some(gamepads) = state.gamepads.as_mut() else { return };
            let actions = gamepads.poll(now);
            let connections = gamepads.take_connections();
            let refresh = state.last_status.is_none_or(|at| now - at >= STATUS_EVERY) || !connections.is_empty();
            if refresh {
                state.last_status = Some(now);
                state.controller = state.gamepads.as_ref().and_then(Gamepads::status).map(|s| s.style);
            }
            for connection in connections {
                (state.handler)(Event::Controller(connection));
            }
            for action in actions {
                // The Guide button belongs to the shell (quick menu)
                if active && action != Action::Guide {
                    state.last = InputDevice::Gamepad;
                    (state.handler)(Event::Action(action));
                }
            }
            state.glyphs();
        });
        Self { state, _timer: timer }
    }

    /// The user pressed a key: the hints show keyboard keys until a controller is used again.
    pub fn keyboard_used(&self) {
        let mut state = self.state.borrow_mut();
        state.last = InputDevice::Keyboard;
        state.glyphs();
    }

    /// The "button glyphs" setting (Auto, PlayStation, Xbox).
    pub fn set_glyph_choice(&self, choice: GlyphChoice) {
        let mut state = self.state.borrow_mut();
        state.choice = choice;
        state.glyphs();
    }
}
