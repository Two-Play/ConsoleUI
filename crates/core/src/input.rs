//! Device-independent gamepad logic: repeat while held, and stick → direction.

use std::time::{Duration, Instant};

use crate::nav::Action;
use crate::settings::GlyphChoice;

/// The Guide button alone opens the quick menu, Guide + R1 takes a screenshot (Story 7.12, like
/// Steam's chord). So Guide counts on release, when it is clear that it was pressed alone.
#[derive(Debug, Default)]
pub struct GuideChord {
    held: bool,
    used: bool,
}

impl GuideChord {
    pub fn press(&mut self) {
        *self = Self { held: true, used: false };
    }

    /// R1 was pressed: whether that completes the chord.
    pub fn second_button(&mut self) -> bool {
        self.used |= self.held;
        self.held
    }

    /// `Action::Guide` if the button was pressed alone.
    pub fn release(&mut self) -> Option<Action> {
        let alone = self.held && !self.used;
        self.held = false;
        alone.then_some(Action::Guide)
    }
}

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

/// Steam Input's virtual gamepad (uinput, named like an Xbox 360 pad). It repeats what a real
/// controller does for games, so next to a real controller it would double every press; on its
/// own it can be the only way to read a controller, for example with Remote Play.
pub fn is_steam_virtual_pad(vendor: Option<u16>, product: Option<u16>) -> bool {
    vendor == Some(0x28de) && product == Some(0x11ff)
}

/// The input device used last; the hints show its buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDevice {
    Keyboard,
    Gamepad,
}

/// Glyphs for the hints: keyboard keys (`None`) after keyboard input or without a controller,
/// otherwise the chosen style, or with `Auto` the connected controller's.
pub fn hint_glyphs(choice: GlyphChoice, controller: Option<GlyphStyle>, last: InputDevice) -> Option<GlyphStyle> {
    let controller = controller?;
    if last == InputDevice::Keyboard {
        return None;
    }
    Some(match choice {
        GlyphChoice::Auto => controller,
        GlyphChoice::PlayStation => GlyphStyle::PlayStation,
        GlyphChoice::Xbox => GlyphStyle::Xbox,
    })
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
    const BTN_NORTH: u16 = 0x133;
    const BTN_WEST: u16 = 0x134;
    const BTN_TL: u16 = 0x136;
    const BTN_TR: u16 = 0x137;
    const BTN_START: u16 = 0x13b;
    const BTN_MODE: u16 = 0x13c;
    const BTN_THUMBR: u16 = 0x13e;
    const BTN_DPAD_UP: u16 = 0x220;
    const BTN_DPAD_DOWN: u16 = 0x221;
    const BTN_DPAD_LEFT: u16 = 0x222;
    const BTN_DPAD_RIGHT: u16 = 0x223;

    use crate::nav::Action;

    /// Only D-pad, confirm, back, menu and PS/Guide; the control center needs nothing more.
    /// The same buttons as gilrs with an SDL mapping. The kernel's Xbox driver reports X and Y as
    /// `BTN_X` and `BTN_Y`, which are the codes of north and west: the wrong way round.
    pub fn key_action(code: u16, style: super::GlyphStyle) -> Option<Action> {
        let xbox = style == super::GlyphStyle::Xbox;
        Some(match code {
            BTN_SOUTH => Action::Confirm,
            BTN_EAST => Action::Back,
            BTN_NORTH if xbox => Action::Secondary,
            BTN_WEST if xbox => Action::Search,
            BTN_NORTH => Action::Search,
            BTN_WEST => Action::Secondary,
            BTN_TL => Action::PrevTab,
            BTN_TR => Action::NextTab,
            BTN_THUMBR => Action::Reposition,
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
    #[test]
    fn only_steams_virtual_pad_counts_as_virtual() {
        assert!(super::is_steam_virtual_pad(Some(0x28de), Some(0x11ff)));
        // A real Steam Controller and a real Xbox 360 pad
        assert!(!super::is_steam_virtual_pad(Some(0x28de), Some(0x1102)));
        assert!(!super::is_steam_virtual_pad(Some(0x045e), Some(0x028e)));
        assert!(!super::is_steam_virtual_pad(None, None));
    }

    use super::*;

    #[test]
    fn guide_alone_opens_on_release_and_the_chord_swallows_it() {
        let mut guide = GuideChord::default();
        assert!(!guide.second_button());
        assert_eq!(guide.release(), None);
        guide.press();
        assert_eq!(guide.release(), Some(Action::Guide));
        guide.press();
        assert!(guide.second_button());
        assert!(guide.second_button());
        assert_eq!(guide.release(), None);
        assert!(!guide.second_button());
    }

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
    fn hints_follow_the_last_input_device() {
        use GlyphChoice::*;
        let pad = InputDevice::Gamepad;
        assert_eq!(hint_glyphs(Auto, None, pad), None);
        assert_eq!(hint_glyphs(Auto, Some(GlyphStyle::PlayStation), pad), Some(GlyphStyle::PlayStation));
        assert_eq!(hint_glyphs(Xbox, Some(GlyphStyle::PlayStation), pad), Some(GlyphStyle::Xbox));
        assert_eq!(hint_glyphs(PlayStation, Some(GlyphStyle::Xbox), InputDevice::Keyboard), None);
    }

    #[test]
    fn evdev_mapping_and_axes() {
        let ps = GlyphStyle::PlayStation;
        assert_eq!(evdev::key_action(0x130, ps), Some(Action::Confirm));
        assert_eq!(evdev::key_action(0x13c, ps), Some(Action::Guide));
        assert_eq!(evdev::key_action(0x222, ps), Some(Action::Left));
        assert_eq!(evdev::key_action(0x133, ps), Some(Action::Search));
        assert_eq!(evdev::key_action(0x134, ps), Some(Action::Secondary));
        assert_eq!(evdev::key_action(0x133, GlyphStyle::Xbox), Some(Action::Secondary));
        assert_eq!(evdev::key_action(0x134, GlyphStyle::Xbox), Some(Action::Search));
        assert_eq!(evdev::key_action(0x137, ps), Some(Action::NextTab));
        assert_eq!(evdev::key_action(0x13e, ps), Some(Action::Reposition));
        assert_eq!(evdev::key_action(0x13a, ps), None);
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
