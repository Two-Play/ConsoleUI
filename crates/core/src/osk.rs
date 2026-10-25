//! Bildschirmtastatur (QWERTZ) für die Suche mit dem Gamepad. Für Passwörter gibt es eine
//! Variante mit Umschalttaste (Großbuchstaben und Sonderzeichen).

use crate::nav::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Space,
    Backspace,
    Done,
    /// Umschalten zwischen Klein- und Großbuchstaben bzw. Ziffern und Sonderzeichen (nur Passwort)
    Shift,
}

/// Zeichen auf der Umschaltebene (deutsche Belegung)
fn shifted(c: char) -> char {
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

impl Key {
    pub fn label(&self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::Shift => "⇧".into(),
            Key::Space => "Leerzeichen".into(),
            Key::Backspace => "⌫".into(),
            Key::Done => "Fertig".into(),
        }
    }
}

/// Eine Taste mit Position in Tastenbreiten (für das Layout und die Navigation nach oben/unten).
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
    /// Umschalttaste gedrückt; die Beschriftungen haben sich geändert
    Shift,
    /// „Fertig“ gedrückt oder unten aus der Tastatur herausnavigiert
    Done,
}

const LETTER_ROWS: [(&str, f32); 4] =
    [("1234567890", 0.5), ("QWERTZUIOPÜ", 0.0), ("ASDFGHJKLÖÄ", 0.25), ("YXCVBNM-.'", 0.75)];

pub struct OnScreenKeyboard {
    keys: Vec<KeyCap>,
    focused: usize,
    /// Nur mit Umschalttaste: Großbuchstaben/Sonderzeichen aktiv (bleibt an, bis erneut gedrückt)
    shift: bool,
    /// Buchstaben in Groß- und Kleinschreibung tippen (Passwort) statt immer klein (Suche)
    case_sensitive: bool,
}

impl Default for OnScreenKeyboard {
    fn default() -> Self {
        Self::build(false)
    }
}

impl OnScreenKeyboard {
    /// Tastatur für Passwörter: mit Umschalttaste, Buchstaben in beiden Schreibweisen.
    pub fn password() -> Self {
        Self::build(true)
    }

    fn build(password: bool) -> Self {
        let mut keys = Vec::new();
        for (row, (chars, indent)) in LETTER_ROWS.iter().enumerate() {
            for (col, c) in chars.chars().enumerate() {
                keys.push(KeyCap { key: Key::Char(c), row, x: indent + col as f32, width: 1.0 });
            }
        }
        let last = LETTER_ROWS.len();
        if password {
            keys.push(KeyCap { key: Key::Shift, row: last, x: 0.0, width: 2.0 });
            keys.push(KeyCap { key: Key::Space, row: last, x: 2.0, width: 4.0 });
        } else {
            keys.push(KeyCap { key: Key::Space, row: last, x: 0.0, width: 6.0 });
        }
        keys.push(KeyCap { key: Key::Backspace, row: last, x: 6.0, width: 2.5 });
        keys.push(KeyCap { key: Key::Done, row: last, x: 8.5, width: 2.5 });
        let mut keyboard = Self { keys, focused: 0, shift: false, case_sensitive: password };
        keyboard.reset_focus();
        keyboard
    }

    pub fn keys(&self) -> &[KeyCap] {
        &self.keys
    }

    pub fn shift(&self) -> bool {
        self.shift
    }

    /// Beschriftung der Taste `index` passend zur Umschaltebene.
    pub fn label(&self, index: usize) -> String {
        match self.keys[index].key {
            Key::Char(c) if self.case_sensitive => self.char_for(c).to_string(),
            key => key.label(),
        }
    }

    /// Welches Zeichen eine Taste tippt
    fn char_for(&self, c: char) -> char {
        match (self.case_sensitive, self.shift) {
            (true, true) if c.is_alphabetic() => c,
            (true, true) => shifted(c),
            _ => c.to_lowercase().next().unwrap_or(c),
        }
    }

    pub fn focused(&self) -> usize {
        self.focused
    }

    /// Fokus auf „Q“, wie beim Öffnen der Suche.
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
                    // Buchstaben kommen klein in die Suche, die Tasten zeigen sie groß.
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

    fn focused_key(k: &OnScreenKeyboard) -> Key {
        k.keys()[k.focused()].key
    }

    #[test]
    fn starts_on_q_and_types_lowercase() {
        let mut k = OnScreenKeyboard::default();
        assert_eq!(focused_key(&k), Key::Char('Q'));
        assert_eq!(k.apply(Action::Confirm), KeyboardEvent::Type('q'));
        k.apply(Action::Right);
        assert_eq!(k.apply(Action::Confirm), KeyboardEvent::Type('w'));
    }

    #[test]
    fn left_right_stay_in_row() {
        let mut k = OnScreenKeyboard::default();
        assert_eq!(k.apply(Action::Left), KeyboardEvent::None);
        for _ in 0..20 {
            k.apply(Action::Right);
        }
        assert_eq!(focused_key(&k), Key::Char('Ü'));
    }

    #[test]
    fn up_down_pick_nearest_key() {
        let mut k = OnScreenKeyboard::default();
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
        let mut k = OnScreenKeyboard::password();
        assert_eq!(k.apply(Action::Confirm), KeyboardEvent::Type('q'));
        assert_eq!(k.label(k.focused()), "q");
        // Zur Umschalttaste: ganz nach unten links
        for _ in 0..4 {
            k.apply(Action::Down);
        }
        assert_eq!(focused_key(&k), Key::Shift);
        assert_eq!(k.apply(Action::Confirm), KeyboardEvent::Shift);
        assert!(k.shift());
        // Zurück zur obersten Reihe: „1“ wird zu „!“
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
        let k = OnScreenKeyboard::default();
        assert!(!k.keys().iter().any(|cap| cap.key == Key::Shift));
        assert_eq!(k.label(k.focused()), "Q");
    }

    #[test]
    fn up_from_top_row_does_nothing() {
        let mut k = OnScreenKeyboard::default();
        k.apply(Action::Up);
        assert_eq!(focused_key(&k), Key::Char('1'));
        assert_eq!(k.apply(Action::Up), KeyboardEvent::None);
    }
}
