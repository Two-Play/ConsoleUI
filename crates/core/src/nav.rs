/// Input-agnostic UI action. Keyboard and (later) gamepad are both mapped to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
    Menu,
    /// Triangle/Y: open the search (in the search: space)
    Search,
    /// Square/X: switch filter (in the search: delete a character)
    Secondary,
    /// L1/LB and R1/RB: switch between the tabs
    PrevTab,
    NextTab,
    /// PS/Guide button: quick menu, also while a game is running
    Guide,
}

/// Focus within a horizontal tile row. Does not wrap around at the edge (like on the PS5).
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

    /// Sets the focus directly; values out of range are clamped to the last entry.
    pub fn focus(&mut self, index: usize) {
        self.focused = index.min(self.len.saturating_sub(1));
    }

    /// Applies an action and returns whether the focus changed.
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

/// Where the focus is on the home screen: the tile row, the buttons below the info on the focused
/// entry, or the tabs of the top bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HomeArea {
    #[default]
    Tiles,
    /// The button with this index (0: the main action, 1: more options)
    Buttons(usize),
    /// The tab with this index (0: Home, 1: Library, 2: Settings)
    Tabs(usize),
}

/// What a home screen action means after [`HomeArea::apply`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeOutcome {
    /// Not about the buttons or tabs: the tile row (or the screen) handles it as before.
    Pass,
    /// The focus moved between the areas or within one.
    Moved,
    /// Nothing happens (e.g. down on the buttons).
    Ignored,
    /// Confirm on a button
    Press(usize),
    /// Confirm on a tab
    OpenTab(usize),
}

impl HomeArea {
    /// Moves between the row (up: the tabs, down: the buttons) and within the buttons and tabs.
    /// `buttons` and `tabs` are how many there are.
    pub fn apply(&mut self, action: Action, buttons: usize, tabs: usize) -> HomeOutcome {
        let (area, outcome) = match (*self, action) {
            (HomeArea::Tiles, Action::Up) if tabs > 0 => (HomeArea::Tabs(0), HomeOutcome::Moved),
            (HomeArea::Tiles, Action::Down) if buttons > 0 => (HomeArea::Buttons(0), HomeOutcome::Moved),
            (HomeArea::Tiles, _) => return HomeOutcome::Pass,
            (HomeArea::Buttons(_), Action::Up | Action::Back) | (HomeArea::Tabs(_), Action::Down | Action::Back) => {
                (HomeArea::Tiles, HomeOutcome::Moved)
            }
            (HomeArea::Buttons(i), Action::Confirm) => (HomeArea::Tiles, HomeOutcome::Press(i)),
            (HomeArea::Tabs(i), Action::Confirm) => (HomeArea::Tiles, HomeOutcome::OpenTab(i)),
            (HomeArea::Buttons(i), Action::Left | Action::Right) => {
                (HomeArea::Buttons(step(i, action, buttons)), HomeOutcome::Moved)
            }
            (HomeArea::Tabs(i), Action::Left | Action::Right) => {
                (HomeArea::Tabs(step(i, action, tabs)), HomeOutcome::Moved)
            }
            (HomeArea::Buttons(_), Action::Down) | (HomeArea::Tabs(_), Action::Up) => return HomeOutcome::Ignored,
            _ => return HomeOutcome::Pass,
        };
        let outcome = if outcome == HomeOutcome::Moved && area == *self { HomeOutcome::Ignored } else { outcome };
        *self = area;
        outcome
    }
}

/// One step left or right within `0..len`, stopping at the ends.
fn step(index: usize, action: Action, len: usize) -> usize {
    match action {
        Action::Left => index.saturating_sub(1),
        _ => (index + 1).min(len.saturating_sub(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_focus_moves_between_row_buttons_and_tabs() {
        let mut area = HomeArea::default();
        assert_eq!(area.apply(Action::Right, 2, 3), HomeOutcome::Pass);
        assert_eq!(area.apply(Action::Down, 2, 3), HomeOutcome::Moved);
        assert_eq!(area, HomeArea::Buttons(0));
        assert_eq!(area.apply(Action::Right, 2, 3), HomeOutcome::Moved);
        assert_eq!(area.apply(Action::Right, 2, 3), HomeOutcome::Ignored);
        assert_eq!(area, HomeArea::Buttons(1));
        assert_eq!(area.apply(Action::Down, 2, 3), HomeOutcome::Ignored);
        assert_eq!(area.apply(Action::Up, 2, 3), HomeOutcome::Moved);
        assert_eq!(area, HomeArea::Tiles);
        assert_eq!(area.apply(Action::Up, 2, 3), HomeOutcome::Moved);
        assert_eq!(area, HomeArea::Tabs(0));
        assert_eq!(area.apply(Action::Left, 2, 3), HomeOutcome::Ignored);
        area.apply(Action::Right, 2, 3);
        area.apply(Action::Right, 2, 3);
        assert_eq!(area.apply(Action::Right, 2, 3), HomeOutcome::Ignored);
        assert_eq!(area, HomeArea::Tabs(2));
        assert_eq!(area.apply(Action::Back, 2, 3), HomeOutcome::Moved);
        assert_eq!(area, HomeArea::Tiles);
    }

    #[test]
    fn home_confirm_presses_the_focused_button_or_tab() {
        let mut area = HomeArea::Buttons(1);
        assert_eq!(area.apply(Action::Confirm, 2, 3), HomeOutcome::Press(1));
        assert_eq!(area, HomeArea::Tiles);
        let mut area = HomeArea::Tabs(2);
        assert_eq!(area.apply(Action::Confirm, 2, 3), HomeOutcome::OpenTab(2));
        assert_eq!(area, HomeArea::Tiles);
        // Other buttons (menu, tabs, guide) keep working from anywhere.
        let mut area = HomeArea::Tabs(1);
        assert_eq!(area.apply(Action::Menu, 2, 3), HomeOutcome::Pass);
        assert_eq!(area.apply(Action::NextTab, 2, 3), HomeOutcome::Pass);
        // Without buttons the focus stays in the row.
        let mut area = HomeArea::Tiles;
        assert_eq!(area.apply(Action::Down, 0, 3), HomeOutcome::Pass);
    }

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
