//! The "Who's playing?" screen (Story 5.7): a row of profile tiles plus "Add profile", a menu per
//! profile (rename, change color, delete), the name entry with the on-screen keyboard and the
//! confirmation before deleting. Without UI; the shell shows [`PickerView`] and saves the
//! [`Profiles`] when an input reports a change.

use crate::i18n::{tr, trf};
use crate::nav::Action;
use crate::osk::{KeyboardEvent, OnScreenKeyboard};
use crate::profiles::{MAX_NAME, Profiles};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Rename,
    Color,
    Delete,
}

impl MenuItem {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Rename => "✎",
            Self::Color => "◐",
            Self::Delete => "✕",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Rename => tr("Rename"),
            Self::Color => tr("Change color"),
            Self::Delete => tr("Delete profile"),
        }
    }
}

enum Mode {
    Pick,
    Menu {
        id: String,
        items: Vec<MenuItem>,
        focused: usize,
    },
    /// `id`: `None` for a new profile, otherwise the profile being renamed
    Name {
        id: Option<String>,
        text: String,
        keyboard: OnScreenKeyboard,
    },
    ConfirmDelete(String),
}

/// What the shell has to do after an input
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    Stay,
    /// Back to the shell with the current profile
    Close,
    /// Play as this profile
    Select(String),
    /// The profile list changed and must be saved
    Changed,
    /// The profile was removed from the list (save it and delete its files)
    Deleted(String),
}

/// A tile of the row
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileView {
    pub name: String,
    pub initial: String,
    pub rgb: [u8; 3],
    /// The profile the shell runs with right now
    pub current: bool,
    /// The "Add profile" tile
    pub add: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerView {
    pub title: String,
    pub tiles: Vec<TileView>,
    pub focused: usize,
    pub menu_title: String,
    /// (symbol, label) of the menu items; empty while the menu is closed
    pub menu: Vec<(&'static str, &'static str)>,
    pub menu_focused: usize,
    pub name_open: bool,
    pub name_title: String,
    pub name_text: String,
    pub key_labels: Vec<String>,
    pub key_focused: usize,
    /// Question before deleting; empty while not asking
    pub confirm: String,
}

pub struct ProfilePicker {
    focused: usize,
    mode: Mode,
    /// Profile the shell runs with; `None` while nobody has chosen yet
    current: Option<String>,
    /// Opened at the start: "Back" does not lead anywhere, a profile must be chosen
    startup: bool,
}

impl ProfilePicker {
    /// `current` gets the focus; `startup`: shown before the shell, no way back.
    pub fn new(profiles: &Profiles, current: Option<&str>, startup: bool) -> Self {
        let focused = current.and_then(|id| profiles.list().iter().position(|p| p.id == id)).unwrap_or(0);
        Self { focused, mode: Mode::Pick, current: current.map(String::from), startup }
    }

    pub fn is_startup(&self) -> bool {
        self.startup
    }

    /// The name entry is open (the real keyboard types there).
    pub fn is_typing(&self) -> bool {
        matches!(self.mode, Mode::Name { .. })
    }

    pub fn is_menu_open(&self) -> bool {
        matches!(self.mode, Mode::Menu { .. })
    }

    pub fn is_confirming(&self) -> bool {
        matches!(self.mode, Mode::ConfirmDelete(_))
    }

    /// Focus positions for the sound feedback
    pub fn feedback(&self) -> [usize; 4] {
        let (mode, focus, len) = match &self.mode {
            Mode::Pick => (0, 0, 0),
            Mode::Menu { focused, .. } => (1, *focused, 0),
            Mode::Name { text, keyboard, .. } => (2, keyboard.focused(), text.len()),
            Mode::ConfirmDelete(_) => (3, 0, 0),
        };
        [mode, self.focused, focus, len]
    }

    /// Profile tiles plus the "Add profile" tile while there is room
    fn tile_count(&self, profiles: &Profiles) -> usize {
        profiles.len() + usize::from(!profiles.is_full())
    }

    fn focused_profile<'a>(&self, profiles: &'a Profiles) -> Option<&'a str> {
        profiles.list().get(self.focused).map(|p| p.id.as_str())
    }

    pub fn apply(&mut self, profiles: &mut Profiles, action: Action) -> PickerOutcome {
        match std::mem::replace(&mut self.mode, Mode::Pick) {
            Mode::Pick => self.apply_pick(profiles, action),
            Mode::Menu { id, items, focused } => self.apply_menu(profiles, action, id, items, focused),
            Mode::Name { id, text, keyboard } => self.apply_name(profiles, action, id, text, keyboard),
            Mode::ConfirmDelete(id) => match action {
                Action::Confirm if profiles.remove(&id) => {
                    self.focused = self.focused.min(self.tile_count(profiles) - 1);
                    if self.current.as_deref() == Some(id.as_str()) {
                        self.current = None;
                    }
                    PickerOutcome::Deleted(id)
                }
                Action::Confirm | Action::Back => PickerOutcome::Stay,
                _ => {
                    self.mode = Mode::ConfirmDelete(id);
                    PickerOutcome::Stay
                }
            },
        }
    }

    fn apply_pick(&mut self, profiles: &Profiles, action: Action) -> PickerOutcome {
        match action {
            Action::Left => self.focused = self.focused.saturating_sub(1),
            Action::Right => self.focused = (self.focused + 1).min(self.tile_count(profiles) - 1),
            Action::Confirm => match self.focused_profile(profiles) {
                Some(id) => return PickerOutcome::Select(id.to_string()),
                None => self.open_name(None, String::new()),
            },
            Action::Menu => {
                if let Some(id) = self.focused_profile(profiles) {
                    // The running profile cannot delete itself; at the start nobody runs yet.
                    let deletable = profiles.len() > 1 && (self.startup || self.current.as_deref() != Some(id));
                    let mut items = vec![MenuItem::Rename, MenuItem::Color];
                    if deletable {
                        items.push(MenuItem::Delete);
                    }
                    self.mode = Mode::Menu { id: id.to_string(), items, focused: 0 };
                }
            }
            Action::Back if !self.startup => return PickerOutcome::Close,
            _ => {}
        }
        PickerOutcome::Stay
    }

    fn apply_menu(
        &mut self,
        profiles: &mut Profiles,
        action: Action,
        id: String,
        items: Vec<MenuItem>,
        focused: usize,
    ) -> PickerOutcome {
        match action {
            Action::Up => self.mode = Mode::Menu { id, focused: focused.saturating_sub(1), items },
            Action::Down => self.mode = Mode::Menu { id, focused: (focused + 1).min(items.len() - 1), items },
            Action::Confirm => match items[focused] {
                MenuItem::Rename => {
                    let name = profiles.get(&id).map(|p| p.name.clone()).unwrap_or_default();
                    self.open_name(Some(id), name);
                }
                MenuItem::Color => {
                    // The menu stays open, so the colors can be stepped through.
                    profiles.next_color(&id);
                    self.mode = Mode::Menu { id, items, focused };
                    return PickerOutcome::Changed;
                }
                MenuItem::Delete => self.mode = Mode::ConfirmDelete(id),
            },
            Action::Back | Action::Menu => {}
            _ => self.mode = Mode::Menu { id, items, focused },
        }
        PickerOutcome::Stay
    }

    fn open_name(&mut self, id: Option<String>, text: String) {
        self.mode = Mode::Name { id, text, keyboard: OnScreenKeyboard::password() };
    }

    fn apply_name(
        &mut self,
        profiles: &mut Profiles,
        action: Action,
        id: Option<String>,
        mut text: String,
        mut keyboard: OnScreenKeyboard,
    ) -> PickerOutcome {
        let event = match action {
            Action::Back => return PickerOutcome::Stay,
            Action::Secondary => KeyboardEvent::Backspace,
            Action::Search => KeyboardEvent::Space,
            _ => keyboard.apply(action),
        };
        match event {
            KeyboardEvent::Type(c) => push_char(&mut text, c),
            KeyboardEvent::Space => push_char(&mut text, ' '),
            KeyboardEvent::Backspace => {
                text.pop();
            }
            // "Done" saves; navigating down out of the keyboard does nothing.
            KeyboardEvent::Done if action == Action::Confirm => {
                let saved = match &id {
                    None => profiles.add(&text).map(|new| {
                        self.focused = profiles.list().iter().position(|p| p.id == new).unwrap_or(0);
                    }),
                    Some(id) => profiles.rename(id, &text).then_some(()),
                };
                if saved.is_some() {
                    return PickerOutcome::Changed;
                }
            }
            _ => {}
        }
        self.mode = Mode::Name { id, text, keyboard };
        PickerOutcome::Stay
    }

    /// Text from the real keyboard during the name entry; false if not taken.
    pub fn type_text(&mut self, text: &str) -> bool {
        let Mode::Name { text: name, .. } = &mut self.mode else { return false };
        for c in text.chars() {
            push_char(name, c);
        }
        true
    }

    pub fn erase(&mut self) -> bool {
        let Mode::Name { text, .. } = &mut self.mode else { return false };
        text.pop();
        true
    }

    pub fn view(&self, profiles: &Profiles) -> PickerView {
        let mut tiles: Vec<TileView> = profiles
            .list()
            .iter()
            .map(|p| TileView {
                name: p.name.clone(),
                initial: p.initial(),
                rgb: p.rgb(),
                current: self.current.as_deref() == Some(p.id.as_str()),
                add: false,
            })
            .collect();
        if !profiles.is_full() {
            tiles.push(TileView {
                name: tr("Add profile").into(),
                initial: "+".into(),
                rgb: [0x2a, 0x2e, 0x3a],
                current: false,
                add: true,
            });
        }
        let title = if self.startup { tr("Who's playing?") } else { tr("Switch profile") };
        let mut view = PickerView { title: title.into(), tiles, focused: self.focused, ..PickerView::default() };
        let name_of = |id: &str| profiles.get(id).map(|p| p.name.clone()).unwrap_or_default();
        match &self.mode {
            Mode::Pick => {}
            Mode::Menu { id, items, focused } => {
                view.menu_title = name_of(id).to_uppercase();
                view.menu = items.iter().map(|item| (item.symbol(), item.label())).collect();
                view.menu_focused = *focused;
            }
            Mode::Name { id, text, keyboard } => {
                view.name_open = true;
                view.name_title = if id.is_some() { tr("Rename profile") } else { tr("New profile") }.into();
                view.name_text = text.clone();
                view.key_labels = (0..keyboard.keys().len()).map(|i| keyboard.label(i)).collect();
                view.key_focused = keyboard.focused();
            }
            Mode::ConfirmDelete(id) => {
                view.confirm = trf("Delete “{}” with its history, pins and settings?", &[&name_of(id)]);
            }
        }
        view
    }
}

/// Appends a character up to [`MAX_NAME`]; a word starts with a capital letter.
fn push_char(text: &mut String, c: char) {
    if text.chars().count() >= MAX_NAME || c.is_control() || (c == ' ' && (text.is_empty() || text.ends_with(' '))) {
        return;
    }
    if text.is_empty() || text.ends_with(' ') {
        text.extend(c.to_uppercase());
    } else {
        text.push(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> Profiles {
        let mut profiles = Profiles::default();
        profiles.add("Anna").unwrap();
        profiles.add("Ben").unwrap();
        profiles
    }

    fn press(picker: &mut ProfilePicker, profiles: &mut Profiles, actions: &[Action]) -> PickerOutcome {
        actions.iter().fold(PickerOutcome::Stay, |_, &a| picker.apply(profiles, a))
    }

    #[test]
    fn pick_a_profile() {
        let mut profiles = two();
        let mut picker = ProfilePicker::new(&profiles, Some("ben"), true);
        assert_eq!(picker.view(&profiles).focused, 1);
        assert!(picker.view(&profiles).tiles[1].current);
        // Back goes nowhere at the start
        assert_eq!(picker.apply(&mut profiles, Action::Back), PickerOutcome::Stay);
        assert_eq!(
            press(&mut picker, &mut profiles, &[Action::Left, Action::Left, Action::Confirm]),
            PickerOutcome::Select("anna".into())
        );
    }

    #[test]
    fn back_closes_when_opened_from_the_shell() {
        let mut profiles = two();
        let mut picker = ProfilePicker::new(&profiles, Some("anna"), false);
        assert_ne!(picker.view(&profiles).title, ProfilePicker::new(&profiles, None, true).view(&profiles).title);
        assert_eq!(picker.apply(&mut profiles, Action::Back), PickerOutcome::Close);
    }

    #[test]
    fn add_a_profile_with_the_keyboard() {
        let mut profiles = two();
        let mut picker = ProfilePicker::new(&profiles, Some("anna"), true);
        // Right past the last profile to "Add profile", then no further
        press(&mut picker, &mut profiles, &[Action::Right, Action::Right, Action::Right]);
        assert!(picker.view(&profiles).tiles[2].add);
        assert_eq!(picker.view(&profiles).focused, 2);
        picker.apply(&mut profiles, Action::Confirm);
        assert!(picker.is_typing());
        // The keyboard opens on "Q": typed as a capital at the start of a word
        picker.apply(&mut profiles, Action::Confirm);
        assert!(picker.type_text("uinn"));
        assert_eq!(picker.view(&profiles).name_text, "Quinn");
        // □ deletes, △ is a space; Back cancels
        press(&mut picker, &mut profiles, &[Action::Secondary, Action::Search]);
        assert_eq!(picker.view(&profiles).name_text, "Quin ");
        assert!(picker.erase());
        // "Done" is reached by going down to the last row and right
        let done = press(&mut picker, &mut profiles, &[Action::Down, Action::Down, Action::Down, Action::Down]);
        assert_eq!(done, PickerOutcome::Stay);
        press(&mut picker, &mut profiles, &[Action::Right, Action::Right, Action::Right]);
        assert_eq!(picker.apply(&mut profiles, Action::Confirm), PickerOutcome::Changed);
        assert!(!picker.is_typing());
        assert_eq!(profiles.list()[2].name, "Quin");
        assert_eq!(picker.view(&profiles).focused, 2, "the new profile is focused");
    }

    #[test]
    fn empty_name_is_not_saved_and_back_cancels() {
        let mut profiles = two();
        let mut picker = ProfilePicker::new(&profiles, None, true);
        press(&mut picker, &mut profiles, &[Action::Right, Action::Right, Action::Confirm]);
        assert!(picker.is_typing());
        assert!(picker.type_text(" "));
        assert!(picker.view(&profiles).name_text.is_empty());
        assert_eq!(picker.apply(&mut profiles, Action::Back), PickerOutcome::Stay);
        assert!(!picker.is_typing());
        assert_eq!(profiles.len(), 2);
    }

    #[test]
    fn menu_rename_color_and_delete() {
        let mut profiles = two();
        let mut picker = ProfilePicker::new(&profiles, Some("anna"), false);
        picker.apply(&mut profiles, Action::Menu);
        // The running profile cannot be deleted from inside the shell
        assert_eq!(picker.view(&profiles).menu.len(), 2);
        assert_eq!(picker.view(&profiles).menu_title, "ANNA");
        assert_eq!(press(&mut picker, &mut profiles, &[Action::Down, Action::Confirm]), PickerOutcome::Changed);
        assert_eq!(profiles.get("anna").unwrap().color, 1);
        assert!(picker.is_menu_open());
        // Rename: the entry starts with the old name
        press(&mut picker, &mut profiles, &[Action::Up, Action::Confirm]);
        assert_eq!(picker.view(&profiles).name_text, "Anna");
        picker.apply(&mut profiles, Action::Back);

        // Ben can be deleted, after a confirmation
        press(&mut picker, &mut profiles, &[Action::Right, Action::Menu, Action::Down, Action::Down]);
        assert_eq!(picker.view(&profiles).menu.len(), 3);
        picker.apply(&mut profiles, Action::Confirm);
        assert!(picker.is_confirming());
        assert!(picker.view(&profiles).confirm.contains("Ben"));
        assert_eq!(picker.apply(&mut profiles, Action::Back), PickerOutcome::Stay);
        assert!(!picker.is_confirming() && profiles.len() == 2);
        press(&mut picker, &mut profiles, &[Action::Menu, Action::Down, Action::Down, Action::Confirm]);
        assert_eq!(picker.apply(&mut profiles, Action::Confirm), PickerOutcome::Deleted("ben".into()));
        assert_eq!(profiles.len(), 1);
        assert_eq!(picker.view(&profiles).focused, 1, "focus moves to the add tile");

        // The last profile has no delete item
        press(&mut picker, &mut profiles, &[Action::Left, Action::Menu]);
        assert_eq!(picker.view(&profiles).menu.len(), 2);
    }

    #[test]
    fn at_the_start_every_profile_can_be_deleted() {
        let mut profiles = two();
        let mut picker = ProfilePicker::new(&profiles, Some("anna"), true);
        press(&mut picker, &mut profiles, &[Action::Menu, Action::Down, Action::Down, Action::Confirm]);
        assert_eq!(picker.apply(&mut profiles, Action::Confirm), PickerOutcome::Deleted("anna".into()));
        assert!(picker.view(&profiles).tiles.iter().all(|t| !t.current));
    }

    #[test]
    fn names_capitalize_words_and_stop_at_the_limit() {
        let mut text = String::new();
        for c in " anna  lena".chars() {
            push_char(&mut text, c);
        }
        assert_eq!(text, "Anna Lena");
        for c in "x".repeat(30).chars() {
            push_char(&mut text, c);
        }
        assert_eq!(text.chars().count(), MAX_NAME);
    }

    #[test]
    fn full_list_has_no_add_tile() {
        let mut profiles = Profiles::default();
        for n in 0..crate::profiles::MAX_PROFILES {
            profiles.add(&format!("P{n}")).unwrap();
        }
        let picker = ProfilePicker::new(&profiles, None, true);
        assert!(picker.view(&profiles).tiles.iter().all(|t| !t.add));
    }
}
