use crate::nav::Action;

/// Fokus in einem Raster mit variabler Spaltenzahl (die UI berechnet sie aus der Fensterbreite).
#[derive(Debug, Clone, Default)]
pub struct Grid {
    len: usize,
    focused: usize,
}

impl Grid {
    pub fn new(len: usize) -> Self {
        Self { len, focused: 0 }
    }

    pub fn focused(&self) -> usize {
        self.focused
    }

    pub fn focus(&mut self, index: usize) {
        self.focused = index.min(self.len.saturating_sub(1));
    }

    /// Neue Anzahl Einträge (z. B. nach einem Filterwechsel); der Fokus bleibt gültig.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        self.focus(self.focused);
    }

    /// Gibt zurück, ob sich der Fokus geändert hat. Links/Rechts laufen über das Zeilenende hinaus
    /// in die nächste Zeile, Runter springt in einer kürzeren letzten Zeile auf deren letzten Eintrag.
    pub fn apply(&mut self, action: Action, columns: usize) -> bool {
        let columns = columns.max(1);
        let current = self.focused;
        let target = match action {
            Action::Left => current.checked_sub(1),
            Action::Right => (current + 1 < self.len).then_some(current + 1),
            Action::Up => current.checked_sub(columns),
            Action::Down => {
                let next_row_start = (current / columns + 1) * columns;
                (next_row_start < self.len).then(|| (current + columns).min(self.len - 1))
            }
            _ => None,
        };
        match target {
            Some(target) if target != current => {
                self.focused = target;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_in_rows_and_columns() {
        let mut g = Grid::new(10);
        assert!(g.apply(Action::Down, 4));
        assert_eq!(g.focused(), 4);
        assert!(g.apply(Action::Right, 4));
        assert_eq!(g.focused(), 5);
        assert!(g.apply(Action::Up, 4));
        assert_eq!(g.focused(), 1);
        assert!(!g.apply(Action::Up, 4));
    }

    #[test]
    fn right_wraps_into_next_row_and_left_back() {
        let mut g = Grid::new(10);
        g.focus(3);
        assert!(g.apply(Action::Right, 4));
        assert_eq!(g.focused(), 4);
        assert!(g.apply(Action::Left, 4));
        assert_eq!(g.focused(), 3);
    }

    #[test]
    fn down_into_shorter_last_row() {
        let mut g = Grid::new(10);
        g.focus(7);
        assert!(g.apply(Action::Down, 4));
        assert_eq!(g.focused(), 9);
        assert!(!g.apply(Action::Down, 4));
    }

    #[test]
    fn set_len_keeps_focus_valid() {
        let mut g = Grid::new(10);
        g.focus(9);
        g.set_len(3);
        assert_eq!(g.focused(), 2);
        g.set_len(0);
        assert_eq!(g.focused(), 0);
        assert!(!g.apply(Action::Right, 4));
        assert!(!g.apply(Action::Down, 4));
    }
}
