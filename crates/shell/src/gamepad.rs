//! Gamepad-Eingaben über gilrs, übersetzt in dieselben `Action`s wie die Tastatur.

use std::time::Instant;

use consoleui_core::Action;
use consoleui_core::input::{DirectionRepeat, GlyphStyle, Stick, StickChange, glyph_style, is_direction};
use gilrs::{Axis, Button, Event, EventType, Gamepad, Gilrs, MappingSource, PowerInfo};

pub struct ControllerStatus {
    pub style: GlyphStyle,
    /// Akkustand in Prozent, falls der Controller ihn meldet
    pub battery: Option<u8>,
}

pub struct Gamepads {
    gilrs: Gilrs,
    repeat: DirectionRepeat,
    left_stick: Stick,
    /// Manche Treiber melden das Steuerkreuz als Achsen statt als Tasten.
    dpad_axes: Stick,
}

impl Gamepads {
    pub fn new() -> Option<Self> {
        match Gilrs::new() {
            Ok(gilrs) => Some(Self {
                gilrs,
                repeat: DirectionRepeat::default(),
                left_stick: Stick::default(),
                dpad_axes: Stick::default(),
            }),
            Err(err) => {
                eprintln!("Gamepads nicht verfügbar: {err}");
                None
            }
        }
    }

    /// Verarbeitet alle anstehenden Events und fällige Wiederholungen.
    pub fn poll(&mut self, now: Instant) -> Vec<Action> {
        let mut actions = Vec::new();
        while let Some(Event { id, event, .. }) = self.gilrs.next_event() {
            let gamepad = self.gilrs.gamepad(id);
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

    /// Erster verbundener Controller, für die Top-Bar und die Tastensymbole.
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

/// Nur Geräte mit bekanntem Mapping; so meldet sich z. B. die Joystick-Schnittstelle mancher
/// Tastaturen nicht als Controller.
fn is_supported(gamepad: &Gamepad) -> bool {
    gamepad.is_connected() && gamepad.mapping_source() != MappingSource::None
}

/// Kreuz/A bestätigt, Kreis/B geht zurück, Dreieck/Y sucht, Quadrat/X filtert, L1/R1 wechseln
/// die Tabs, Options/Start öffnet das Menü, PS/Guide die Steuerzentrale.
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
