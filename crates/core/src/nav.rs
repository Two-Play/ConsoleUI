/// Eingabeunabhängige UI-Aktion. Tastatur und (später) Gamepad werden beide hierauf abgebildet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
    Menu,
    /// Dreieck/Y: Suche öffnen (in der Suche: Leerzeichen)
    Search,
    /// Quadrat/X: Filter wechseln (in der Suche: Zeichen löschen)
    Secondary,
    /// L1/LB und R1/RB: zwischen den Tabs wechseln
    PrevTab,
    NextTab,
}

/// Fokus innerhalb einer horizontalen Kachelreihe. Läuft am Rand nicht im Kreis (wie bei der PS5).
#[derive(Debug, Clone)]
pub struct Carousel {
    len: usize,
    focused: usize,
}

impl Carousel {
    pub fn new(len: usize) -> Self {
        Self { len, focused: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn focused(&self) -> usize {
        self.focused
    }

    /// Setzt den Fokus direkt; Werte außerhalb werden auf den letzten Eintrag begrenzt.
    pub fn focus(&mut self, index: usize) {
        self.focused = index.min(self.len.saturating_sub(1));
    }

    /// Wendet eine Aktion an und gibt zurück, ob sich der Fokus geändert hat.
    pub fn apply(&mut self, action: Action) -> bool {
        let target = match action {
            Action::Left => self.focused.saturating_sub(1),
            Action::Right if self.focused + 1 < self.len => self.focused + 1,
            _ => return false,
        };
        let changed = target != self.focused;
        self.focused = target;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_first_tile() {
        assert_eq!(Carousel::new(5).focused(), 0);
    }

    #[test]
    fn moves_right_and_left() {
        let mut c = Carousel::new(3);
        assert!(c.apply(Action::Right));
        assert!(c.apply(Action::Right));
        assert_eq!(c.focused(), 2);
        assert!(c.apply(Action::Left));
        assert_eq!(c.focused(), 1);
    }

    #[test]
    fn clamps_at_edges() {
        let mut c = Carousel::new(2);
        assert!(!c.apply(Action::Left));
        c.apply(Action::Right);
        assert!(!c.apply(Action::Right));
        assert_eq!(c.focused(), 1);
    }

    #[test]
    fn focus_is_clamped() {
        let mut c = Carousel::new(3);
        c.focus(1);
        assert_eq!(c.focused(), 1);
        c.focus(10);
        assert_eq!(c.focused(), 2);
        let mut empty = Carousel::new(0);
        empty.focus(4);
        assert_eq!(empty.focused(), 0);
    }

    #[test]
    fn empty_carousel_ignores_input() {
        let mut c = Carousel::new(0);
        assert!(c.is_empty());
        assert!(!c.apply(Action::Right));
        assert!(!c.apply(Action::Left));
        assert_eq!(c.focused(), 0);
    }

    #[test]
    fn other_actions_do_not_move_focus() {
        let mut c = Carousel::new(3);
        for action in [Action::Up, Action::Down, Action::Confirm, Action::Back, Action::Menu] {
            assert!(!c.apply(action));
        }
        assert_eq!(c.focused(), 0);
    }
}
