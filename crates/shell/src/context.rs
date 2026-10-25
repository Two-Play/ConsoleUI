//! Context menu (Options/Tab) for the focused entry in home and library (Story 2.7).

use consoleui_core::i18n::{tr, trf};
use consoleui_core::{Action, LibrarySort};

use crate::ContextItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextAction {
    Pin,
    Unpin,
    Hide,
    /// Show a hidden entry again
    Show,
    /// Open the nested gamescope options of the entry (Story 7.10)
    GameOptions,
    /// Quit the running app (with the usual confirmation)
    Quit,
    /// Switch the library sort; the menu stays open.
    Sort,
}

/// What the entry currently is; the menu items follow from it.
pub struct EntryState {
    pub id: String,
    pub title: String,
    pub pinned: bool,
    pub hidden: bool,
    pub running: bool,
}

pub struct ContextMenu {
    /// Entry ID the menu refers to (`None`: empty library, only the sort)
    entry_id: Option<String>,
    title: String,
    actions: Vec<ContextAction>,
    focused: usize,
}

/// What should happen after an input in the context menu
pub enum ContextOutcome {
    Stay,
    Close,
    Run(ContextAction),
}

impl ContextMenu {
    /// `library`: in the library the sort is part of the menu.
    pub fn new(entry: Option<EntryState>, library: bool) -> Self {
        let mut actions = Vec::new();
        if let Some(entry) = &entry {
            if entry.hidden {
                actions.push(ContextAction::Show);
            } else {
                actions.push(if entry.pinned { ContextAction::Unpin } else { ContextAction::Pin });
                actions.push(ContextAction::Hide);
            }
            actions.push(ContextAction::GameOptions);
            if entry.running {
                actions.push(ContextAction::Quit);
            }
        }
        if library {
            actions.push(ContextAction::Sort);
        }
        let title = entry.as_ref().map_or_else(|| tr("Library").to_string(), |e| e.title.clone());
        Self { entry_id: entry.map(|e| e.id), title, actions, focused: 0 }
    }

    pub fn entry_id(&self) -> Option<&str> {
        self.entry_id.as_deref()
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn focused(&self) -> usize {
        self.focused
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    pub fn apply(&mut self, action: Action) -> ContextOutcome {
        match action {
            Action::Up => self.focused = self.focused.saturating_sub(1),
            Action::Down => self.focused = (self.focused + 1).min(self.actions.len().saturating_sub(1)),
            Action::Confirm => return ContextOutcome::Run(self.actions[self.focused]),
            Action::Back | Action::Menu => return ContextOutcome::Close,
            _ => {}
        }
        ContextOutcome::Stay
    }

    pub fn items(&self, sort: LibrarySort) -> Vec<ContextItem> {
        self.actions
            .iter()
            .map(|action| {
                let (symbol, label) = match action {
                    ContextAction::Pin => ("⚲", tr("Pin").to_string()),
                    ContextAction::Unpin => ("⚲", tr("Unpin").to_string()),
                    ContextAction::Hide => ("◌", tr("Hide").to_string()),
                    ContextAction::Show => ("●", tr("Show again").to_string()),
                    ContextAction::GameOptions => ("▣", tr("Gamescope options…").to_string()),
                    ContextAction::Quit => ("■", tr("Quit").to_string()),
                    ContextAction::Sort => ("⇅", trf("Sort: {}", &[&sort.label()])),
                };
                ContextItem { symbol: symbol.into(), label: label.into() }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pinned: bool, hidden: bool, running: bool) -> Option<EntryState> {
        Some(EntryState { id: "steam:730".into(), title: "CS2".into(), pinned, hidden, running })
    }

    #[test]
    fn items_depend_on_entry_state() {
        use ContextAction::*;
        assert_eq!(ContextMenu::new(entry(false, false, false), false).actions, [Pin, Hide, GameOptions]);
        assert_eq!(ContextMenu::new(entry(true, false, true), true).actions, [Unpin, Hide, GameOptions, Quit, Sort]);
        assert_eq!(ContextMenu::new(entry(false, true, false), true).actions, [Show, GameOptions, Sort]);
        assert_eq!(ContextMenu::new(None, true).actions, [Sort]);
        assert!(ContextMenu::new(None, false).is_empty());
    }

    #[test]
    fn navigates_and_runs() {
        let mut menu = ContextMenu::new(entry(false, false, true), false);
        menu.apply(Action::Up);
        assert_eq!(menu.focused(), 0);
        for _ in 0..5 {
            menu.apply(Action::Down);
        }
        assert_eq!(menu.focused(), 3);
        assert!(matches!(menu.apply(Action::Confirm), ContextOutcome::Run(ContextAction::Quit)));
        assert!(matches!(menu.apply(Action::Back), ContextOutcome::Close));
    }
}
