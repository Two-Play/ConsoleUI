//! What the app does, without any UI: easy to test, and every input path ends up here.

use consoleui_sdk::{Action, Sound};

pub struct Page {
    pub rows: Vec<(&'static str, bool)>,
    pub focused: usize,
    /// The quit dialog is open, with "Stay" (0) or "Quit" (1) focused
    pub quitting: Option<usize>,
}

/// What the window should do after an input
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    Changed(Sound),
    Quit,
}

impl Page {
    pub fn new() -> Self {
        Self { rows: vec![("First option", true), ("Second option", false)], focused: 0, quitting: None }
    }

    pub fn apply(&mut self, action: Action) -> Outcome {
        if let Some(button) = self.quitting {
            return self.apply_dialog(button, action);
        }
        match action {
            Action::Up if self.focused > 0 => self.focused -= 1,
            Action::Down if self.focused + 1 < self.rows.len() => self.focused += 1,
            Action::Left | Action::Right | Action::Confirm => self.rows[self.focused].1 ^= true,
            // ○ always leads back; on the first page that means leaving the app
            Action::Back => {
                self.quitting = Some(0);
                return Outcome::Changed(Sound::Confirm);
            }
            _ => return Outcome::Nothing,
        }
        Outcome::Changed(Sound::Move)
    }

    fn apply_dialog(&mut self, button: usize, action: Action) -> Outcome {
        match action {
            Action::Left | Action::Right => self.quitting = Some(1 - button),
            Action::Confirm if button == 1 => return Outcome::Quit,
            Action::Confirm | Action::Back => self.quitting = None,
            _ => return Outcome::Nothing,
        }
        Outcome::Changed(Sound::Back)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_toggles_the_focused_row() {
        let mut page = Page::new();
        page.apply(Action::Down);
        page.apply(Action::Confirm);
        assert!(page.rows[1].1);
    }

    #[test]
    fn back_asks_before_quitting() {
        let mut page = Page::new();
        page.apply(Action::Back);
        assert_eq!(page.apply(Action::Confirm), Outcome::Changed(Sound::Back));
        assert_eq!(page.quitting, None);
        page.apply(Action::Back);
        page.apply(Action::Right);
        assert_eq!(page.apply(Action::Confirm), Outcome::Quit);
    }
}
