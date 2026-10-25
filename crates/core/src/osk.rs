//! On-screen keyboard for searching with the gamepad. For passwords there is a variant with a
//! shift key (upper case and symbols). The layout follows the language: QWERTZ for German,
//! QWERTY for English.

use crate::i18n::{self, Language, tr};
use crate::nav::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Space,
    Backspace,
    Done,
    /// Switches between lower and upper case, or digits and symbols (password only)
    Shift,
}

/// Key layout
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// German
    Qwertz,
    /// US English
    Qwerty,
}

impl Layout {
    pub fn for_language(language: Language) -> Self {
        match language {
            Language::German => Self::Qwertz,
            Language::English => Self::Qwerty,
        }
    }

    /// Character rows with their indent in key widths
    fn rows(self) -> [(&'static str, f32); 4] {
        match self {
            Self::Qwertz => [("1234567890", 0.5), ("QWERTZUIOPÜ", 0.0), ("ASDFGHJKLÖÄ", 0.25), ("YXCVBNM-.'", 0.75)],
            Self::Qwerty => [("1234567890", 0.5), ("QWERTYUIOP", 0.5), ("ASDFGHJKL'", 0.75), ("ZXCVBNM-.@", 0.75)],
        }
    }

    /// Character on the shift layer
    fn shifted(self, c: char) -> char {
        match self {
            Self::Qwertz => shifted_qwertz(c),
            Self::Qwerty => shifted_qwerty(c),
        }
    }
}

/// Character on the shift layer (German layout)
fn shifted_qwertz(c: char) -> char {
    match c {
        '1' => '!',
        '2' => '"',
        '3' => '§',
        '4' => '$',
        '5' => '%',
        '6' => '&',
        '7' => '/',
        '8' => '(',
        '9' => ')',
        '0' => '=',
        '-' => '_',
        '.' => ':',
        '\'' => '#',
        other => other,
    }
}

/// Character on the shift layer (US layout)
fn shifted_qwerty(c: char) -> char {
    match c {
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '.' => ':',
        '\'' => '"',
        '@' => '/',
        other => other,
    }
}

impl Key {
    pub fn label(&self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::Shift => "⇧".into(),
            Key::Space => tr("Space").into(),
            Key::Backspace => "⌫".into(),
            Key::Done => tr("Done").into(),
        }
    }
}

/// A key with its position in key widths (for the layout and for up/down navigation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyCap {
    pub key: Key,
    pub row: usize,
    pub x: f32,
    pub width: f32,
}

impl KeyCap {
    fn center(&self) -> f32 {
        self.x + self.width / 2.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardEvent {
    None,
    Moved,
    Type(char),
    Space,
    Backspace,
    /// Shift pressed; the labels have changed
    Shift,
    /// "Done" pressed, or navigated down out of the keyboard
    Done,
}

pub struct OnScreenKeyboard {
    layout: Layout,
    keys: Vec<KeyCap>,
    focused: usize,
    /// Only with a shift key: upper case/symbols active (stays on until pressed again)
    shift: bool,
    /// Type letters in upper and lower case (password) instead of always lower case (search)
    case_sensitive: bool,
}

impl Default for OnScreenKeyboard {
    /// Search keyboard in the layout of the current language
    fn default() -> Self {
        Self::new(Layout::for_language(i18n::language()), false)
    }
}

impl OnScreenKeyboard {
    /// Keyboard for passwords in the layout of the current language: with a shift key, letters
    /// in both cases.
    pub fn password() -> Self {
        Self::new(Layout::for_language(i18n::language()), true)
    }

    pub fn new(layout: Layout, password: bool) -> Self {
        let rows = layout.rows();
        let mut keys = Vec::new();
        for (row, (chars, indent)) in rows.iter().enumerate() {
            for (col, c) in chars.chars().enumerate() {
                keys.push(KeyCap { key: Key::Char(c), row, x: indent + col as f32, width: 1.0 });
            }
        }
        let last = rows.len();
        if password {
            keys.push(KeyCap { key: Key::Shift, row: last, x: 0.0, width: 2.0 });
            keys.push(KeyCap { key: Key::Space, row: last, x: 2.0, width: 4.0 });
        } else {
            keys.push(KeyCap { key: Key::Space, row: last, x: 0.0, width: 6.0 });
        }
        keys.push(KeyCap { key: Key::Backspace, row: last, x: 6.0, width: 2.5 });
        keys.push(KeyCap { key: Key::Done, row: last, x: 8.5, width: 2.5 });
        let mut keyboard = Self { layout, keys, focused: 0, shift: false, case_sensitive: password };
        keyboard.reset_focus();
        keyboard
    }

    pub fn keys(&self) -> &[KeyCap] {
        &self.keys
    }

    pub fn shift(&self) -> bool {
        self.shift
    }

    /// Label of key `index` for the current shift layer.
    pub fn label(&self, index: usize) -> String {
        match self.keys[index].key {
            Key::Char(c) if self.case_sensitive => self.char_for(c).to_string(),
            key => key.label(),
        }
    }

    /// Which character a key types
    fn char_for(&self, c: char) -> char {
        match (self.case_sensitive, self.shift) {
            (true, true) if c.is_alphabetic() => c,
            (true, true) => self.layout.shifted(c),
            _ => c.to_lowercase().next().unwrap_or(c),
        }
    }

    pub fn focused(&self) -> usize {
        self.focused
    }

    /// Focus on "Q", as when the search opens.
    pub fn reset_focus(&mut self) {
        self.focused = self.keys.iter().position(|k| k.key == Key::Char('Q')).unwrap_or(0);
        self.shift = false;
    }

    pub fn apply(&mut self, action: Action) -> KeyboardEvent {
        let current = self.keys[self.focused];
        let target = match action {
            Action::Left => self.focused.checked_sub(1).filter(|&i| self.keys[i].row == current.row),
            Action::Right => Some(self.focused + 1).filter(|&i| i < self.keys.len() && self.keys[i].row == current.row),
            Action::Up => current.row.checked_sub(1).and_then(|row| self.nearest_in_row(row, current.center())),
            Action::Down => match self.nearest_in_row(current.row + 1, current.center()) {
                Some(i) => Some(i),
                None => return KeyboardEvent::Done,
            },
            Action::Confirm => {
                return match current.key {
                    // Letters go into the search in lower case; the keys show them in upper case.
                    Key::Char(c) => KeyboardEvent::Type(self.char_for(c)),
                    Key::Shift => {
                        self.shift = !self.shift;
                        KeyboardEvent::Shift
                    }
                    Key::Space => KeyboardEvent::Space,
                    Key::Backspace => KeyboardEvent::Backspace,
                    Key::Done => KeyboardEvent::Done,
                };
            }
            _ => None,
        };
        match target {
            Some(i) => {
                self.focused = i;
                KeyboardEvent::Moved
            }
            None => KeyboardEvent::None,
        }
    }

    fn nearest_in_row(&self, row: usize, center: f32) -> Option<usize> {
        self.keys
            .iter()
            .enumerate()
            .filter(|(_, k)| k.row == row)
            .min_by(|(_, a), (_, b)| (a.center() - center).abs().total_cmp(&(b.center() - center).abs()))
            .map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qwertz() -> OnScreenKeyboard {
        OnScreenKeyboard::new(Layout::Qwertz, false)
    }

    fn focused_key(k: &OnScreenKeyboard) -> Key {
        k.keys()[k.focused()].key
    }

    #[test]
    fn starts_on_q_and_types_lowercase() {
        let mut k = qwertz();
        assert_eq!(focused_key(&k), Key::Char('Q'));
        assert_eq!(k.apply(Action::Confirm), KeyboardEvent::Type('q'));
        k.apply(Action::Right);
        assert_eq!(k.apply(Action::Confirm), KeyboardEvent::Type('w'));
    }

    #[test]
    fn left_right_stay_in_row() {
        let mut k = qwertz();
        assert_eq!(k.apply(Action::Left), KeyboardEvent::None);
        for _ in 0..20 {
            k.apply(Action::Right);
        }
        assert_eq!(focused_key(&k), Key::Char('Ü'));
    }

    #[test]
    fn up_down_pick_nearest_key() {
        let mut k = qwertz();
        k.apply(Action::Down);
        assert_eq!(focused_key(&k), Key::Char('A'));
        k.apply(Action::Down);
        assert_eq!(focused_key(&k), Key::Char('Y'));
        k.apply(Action::Down);
        assert_eq!(focused_key(&k), Key::Space);
        assert_eq!(k.apply(Action::Down), KeyboardEvent::Done);
        k.apply(Action::Right);
        assert_eq!(k.apply(Action::Confirm), KeyboardEvent::Backspace);
        k.apply(Action::Up);
        assert_eq!(focused_key(&k), Key::Char('M'));
    }

    #[test]
    fn password_keyboard_has_shift_layer() {
        let mut k = OnScreenKeyboard::new(Layout::Qwertz, true);
        assert_eq!(k.apply(Action::Confirm), KeyboardEvent::Type('q'));
        assert_eq!(k.label(k.focused()), "q");
        // To the shift key: all the way down on the left
        for _ in 0..4 {
            k.apply(Action::Down);
        }
        assert_eq!(focused_key(&k), Key::Shift);
        assert_eq!(k.apply(Action::Confirm), KeyboardEvent::Shift);
        assert!(k.shift());
        // Back to the top row: "1" becomes "!"
        for _ in 0..4 {
            k.apply(Action::Up);
        }
        assert_eq!(k.label(k.focused()), "!");
        assert_eq!(k.apply(Action::Confirm), KeyboardEvent::Type('!'));
        k.apply(Action::Down);
        assert_eq!(k.apply(Action::Confirm), KeyboardEvent::Type('Q'));
        k.reset_focus();
        assert!(!k.shift());
    }

    #[test]
    fn search_keyboard_has_no_shift() {
        let k = qwertz();
        assert!(!k.keys().iter().any(|cap| cap.key == Key::Shift));
        assert_eq!(k.label(k.focused()), "Q");
    }

    #[test]
    fn up_from_top_row_does_nothing() {
        let mut k = qwertz();
        k.apply(Action::Up);
        assert_eq!(focused_key(&k), Key::Char('1'));
        assert_eq!(k.apply(Action::Up), KeyboardEvent::None);
    }

    #[test]
    fn qwerty_layout() {
        let mut k = OnScreenKeyboard::new(Layout::Qwerty, true);
        for _ in 0..5 {
            k.apply(Action::Right);
        }
        assert_eq!(focused_key(&k), Key::Char('Y'));
        k.apply(Action::Down);
        k.apply(Action::Down);
        assert_eq!(focused_key(&k), Key::Char('N'));
        k.focused = k.keys().iter().position(|cap| cap.key == Key::Shift).unwrap();
        k.apply(Action::Confirm);
        k.focused = 1;
        assert_eq!(k.label(1), "@");
        assert_eq!(k.apply(Action::Confirm), KeyboardEvent::Type('@'));
    }
}
