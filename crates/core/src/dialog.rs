//! State of a modal alert/confirmation dialog: a title, a text and one to three buttons.
//! Left/Right move the focus, Confirm picks the focused button, Back dismisses the dialog.
//! `T` says what a button does; the shell maps it to a reaction.

use crate::Action;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogButton<T> {
    pub label: String,
    pub tag: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogOutcome<T> {
    /// Nothing to do (focus moved or the action does not apply)
    None,
    /// The tag of the picked button
    Chosen(T),
    /// Back: closed without a choice
    Dismissed,
}

#[derive(Debug, Clone)]
pub struct Dialog<T> {
    pub title: String,
    pub text: String,
    buttons: Vec<DialogButton<T>>,
    focused: usize,
}

impl<T: Clone> Dialog<T> {
    /// `focused` is the button that is focused first (clamped); buttons must not be empty.
    pub fn new(
        title: impl Into<String>,
        text: impl Into<String>,
        buttons: Vec<DialogButton<T>>,
        focused: usize,
    ) -> Self {
        assert!(!buttons.is_empty(), "a dialog needs a button");
        let focused = focused.min(buttons.len() - 1);
        Self { title: title.into(), text: text.into(), buttons, focused }
    }

    /// A dialog with one "OK"-style button.
    pub fn alert(title: impl Into<String>, text: impl Into<String>, label: impl Into<String>, tag: T) -> Self {
        Self::new(title, text, vec![DialogButton { label: label.into(), tag }], 0)
    }

    pub fn buttons(&self) -> &[DialogButton<T>] {
        &self.buttons
    }

    pub fn focused(&self) -> usize {
        self.focused
    }

    pub fn apply(&mut self, action: Action) -> DialogOutcome<T> {
        match action {
            Action::Left | Action::PrevTab => self.focused = self.focused.saturating_sub(1),
            Action::Right | Action::NextTab => self.focused = (self.focused + 1).min(self.buttons.len() - 1),
            Action::Confirm => return DialogOutcome::Chosen(self.buttons[self.focused].tag.clone()),
            Action::Back => return DialogOutcome::Dismissed,
            _ => {}
        }
        DialogOutcome::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> Dialog<&'static str> {
        let button = |label: &str, tag| DialogButton { label: label.into(), tag };
        Dialog::new(
            "Quit?",
            "Unsaved progress may be lost.",
            vec![button("Cancel", "cancel"), button("Quit", "quit")],
            0,
        )
    }

    #[test]
    fn focus_moves_without_wrapping_and_confirm_picks() {
        let mut dialog = two();
        assert_eq!(dialog.apply(Action::Left), DialogOutcome::None);
        assert_eq!(dialog.focused(), 0);
        dialog.apply(Action::Right);
        dialog.apply(Action::Right);
        assert_eq!(dialog.focused(), 1);
        assert_eq!(dialog.apply(Action::Confirm), DialogOutcome::Chosen("quit"));
    }

    #[test]
    fn back_dismisses_and_other_actions_are_ignored() {
        let mut dialog = two();
        assert_eq!(dialog.apply(Action::Down), DialogOutcome::None);
        assert_eq!(dialog.apply(Action::Back), DialogOutcome::Dismissed);
    }

    #[test]
    fn initial_focus_is_clamped_and_alert_has_one_button() {
        let button = DialogButton { label: "OK".into(), tag: 1 };
        assert_eq!(Dialog::new("t", "x", vec![button], 5).focused(), 0);
        let mut alert = Dialog::alert("Oops", "text", "OK", 7);
        assert_eq!(alert.buttons().len(), 1);
        assert_eq!(alert.apply(Action::Confirm), DialogOutcome::Chosen(7));
    }
}
