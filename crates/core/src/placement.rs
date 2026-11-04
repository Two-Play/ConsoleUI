//! Where the floating on-screen keyboard sits on the screen, and the "move" mode for it: R3 grabs
//! the keyboard, the directions move it, R3, Confirm or Back let go.

use crate::nav::Action;

/// How far one press of a direction moves the panel, as a fraction of the free space.
const STEP: f32 = 0.06;

/// Position of a panel as fractions of the free space on the screen: 0 is the left or top edge,
/// 1 the right or bottom edge, so the panel always stays fully on the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: f32,
    pub y: f32,
}

impl Default for Placement {
    /// Centered, near the bottom edge
    fn default() -> Self {
        Self { x: 0.5, y: 0.94 }
    }
}

impl Placement {
    fn step(&mut self, dx: f32, dy: f32) {
        self.x = (self.x + dx).clamp(0.0, 1.0);
        self.y = (self.y + dy).clamp(0.0, 1.0);
    }
}

/// The keyboard's placement plus the move mode.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KeyboardPlacement {
    placement: Placement,
    moving: bool,
}

impl KeyboardPlacement {
    pub fn placement(&self) -> Placement {
        self.placement
    }

    /// The keyboard is grabbed: the directions move it instead of the key focus.
    pub fn moving(&self) -> bool {
        self.moving
    }

    /// Handles an action; `true` when it was consumed (the keyboard must not react to it).
    /// `Reposition` always grabs or lets go; the rest only counts while the keyboard is grabbed.
    pub fn apply(&mut self, action: Action) -> bool {
        if action == Action::Reposition {
            self.moving = !self.moving;
            return true;
        }
        if !self.moving {
            return false;
        }
        match action {
            Action::Left => self.placement.step(-STEP, 0.0),
            Action::Right => self.placement.step(STEP, 0.0),
            Action::Up => self.placement.step(0.0, -STEP),
            Action::Down => self.placement.step(0.0, STEP),
            Action::Confirm | Action::Back => self.moving = false,
            _ => {}
        }
        true
    }

    /// Back to the default place and not grabbed (a new text entry starts)
    pub fn reset_mode(&mut self) {
        self.moving = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reposition_grabs_and_lets_go() {
        let mut k = KeyboardPlacement::default();
        assert!(!k.moving());
        assert!(k.apply(Action::Reposition));
        assert!(k.moving());
        assert!(k.apply(Action::Reposition));
        assert!(!k.moving());
    }

    #[test]
    fn directions_only_move_while_grabbed() {
        let mut k = KeyboardPlacement::default();
        let start = k.placement();
        assert!(!k.apply(Action::Left), "not consumed: the key focus moves instead");
        assert_eq!(k.placement(), start);
        k.apply(Action::Reposition);
        assert!(k.apply(Action::Left));
        assert!(k.placement().x < start.x);
        assert!(k.apply(Action::Up));
        assert!(k.placement().y < start.y);
    }

    #[test]
    fn stays_on_the_screen() {
        let mut k = KeyboardPlacement::default();
        k.apply(Action::Reposition);
        for _ in 0..50 {
            k.apply(Action::Right);
            k.apply(Action::Down);
        }
        assert_eq!(k.placement(), Placement { x: 1.0, y: 1.0 });
        for _ in 0..50 {
            k.apply(Action::Left);
            k.apply(Action::Up);
        }
        assert_eq!(k.placement(), Placement { x: 0.0, y: 0.0 });
    }

    #[test]
    fn confirm_or_back_let_go_and_keep_the_place() {
        let mut k = KeyboardPlacement::default();
        k.apply(Action::Reposition);
        k.apply(Action::Left);
        let at = k.placement();
        assert!(k.apply(Action::Confirm));
        assert!(!k.moving());
        assert_eq!(k.placement(), at);
        k.apply(Action::Reposition);
        assert!(k.apply(Action::Back));
        assert!(!k.moving());
    }
}
