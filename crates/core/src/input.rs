//! Gamepad-Logik ohne Geräteabhängigkeit: Wiederholung beim Gedrückthalten und Stick → Richtung.

use std::time::{Duration, Instant};

use crate::nav::Action;

/// Verzögerung bis zur ersten Wiederholung.
const INITIAL_DELAY: Duration = Duration::from_millis(350);
/// Abstand der ersten Wiederholungen; danach wird es schneller.
const REPEAT_SLOW: Duration = Duration::from_millis(120);
const REPEAT_FAST: Duration = Duration::from_millis(70);
const REPEATS_UNTIL_FAST: u32 = 5;

/// Welche Tastensymbole die Hinweise zeigen (Story 3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphStyle {
    PlayStation,
    /// Auch für unbekannte Controller, weil SDL-Mappings dem Xbox-Layout folgen.
    Xbox,
}

/// Erkennt den Controller-Typ am Gerätenamen (der DualShock 4 meldet sich als „Wireless Controller“).
pub fn glyph_style(name: &str) -> GlyphStyle {
    let name = name.to_lowercase();
    let sony = ["dualsense", "dualshock", "playstation", "ps4", "ps5", "sony", "wireless controller"];
    if sony.iter().any(|s| name.contains(s)) { GlyphStyle::PlayStation } else { GlyphStyle::Xbox }
}

pub fn is_direction(action: Action) -> bool {
    matches!(action, Action::Up | Action::Down | Action::Left | Action::Right)
}

/// Eine gehaltene Richtung (Steuerkreuz oder Stick), die sich wie eine Tastatur-Wiederholung verhält.
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
    /// Neue Richtung: sofort auslösen und die Wiederholung starten. Die zuletzt gedrückte Richtung gewinnt.
    pub fn press(&mut self, action: Action, now: Instant) -> Action {
        self.held = Some(Held { action, next: now + INITIAL_DELAY, repeats: 0 });
        action
    }

    /// Nur das Loslassen der aktuell gehaltenen Richtung beendet die Wiederholung.
    pub fn release(&mut self, action: Action) {
        if self.held.as_ref().is_some_and(|h| h.action == action) {
            self.held = None;
        }
    }

    /// Liefert eine Wiederholung, wenn sie fällig ist (höchstens eine pro Aufruf).
    pub fn poll(&mut self, now: Instant) -> Option<Action> {
        let held = self.held.as_mut()?;
        if now < held.next {
            return None;
        }
        held.repeats += 1;
        let interval = if held.repeats < REPEATS_UNTIL_FAST { REPEAT_SLOW } else { REPEAT_FAST };
        // Ab jetzt zählen, damit nach einer Pause von `poll` keine Wiederholungen nachgeholt werden.
        held.next = now + interval;
        Some(held.action)
    }
}

/// Analog-Stick mit Hysterese, damit die Richtung an der Schwelle nicht flackert.
#[derive(Debug, Default)]
pub struct Stick {
    current: Option<Action>,
}

const PRESS_THRESHOLD: f32 = 0.6;
const RELEASE_THRESHOLD: f32 = 0.4;

/// Änderung der Stick-Richtung: was losgelassen und was neu gedrückt wurde.
#[derive(Debug, Default, PartialEq)]
pub struct StickChange {
    pub released: Option<Action>,
    pub pressed: Option<Action>,
}

impl Stick {
    /// `x` nach rechts positiv, `y` nach oben positiv (wie gilrs), jeweils −1…1.
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

/// Auslenkung in Richtung `action`.
fn axis_value(action: Action, x: f32, y: f32) -> f32 {
    match action {
        Action::Right => x,
        Action::Left => -x,
        Action::Up => y,
        Action::Down => -y,
        _ => 0.0,
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
        // Zwischen den Schwellen bleibt die Richtung bestehen.
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
