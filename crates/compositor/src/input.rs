//! Keyboard and mouse input. The keyboard goes to the window the policy focuses (clicks do not
//! move it: on a console the shell decides what is in front); the pointer goes to the topmost
//! surface under it that the policy lets have pointer input (while the quick menu is open only
//! the shell's, Story 7.5). F1 (the shell's own Guide key), Super+Escape and the Home key
//! (`XF86HomePage`) are the keyboard's Guide button while a shell is connected: they go to the
//! shell as `Guide` over the control socket, never to the focused window (so games never see F1). Gamepads are not handled here: the shell reads them with gilrs (and grabs them while
//! the quick menu is open, `crates/shell/src/grab.rs`).
//!
//! Pointer handling with constraints follows anvil (`anvil/src/input_handler.rs`, MIT,
//! Copyright (c) 2017 Victor Berger).

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, Event, InputBackend, InputEvent, InputTime, KeyState, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::desktop::WindowSurfaceType;
use smithay::input::keyboard::{FilterResult, Keysym, ModifiersState, keysyms};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent};
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use smithay::wayland::pointer_constraints::{PointerConstraint, with_pointer_constraint};
use smithay::wayland::seat::WaylandFocus;
use tracing::info;

use consoleui_core::protocol::CompositorMessage;

use crate::focus::FocusTarget;
use crate::state::State;

enum KeyAction {
    Quit,
    VtSwitch(i32),
    Guide,
}

fn shortcut(modifiers: &ModifiersState, keysym: Keysym, shell_connected: bool) -> Option<KeyAction> {
    if modifiers.ctrl && modifiers.alt && keysym == Keysym::BackSpace {
        return Some(KeyAction::Quit);
    }
    let guide = matches!(keysym, Keysym::F1 | Keysym::XF86_HomePage) || (modifiers.logo && keysym == Keysym::Escape);
    if shell_connected && guide {
        return Some(KeyAction::Guide);
    }
    let raw = keysym.raw();
    if (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12).contains(&raw) {
        return Some(KeyAction::VtSwitch((raw - keysyms::KEY_XF86Switch_VT_1 + 1) as i32));
    }
    None
}

impl State {
    pub fn process_input<B: InputBackend>(&mut self, event: InputEvent<B>) {
        match event {
            InputEvent::Keyboard { event } => self.on_key::<B>(event),
            InputEvent::PointerMotion { event } => self.on_pointer_move::<B>(event),
            InputEvent::PointerMotionAbsolute { event } => self.on_pointer_move_absolute::<B>(event),
            InputEvent::PointerButton { event } => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event } => self.on_pointer_axis::<B>(event),
            _ => {}
        }
    }

    fn on_key<B: InputBackend>(&mut self, event: B::KeyboardKeyEvent) {
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        let serial = SERIAL_COUNTER.next_serial();
        let time = Event::time(&event);
        let state = event.state();
        let mut suppressed = std::mem::take(&mut self.suppressed_keys);
        let shell_connected = self.control.has_clients();
        let action = keyboard.input(self, event.key_code(), state, serial, time, |_, modifiers, handle| {
            let keysym = handle.modified_sym();
            if state == KeyState::Pressed {
                match shortcut(modifiers, keysym, shell_connected) {
                    Some(action) => {
                        suppressed.push(keysym.raw());
                        FilterResult::Intercept(Some(action))
                    }
                    None => FilterResult::Forward,
                }
            } else if let Some(pos) = suppressed.iter().position(|&k| k == keysym.raw()) {
                suppressed.remove(pos);
                FilterResult::Intercept(None)
            } else {
                FilterResult::Forward
            }
        });
        self.suppressed_keys = suppressed;
        match action.flatten() {
            Some(KeyAction::Quit) => {
                info!("Ctrl+Alt+Backspace: quitting");
                self.running = false;
                self.loop_signal.stop();
            }
            Some(KeyAction::VtSwitch(vt)) => self.backend.change_vt(vt),
            Some(KeyAction::Guide) => {
                info!("Guide key: to the shell");
                self.control.broadcast(&CompositorMessage::Guide);
            }
            None => {}
        }
        // The keyboard waits for the last key release before it goes back to the app.
        if self.focus_deferred && keyboard.pressed_keys().is_empty() {
            self.apply_policy();
        }
    }

    /// The topmost surface under `position` among the windows that may get pointer input (a
    /// Wayland surface; X11 windows get pointer input through their XWayland surface).
    pub fn surface_under(&self, position: Point<f64, Logical>) -> Option<(FocusTarget, Point<f64, Logical>)> {
        self.space.elements().rev().filter(|window| self.policy.accepts_pointer(State::window_id(window))).find_map(
            |window| {
                let location = self.space.element_location(window)? - window.geometry().loc;
                window.surface_under(position - location.to_f64(), WindowSurfaceType::ALL).map(
                    |(surface, surface_location)| {
                        (FocusTarget::Wayland(surface), (surface_location + location).to_f64())
                    },
                )
            },
        )
    }

    /// Re-targets the pointer after stacking or input rules changed without pointer motion: the
    /// overlay opening takes the pointer from the game (which also releases its pointer lock or
    /// confinement, Smithay deactivates constraints on leave), closing it gives it back and
    /// re-activates the game's constraint.
    pub fn refresh_pointer_focus(&mut self) {
        let pointer = self.pointer.clone();
        let location = pointer.current_location();
        let under = self.surface_under(location);
        let current = pointer.current_focus();
        if current.as_ref() == under.as_ref().map(|(target, _)| target) {
            return;
        }
        let event = MotionEvent { location, serial: SERIAL_COUNTER.next_serial(), time: InputTime::now() };
        pointer.motion(self, under.clone(), &event);
        pointer.frame(self);
        self.activate_constraint(under, location);
    }

    /// Activates a pointer constraint of the surface the pointer is now over.
    fn activate_constraint(
        &mut self,
        under: Option<(FocusTarget, Point<f64, Logical>)>,
        location: Point<f64, Logical>,
    ) {
        let pointer = self.pointer.clone();
        if let Some((surface, surface_location)) =
            under.and_then(|(target, l)| Some((target.wl_surface()?.into_owned(), l)))
        {
            with_pointer_constraint(&surface, &pointer, |constraint| {
                if let Some(constraint) = constraint.filter(|c| !c.is_active()) {
                    let point = (location - surface_location).to_i32_round();
                    if constraint.region().is_none_or(|region| region.contains(point)) {
                        constraint.activate();
                    }
                }
            });
        }
    }

    fn clamp_to_output(&self, position: Point<f64, Logical>) -> Point<f64, Logical> {
        let Some(size) = self.output_size() else { return position };
        (position.x.clamp(0.0, size.w as f64 - 1.0), position.y.clamp(0.0, size.h as f64 - 1.0)).into()
    }

    fn on_pointer_move_absolute<B: InputBackend>(&mut self, event: B::PointerMotionAbsoluteEvent) {
        let Some(size) = self.output_size() else { return };
        let position = event.position_transformed(size);
        let under = self.surface_under(position);
        let pointer = self.pointer.clone();
        pointer.motion(
            self,
            under,
            &MotionEvent { location: position, serial: SERIAL_COUNTER.next_serial(), time: event.time() },
        );
        pointer.frame(self);
    }

    /// Relative motion (libinput), respecting pointer locks and confinement (games).
    fn on_pointer_move<B: InputBackend>(&mut self, event: B::PointerMotionEvent) {
        let pointer = self.pointer.clone();
        let mut location = pointer.current_location();
        let under = self.surface_under(location);

        let mut locked = false;
        let mut confined = false;
        let mut confine_region = None;
        if let Some((surface, surface_location)) =
            under.as_ref().and_then(|(target, l)| Some((target.wl_surface()?.into_owned(), *l)))
        {
            with_pointer_constraint(&surface, &pointer, |constraint| {
                let Some(constraint) = constraint.filter(|c| c.is_active()) else { return };
                if !constraint.region().is_none_or(|r| r.contains((location - surface_location).to_i32_round())) {
                    return;
                }
                match &*constraint {
                    PointerConstraint::Locked(_) => locked = true,
                    PointerConstraint::Confined(confine) => {
                        confined = true;
                        confine_region = confine.region().cloned();
                    }
                }
            });
        }

        pointer.relative_motion(
            self,
            under.clone(),
            &RelativeMotionEvent { delta: event.delta(), delta_unaccel: event.delta_unaccel(), time: event.time() },
        );
        if locked {
            pointer.frame(self);
            return;
        }

        let mut delta = self.clamp_to_output(location + event.delta()) - location;
        if confined && let (Some((_, surface_location)), Some(region)) = (&under, &confine_region) {
            if !region.contains((location + Point::from((delta.x, 0.0)) - *surface_location).to_i32_round()) {
                delta.x = 0.0;
            }
            if !region.contains((location + Point::from((0.0, delta.y)) - *surface_location).to_i32_round()) {
                delta.y = 0.0;
            }
        }
        location += delta;
        let new_under = self.surface_under(location);
        if confined
            && let Some((surface, _)) = &under
            && new_under.as_ref().and_then(|(t, _)| t.wl_surface()) != surface.wl_surface()
        {
            pointer.frame(self);
            return;
        }
        pointer.motion(
            self,
            new_under.clone(),
            &MotionEvent { location, serial: SERIAL_COUNTER.next_serial(), time: event.time() },
        );
        pointer.frame(self);

        // Activate a constraint the pointer just entered.
        self.activate_constraint(new_under, location);
    }

    fn on_pointer_button<B: InputBackend>(&mut self, event: B::PointerButtonEvent) {
        let pointer = self.pointer.clone();
        let state = event.state();
        pointer.button(
            self,
            &ButtonEvent {
                button: event.button_code(),
                state,
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time(),
            },
        );
        pointer.frame(self);
    }

    fn on_pointer_axis<B: InputBackend>(&mut self, event: B::PointerAxisEvent) {
        let amount = |axis| event.amount(axis).unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0);
        let mut frame = AxisFrame::new(event.time()).source(event.source());
        for axis in [Axis::Horizontal, Axis::Vertical] {
            let value = amount(axis);
            if value != 0.0 {
                frame = frame.relative_direction(axis, event.relative_direction(axis)).value(axis, value);
                if let Some(discrete) = event.amount_v120(axis) {
                    frame = frame.v120(axis, discrete as i32);
                }
            }
            if event.source() == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                frame = frame.stop(axis);
            }
        }
        let pointer = self.pointer.clone();
        pointer.axis(self, frame);
        pointer.frame(self);
    }
}
