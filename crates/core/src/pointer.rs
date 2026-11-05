//! The gamepad as a mouse (Story 7.15): the stick moves the pointer of the compositor, buttons click
//! and scroll. For windows made for a mouse, such as the Steam client's dialogs.

use crate::Action;

/// `BTN_LEFT` and `BTN_RIGHT` of `linux/input-event-codes.h`
pub const BUTTON_LEFT: u32 = 0x110;
pub const BUTTON_RIGHT: u32 = 0x111;

/// Stick travel that counts as resting
const DEAD_ZONE: f64 = 0.15;
/// Pixels per second with the stick at its limit
const MAX_SPEED: f64 = 1100.0;
/// A longer gap between two polls (the quick menu was open) does not become a jump.
const MAX_STEP_SECONDS: f64 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerCommand {
    Click(u32),
    /// Wheel steps, positive is down
    Scroll(i32),
    Leave,
}

/// What a button does while the gamepad is a mouse; the rest does nothing.
pub fn command(action: Action) -> Option<PointerCommand> {
    match action {
        Action::Confirm => Some(PointerCommand::Click(BUTTON_LEFT)),
        Action::Secondary => Some(PointerCommand::Click(BUTTON_RIGHT)),
        Action::PrevTab => Some(PointerCommand::Scroll(-1)),
        Action::NextTab => Some(PointerCommand::Scroll(1)),
        Action::Back => Some(PointerCommand::Leave),
        _ => None,
    }
}

/// Turns the stick into whole pixels of motion; what is left of a pixel carries over, so slow
/// movements still arrive.
#[derive(Debug, Default)]
pub struct StickPointer {
    rest: (f64, f64),
}

impl StickPointer {
    /// `x` and `y` from -1 to 1, down is positive. Slow near the center for aiming at small buttons.
    pub fn step(&mut self, x: f32, y: f32, seconds: f64) -> Option<(i32, i32)> {
        let (x, y) = (x as f64, y as f64);
        let length = x.hypot(y);
        if length <= DEAD_ZONE {
            self.rest = (0.0, 0.0);
            return None;
        }
        let travel = ((length - DEAD_ZONE) / (1.0 - DEAD_ZONE)).min(1.0);
        let distance = MAX_SPEED * travel * travel * seconds.min(MAX_STEP_SECONDS);
        let moved = (self.rest.0 + x / length * distance, self.rest.1 + y / length * distance);
        let whole = (moved.0.trunc(), moved.1.trunc());
        self.rest = (moved.0 - whole.0, moved.1 - whole.1);
        (whole != (0.0, 0.0)).then_some((whole.0 as i32, whole.1 as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resting_stick_does_not_move() {
        let mut pointer = StickPointer::default();
        assert_eq!(pointer.step(0.1, -0.1, 0.008), None);
    }

    #[test]
    fn full_travel_reaches_the_top_speed() {
        let mut pointer = StickPointer::default();
        assert_eq!(pointer.step(1.0, 0.0, 0.01), Some((11, 0)));
        assert_eq!(pointer.step(0.0, -1.0, 0.01), Some((0, -11)));
    }

    #[test]
    fn slow_movement_adds_up() {
        let mut pointer = StickPointer::default();
        let moved: i32 = (0..125).filter_map(|_| pointer.step(0.3, 0.0, 0.008)).map(|(x, _)| x).sum();
        // About 34 px/s at this travel
        assert!((30..=38).contains(&moved), "{moved}");
    }

    #[test]
    fn a_long_gap_is_no_jump() {
        let mut pointer = StickPointer::default();
        assert_eq!(pointer.step(1.0, 0.0, 5.0), Some((55, 0)));
    }

    #[test]
    fn buttons_click_and_scroll() {
        assert_eq!(command(Action::Confirm), Some(PointerCommand::Click(BUTTON_LEFT)));
        assert_eq!(command(Action::Secondary), Some(PointerCommand::Click(BUTTON_RIGHT)));
        assert_eq!(command(Action::NextTab), Some(PointerCommand::Scroll(1)));
        assert_eq!(command(Action::Back), Some(PointerCommand::Leave));
        assert_eq!(command(Action::Left), None);
    }
}
