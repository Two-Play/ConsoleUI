//! Gamepad input via gilrs, translated into the same `Action`s as the keyboard.

use std::time::Instant;

use consoleui_core::Action;
use consoleui_core::input::{DirectionRepeat, GlyphStyle, Stick, StickChange, glyph_style, is_direction};
use gilrs::{Axis, Button, Event, EventType, Gamepad, Gilrs, MappingSource, PowerInfo};

pub struct ControllerStatus {
    pub style: GlyphStyle,
    /// Battery level in percent, if the controller reports it
    pub battery: Option<u8>,
}

/// Connection of a controller, for toasts
pub enum Connection {
    Connected(String),
    Disconnected(String),
}

pub struct Gamepads {
    gilrs: Gilrs,
    connections: Vec<Connection>,
    repeat: DirectionRepeat,
    left_stick: Stick,
    /// Some drivers report the D-pad as axes instead of buttons.
    dpad_axes: Stick,
}

impl Gamepads {
    pub fn new() -> Option<Self> {
        match Gilrs::new() {
            Ok(gilrs) => Some(Self {
                gilrs,
                connections: Vec::new(),
                repeat: DirectionRepeat::default(),
                left_stick: Stick::default(),
                dpad_axes: Stick::default(),
            }),
            Err(err) => {
                eprintln!("Gamepads not available: {err}");
                None
            }
        }
    }

    /// Processes all pending events and due repeats.
    pub fn poll(&mut self, now: Instant) -> Vec<Action> {
        let mut actions = Vec::new();
        while let Some(Event { id, event, .. }) = self.gilrs.next_event() {
            let gamepad = self.gilrs.gamepad(id);
            if mapping_known(&gamepad) {
                match event {
                    EventType::Connected => self.connections.push(Connection::Connected(gamepad.name().to_string())),
                    EventType::Disconnected => {
                        self.connections.push(Connection::Disconnected(gamepad.name().to_string()));
                    }
                    _ => {}
                }
            }
            if !is_supported(&gamepad) {
                continue;
            }
            match event {
                EventType::ButtonPressed(button, _) => {
                    if let Some(action) = button_action(button) {
                        actions.push(if is_direction(action) { self.repeat.press(action, now) } else { action });
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if let Some(action) = button_action(button) {
                        self.repeat.release(action);
                    }
                }
                EventType::AxisChanged(Axis::LeftStickX | Axis::LeftStickY, ..) => {
                    let (x, y) = (gamepad.value(Axis::LeftStickX), gamepad.value(Axis::LeftStickY));
                    let change = self.left_stick.update(x, y);
                    apply_change(&mut self.repeat, change, now, &mut actions);
                }
                EventType::AxisChanged(Axis::DPadX | Axis::DPadY, ..) => {
                    let (x, y) = (gamepad.value(Axis::DPadX), gamepad.value(Axis::DPadY));
                    let change = self.dpad_axes.update(x, y);
                    apply_change(&mut self.repeat, change, now, &mut actions);
                }
                _ => {}
            }
        }
        actions.extend(self.repeat.poll(now));
        actions
    }

    /// Connection events since the last call
    pub fn take_connections(&mut self) -> Vec<Connection> {
        std::mem::take(&mut self.connections)
    }

    /// Device names of all connected controllers as the kernel reports them (for exclusive access).
    pub fn names(&self) -> Vec<String> {
        self.gilrs.gamepads().map(|(_, g)| g).filter(is_supported).map(|g| g.os_name().to_string()).collect()
    }

    /// After exclusive access: gilrs did not see the release, so treat nothing as held anymore.
    pub fn reset(&mut self) {
        self.repeat = DirectionRepeat::default();
        self.left_stick = Stick::default();
        self.dpad_axes = Stick::default();
        while self.gilrs.next_event().is_some() {}
    }

    /// First connected controller, for the top bar and the button glyphs.
    pub fn status(&self) -> Option<ControllerStatus> {
        self.gilrs.gamepads().map(|(_, gamepad)| gamepad).find(is_supported).map(|gamepad| ControllerStatus {
            style: glyph_style(gamepad.name()),
            battery: match gamepad.power_info() {
                PowerInfo::Discharging(percent) | PowerInfo::Charging(percent) => Some(percent),
                PowerInfo::Charged => Some(100),
                _ => None,
            },
        })
    }
}

fn apply_change(repeat: &mut DirectionRepeat, change: StickChange, now: Instant, actions: &mut Vec<Action>) {
    if let Some(released) = change.released {
        repeat.release(released);
    }
    if let Some(pressed) = change.pressed {
        actions.push(repeat.press(pressed, now));
    }
}

/// Only devices with a known mapping; this way e.g. the joystick interface of some keyboards
/// does not show up as a controller.
fn is_supported(gamepad: &Gamepad) -> bool {
    gamepad.is_connected() && mapping_known(gamepad)
}

fn mapping_known(gamepad: &Gamepad) -> bool {
    gamepad.mapping_source() != MappingSource::None
}

/// Cross/A confirms, Circle/B goes back, Triangle/Y searches, Square/X filters, L1/R1 switch
/// tabs, Options/Start opens the menu, PS/Guide the control center.
fn button_action(button: Button) -> Option<Action> {
    Some(match button {
        Button::DPadUp => Action::Up,
        Button::DPadDown => Action::Down,
        Button::DPadLeft => Action::Left,
        Button::DPadRight => Action::Right,
        Button::South => Action::Confirm,
        Button::East => Action::Back,
        Button::North => Action::Search,
        Button::West => Action::Secondary,
        Button::LeftTrigger => Action::PrevTab,
        Button::RightTrigger => Action::NextTab,
        Button::Start => Action::Menu,
        Button::Mode => Action::Guide,
        _ => return None,
    })
}
