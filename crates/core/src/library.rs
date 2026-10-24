#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKind {
    Game,
    App,
}

/// Ein Eintrag der App-Übersicht. In Sprint 2 kommen die Einträge aus Steam und `.desktop`-Dateien.
#[derive(Debug, Clone, PartialEq)]
pub struct AppEntry {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub kind: AppKind,
    /// Akzentfarben (RGB) für Platzhalter-Cover und Hintergrund.
    pub accent: [u8; 3],
    pub accent2: [u8; 3],
}

impl AppEntry {
    /// Erstes alphanumerisches Zeichen des Titels als Großbuchstabe (für Platzhalter-Cover).
    pub fn monogram(&self) -> String {
        self.title
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default()
    }
}

/// Titel, Untertitel, Art, Akzentfarbe, zweite Akzentfarbe
type DummyEntry = (&'static str, &'static str, AppKind, [u8; 3], [u8; 3]);

const DUMMY: &[DummyEntry] = &[
    ("Nebula Drift", "Weltraum-Rennspiel", AppKind::Game, [0x5b, 0x3c, 0xff], [0x00, 0xd4, 0xff]),
    ("Ironwood Saga", "Action-Rollenspiel", AppKind::Game, [0x8a, 0x4b, 0x1f], [0xf2, 0xa6, 0x3b]),
    ("Pixel Harbor", "Aufbau-Simulation", AppKind::Game, [0x0f, 0x9d, 0x8a], [0xb4, 0xf0, 0x6b]),
    ("Frostline", "Survival", AppKind::Game, [0x1d, 0x4e, 0x89], [0xd6, 0xf0, 0xff]),
    ("Neon Courier", "Arcade", AppKind::Game, [0xff, 0x2e, 0x88], [0xff, 0xc8, 0x3d]),
    ("Echoes of Tarn", "Abenteuer", AppKind::Game, [0x3a, 0x1c, 0x5a], [0xc9, 0x7b, 0xff]),
    ("Gridlock 2099", "Strategie", AppKind::Game, [0x22, 0x22, 0x22], [0x00, 0xff, 0xa3]),
    ("Mediathek", "Filme & Serien", AppKind::App, [0xc0, 0x1c, 0x28], [0xff, 0x7a, 0x59]),
    ("Musik", "Wiedergabe", AppKind::App, [0x10, 0x80, 0x4b], [0x7c, 0xff, 0xb2]),
    ("Browser", "Im Web surfen", AppKind::App, [0x1a, 0x5f, 0xd6], [0x7a, 0xc4, 0xff]),
    ("Galerie", "Screenshots & Clips", AppKind::App, [0xd9, 0x7a, 0x00], [0xff, 0xe0, 0x7a]),
    ("Einstellungen", "System", AppKind::App, [0x44, 0x4a, 0x55], [0xa9, 0xb1, 0xbd]),
];

/// Erzeugt `count` Dummy-Einträge. Ab dem 13. Eintrag wiederholt sich die Liste mit Nummer im Titel.
pub fn dummy_library(count: usize) -> Vec<AppEntry> {
    (0..count)
        .map(|i| {
            let (title, subtitle, kind, accent, accent2) = DUMMY[i % DUMMY.len()];
            let round = i / DUMMY.len();
            let title = if round == 0 { title.to_string() } else { format!("{title} {}", round + 1) };
            AppEntry {
                id: format!("dummy-{i}"),
                title,
                subtitle: subtitle.to_string(),
                kind,
                accent,
                accent2,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dummy_library_has_requested_size_and_unique_ids() {
        let apps = dummy_library(50);
        assert_eq!(apps.len(), 50);
        let mut ids: Vec<_> = apps.iter().map(|a| &a.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 50);
    }

    #[test]
    fn repeated_entries_get_numbered_titles() {
        let apps = dummy_library(DUMMY.len() + 1);
        assert_eq!(apps[DUMMY.len()].title, format!("{} 2", apps[0].title));
    }

    #[test]
    fn monogram_skips_non_alphanumeric_and_uppercases() {
        let mut app = dummy_library(1).remove(0);
        app.title = "  élan".into();
        assert_eq!(app.monogram(), "É");
        app.title = "".into();
        assert_eq!(app.monogram(), "");
    }
}
