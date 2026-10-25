//! Kontextmenü (Options/Tab) für den fokussierten Eintrag in Home und Bibliothek (Story 2.7).

use consoleui_core::i18n::{tr, trf};
use consoleui_core::{Action, LibrarySort};

use crate::ContextItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextAction {
    Pin,
    Unpin,
    Hide,
    /// Ausgeblendeten Eintrag wieder anzeigen
    Show,
    /// Laufende App beenden (mit der üblichen Rückfrage)
    Quit,
    /// Sortierung der Bibliothek umschalten; das Menü bleibt dabei offen.
    Sort,
}

/// Was der Eintrag gerade ist; daraus ergeben sich die Menüpunkte.
pub struct EntryState {
    pub id: String,
    pub title: String,
    pub pinned: bool,
    pub hidden: bool,
    pub running: bool,
}

pub struct ContextMenu {
    /// Eintrags-ID, auf die sich das Menü bezieht (`None`: leere Bibliothek, nur Sortierung)
    entry_id: Option<String>,
    title: String,
    actions: Vec<ContextAction>,
    focused: usize,
}

/// Was nach einer Eingabe im Kontextmenü passieren soll
pub enum ContextOutcome {
    Stay,
    Close,
    Run(ContextAction),
}

impl ContextMenu {
    /// `library`: in der Bibliothek gehört die Sortierung mit ins Menü.
    pub fn new(entry: Option<EntryState>, library: bool) -> Self {
        let mut actions = Vec::new();
        if let Some(entry) = &entry {
            if entry.hidden {
                actions.push(ContextAction::Show);
            } else {
                actions.push(if entry.pinned { ContextAction::Unpin } else { ContextAction::Pin });
                actions.push(ContextAction::Hide);
            }
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
        assert_eq!(ContextMenu::new(entry(false, false, false), false).actions, [Pin, Hide]);
        assert_eq!(ContextMenu::new(entry(true, false, true), true).actions, [Unpin, Hide, Quit, Sort]);
        assert_eq!(ContextMenu::new(entry(false, true, false), true).actions, [Show, Sort]);
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
        assert_eq!(menu.focused(), 2);
        assert!(matches!(menu.apply(Action::Confirm), ContextOutcome::Run(ContextAction::Quit)));
        assert!(matches!(menu.apply(Action::Back), ContextOutcome::Close));
    }
}
