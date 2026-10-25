//! Device-independent gamepad logic: repeat while held, and stick → direction.

use std::time::{Duration, Instant};

use crate::nav::Action;

/// Delay until the first repeat.
const INITIAL_DELAY: Duration = Duration::from_millis(350);
/// Interval of the first repeats; after that it gets faster.
const REPEAT_SLOW: Duration = Duration::from_millis(120);
const REPEAT_FAST: Duration = Duration::from_millis(70);
const REPEATS_UNTIL_FAST: u32 = 5;

/// Which button glyphs the hints show (Story 3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphStyle {
    PlayStation,
    /// Also for unknown controllers, because SDL mappings follow the Xbox layout.
    Xbox,
}

/// Detects the controller type by device name (the DualShock 4 reports itself as "Wireless Controller").
pub fn glyph_style(name: &str) -> GlyphStyle {
    let name = name.to_lowercase();
    let sony = ["dualsense", "dualshock", "playstation", "ps4", "ps5", "sony", "wireless controller"];
    if sony.iter().any(|s| name.contains(s)) { GlyphStyle::PlayStation } else { GlyphStyle::Xbox }
}

pub fn is_direction(action: Action) -> bool {
    matches!(action, Action::Up | Action::Down | Action::Left | Action::Right)
}

/// A held direction (D-pad or stick) that behaves like keyboard auto-repeat.
#[derive(Debug, Default)]
pub struct DirectionRepeat {
    held: Option<Held>,
}

#[derive(Debug)]
struct Held {
    action: Action,
    next: Instant,
    repeats: u32,
}

impl DirectionRepeat {
    /// New direction: fire immediately and start the repeat. The most recently pressed direction wins.
    pub fn press(&mut self, action: Action, now: Instant) -> Action {
        self.held = Some(Held { action, next: now + INITIAL_DELAY, repeats: 0 });
        action
    }

    /// Only releasing the currently held direction ends the repeat.
    pub fn release(&mut self, action: Action) {
        if self.held.as_ref().is_some_and(|h| h.action == action) {
            self.held = None;
        }
    }

    /// Returns a repeat when it is due (at most one per call).
    pub fn poll(&mut self, now: Instant) -> Option<Action> {
        let held = self.held.as_mut()?;
        if now < held.next {
            return None;
        }
        held.repeats += 1;
        let interval = if held.repeats < REPEATS_UNTIL_FAST { REPEAT_SLOW } else { REPEAT_FAST };
        // Count from now, so no repeats are made up for after a pause in `poll`.
        held.next = now + interval;
        Some(held.action)
    }
}

/// Analog stick with hysteresis, so the direction does not flicker at the threshold.
#[derive(Debug, Default)]
pub struct Stick {
    current: Option<Action>,
}

const PRESS_THRESHOLD: f32 = 0.6;
const RELEASE_THRESHOLD: f32 = 0.4;

/// Change of the stick direction: what was released and what was newly pressed.
#[derive(Debug, Default, PartialEq)]
pub struct StickChange {
    pub released: Option<Action>,
    pub pressed: Option<Action>,
}

impl Stick {
    /// `x` positive to the right, `y` positive upwards (like gilrs), each −1…1.
    pub fn update(&mut self, x: f32, y: f32) -> StickChange {
        let magnitude = x.abs().max(y.abs());
        let dominant = if x.abs() >= y.abs() {
            if x > 0.0 { Action::Right } else { Action::Left }
        } else if y > 0.0 {
            Action::Up
        } else {
            Action::Down
        };

        let next = if magnitude >= PRESS_THRESHOLD {
            Some(dominant)
        } else {
            self.current.filter(|&current| axis_value(current, x, y) >= RELEASE_THRESHOLD)
        };
        if next == self.current {
            return StickChange::default();
        }
        let change = StickChange { released: self.current, pressed: next };
        self.current = next;
        change
    }
}

/// Deflection in the direction of `action`.
fn axis_value(action: Action, x: f32, y: f32) -> f32 {
    match action {
        Action::Right => x,
        Action::Left => -x,
        Action::Up => y,
        Action::Down => -y,
        _ => 0.0,
    }
}

/// Linux evdev key codes (`linux/input-event-codes.h`) needed in the control center while the
/// shell reads the controller exclusively.
pub mod evdev {
    pub const EV_KEY: u16 = 0x01;
    pub const EV_ABS: u16 = 0x03;
    pub const ABS_X: u16 = 0x00;
    pub const ABS_Y: u16 = 0x01;
    pub const ABS_HAT0X: u16 = 0x10;
    pub const ABS_HAT0Y: u16 = 0x11;
    const BTN_SOUTH: u16 = 0x130;
    const BTN_EAST: u16 = 0x131;
    const BTN_START: u16 = 0x13b;
    const BTN_MODE: u16 = 0x13c;
    const BTN_DPAD_UP: u16 = 0x220;
    const BTN_DPAD_DOWN: u16 = 0x221;
    const BTN_DPAD_LEFT: u16 = 0x222;
    const BTN_DPAD_RIGHT: u16 = 0x223;

    use crate::nav::Action;

    /// Only D-pad, confirm, back, menu and PS/Guide; the control center needs nothing more.
    pub fn key_action(code: u16) -> Option<Action> {
        Some(match code {
            BTN_SOUTH => Action::Confirm,
            BTN_EAST => Action::Back,
            BTN_START => Action::Menu,
            BTN_MODE => Action::Guide,
            BTN_DPAD_UP => Action::Up,
            BTN_DPAD_DOWN => Action::Down,
            BTN_DPAD_LEFT => Action::Left,
            BTN_DPAD_RIGHT => Action::Right,
            _ => return None,
        })
    }

    /// Maps an axis value to −1…1 (`min`/`max` from `EVIOCGABS`).
    pub fn normalize(value: i32, min: i32, max: i32) -> f32 {
        if max <= min {
            return 0.0;
        }
        let center = (min as f32 + max as f32) / 2.0;
        let half = (max as f32 - min as f32) / 2.0;
        ((value as f32 - center) / half).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn press_fires_immediately_then_repeats_after_delay() {
        let t0 = Instant::now();
        let mut r = DirectionRepeat::default();
        assert_eq!(r.press(Action::Right, t0), Action::Right);
        assert_eq!(r.poll(t0 + ms(100)), None);
        assert_eq!(r.poll(t0 + ms(350)), Some(Action::Right));
        assert_eq!(r.poll(t0 + ms(400)), None);
        assert_eq!(r.poll(t0 + ms(470)), Some(Action::Right));
    }

    #[test]
    fn repeats_get_faster() {
        let t0 = Instant::now();
        let mut r = DirectionRepeat::default();
        r.press(Action::Left, t0);
        let mut t = t0 + INITIAL_DELAY;
        for _ in 1..REPEATS_UNTIL_FAST {
            assert!(r.poll(t).is_some());
            t += REPEAT_SLOW;
        }
        assert!(r.poll(t).is_some());
        assert_eq!(r.poll(t + REPEAT_FAST - ms(1)), None);
        assert!(r.poll(t + REPEAT_FAST).is_some());
    }

    #[test]
    fn release_stops_only_the_held_direction() {
        let t0 = Instant::now();
        let mut r = DirectionRepeat::default();
        r.press(Action::Left, t0);
        r.press(Action::Right, t0);
        r.release(Action::Left);
        assert_eq!(r.poll(t0 + ms(400)), Some(Action::Right));
        r.release(Action::Right);
        assert_eq!(r.poll(t0 + ms(2000)), None);
    }

    #[test]
    fn no_burst_after_long_pause() {
        let t0 = Instant::now();
        let mut r = DirectionRepeat::default();
        r.press(Action::Down, t0);
        assert!(r.poll(t0 + ms(5000)).is_some());
        assert_eq!(r.poll(t0 + ms(5001)), None);
    }

    #[test]
    fn evdev_mapping_and_axes() {
        assert_eq!(evdev::key_action(0x130), Some(Action::Confirm));
        assert_eq!(evdev::key_action(0x13c), Some(Action::Guide));
        assert_eq!(evdev::key_action(0x222), Some(Action::Left));
        assert_eq!(evdev::key_action(0x133), None);
        // DualSense sticks: 0…255, Xbox: −32768…32767
        assert_eq!(evdev::normalize(255, 0, 255), 1.0);
        assert!(evdev::normalize(128, 0, 255).abs() < 0.01);
        assert_eq!(evdev::normalize(-32768, -32768, 32767), -1.0);
        assert_eq!(evdev::normalize(5, 0, 0), 0.0);
    }

    #[test]
    fn glyph_style_from_name() {
        assert_eq!(glyph_style("DualSense Wireless Controller"), GlyphStyle::PlayStation);
        assert_eq!(glyph_style("Wireless Controller"), GlyphStyle::PlayStation);
        assert_eq!(glyph_style("Xbox Series X Controller"), GlyphStyle::Xbox);
        assert_eq!(glyph_style("8BitDo Pro 2"), GlyphStyle::Xbox);
    }

    #[test]
    fn stick_uses_thresholds_with_hysteresis() {
        let mut s = Stick::default();
        assert_eq!(s.update(0.5, 0.0), StickChange::default());
        assert_eq!(s.update(0.7, 0.1), StickChange { released: None, pressed: Some(Action::Right) });
        // Between the thresholds the direction stays.
        assert_eq!(s.update(0.45, 0.0), StickChange::default());
        assert_eq!(s.update(0.3, 0.0), StickChange { released: Some(Action::Right), pressed: None });
    }

    #[test]
    fn stick_switches_direction() {
        let mut s = Stick::default();
        s.update(0.0, 0.9);
        assert_eq!(s.update(0.0, -0.9), StickChange { released: Some(Action::Up), pressed: Some(Action::Down) });
        assert_eq!(s.update(-0.8, -0.2), StickChange { released: Some(Action::Down), pressed: Some(Action::Left) });
    }
}
