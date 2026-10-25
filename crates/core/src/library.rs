use std::path::PathBuf;

use chrono::NaiveDate;

use crate::collection::Collection;
use crate::history::History;
use crate::search::normalize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKind {
    Game,
    App,
}

/// Wie ein Eintrag gestartet wird.
#[derive(Debug, Clone, PartialEq)]
pub enum Launch {
    /// Dummy-Einträge: nur die Animation, kein Prozess.
    None,
    Steam(u32),
    /// Programm und Argumente, bereits ohne `.desktop`-Feldcodes.
    Command(Vec<String>),
}

/// Pfade zu Bildern auf der Platte; geladen werden sie erst in der Shell.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Artwork {
    pub cover: Option<PathBuf>,
    pub hero: Option<PathBuf>,
    pub logo: Option<PathBuf>,
    pub icon: Option<PathBuf>,
}

/// Ein Eintrag der App-Übersicht (Steam-Spiel, Desktop-App oder Dummy).
#[derive(Debug, Clone, PartialEq)]
pub struct AppEntry {
    /// Eindeutig über alle Quellen, z. B. `steam:730` oder `desktop:firefox.desktop`.
    pub id: String,
    pub title: String,
    pub description: String,
    pub kind: AppKind,
    /// Akzentfarben (RGB) für Platzhalter-Cover und Hintergrund.
    pub accent: [u8; 3],
    pub accent2: [u8; 3],
    pub launch: Launch,
    pub artwork: Artwork,
    /// Letzte Nutzung als Unix-Zeit in Sekunden.
    pub last_used: Option<u64>,
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

/// Titel, Beschreibung, Art, Akzentfarbe, zweite Akzentfarbe
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

/// Feste Akzentfarben pro Schlüssel, für Einträge ohne eigenes Artwork.
pub fn palette_for(key: &str) -> ([u8; 3], [u8; 3]) {
    // FNV-1a: stabil über Programmläufe, anders als `DefaultHasher`.
    let hash = key.bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    let (_, _, _, accent, accent2) = DUMMY[hash as usize % DUMMY.len()];
    (accent, accent2)
}

/// Erzeugt `count` Dummy-Einträge. Ab dem 13. Eintrag wiederholt sich die Liste mit Nummer im Titel.
pub fn dummy_library(count: usize) -> Vec<AppEntry> {
    (0..count)
        .map(|i| {
            let (title, description, kind, accent, accent2) = DUMMY[i % DUMMY.len()];
            let round = i / DUMMY.len();
            let title = if round == 0 { title.to_string() } else { format!("{title} {}", round + 1) };
            AppEntry {
                id: format!("dummy:{i}"),
                title,
                description: description.to_string(),
                kind,
                accent,
                accent2,
                launch: Launch::None,
                artwork: Artwork::default(),
                last_used: None,
            }
        })
        .collect()
}

/// Übernimmt die eigene Historie, wenn sie neuer ist als die Angabe der Quelle (z. B. Steam).
pub fn apply_history(entries: &mut [AppEntry], history: &History) {
    for entry in entries {
        entry.last_used = entry.last_used.max(history.get(&entry.id));
    }
}

/// Zuletzt genutzte zuerst, danach alphabetisch.
pub fn sort_by_recent(entries: &mut [AppEntry]) {
    entries.sort_by(|a, b| {
        b.last_used
            .cmp(&a.last_used)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryFilter {
    #[default]
    All,
    Games,
    Apps,
    /// Nur die ausgeblendeten Einträge, um sie wieder anzuzeigen
    Hidden,
}

impl LibraryFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Games,
            Self::Games => Self::Apps,
            Self::Apps => Self::Hidden,
            Self::Hidden => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Alle",
            Self::Games => "Spiele",
            Self::Apps => "Apps",
            Self::Hidden => "Ausgeblendet",
        }
    }

    fn matches(self, entry: &AppEntry, collection: &Collection) -> bool {
        let hidden = collection.is_hidden(&entry.id);
        match self {
            Self::All => !hidden,
            Self::Games => !hidden && entry.kind == AppKind::Game,
            Self::Apps => !hidden && entry.kind == AppKind::App,
            Self::Hidden => hidden,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibrarySort {
    #[default]
    Recent,
    Alphabetical,
}

impl LibrarySort {
    pub fn next(self) -> Self {
        match self {
            Self::Recent => Self::Alphabetical,
            Self::Alphabetical => Self::Recent,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Recent => "Zuletzt genutzt",
            Self::Alphabetical => "A–Z",
        }
    }
}

/// Indizes in `entries` für die Bibliothek. `entries` ist bereits nach letzter Nutzung sortiert.
/// Ausgeblendete erscheinen nur im Filter „Ausgeblendet“; Angeheftete behalten die normale Sortierung.
pub fn library_view(
    entries: &[AppEntry],
    filter: LibraryFilter,
    sort: LibrarySort,
    collection: &Collection,
) -> Vec<usize> {
    let mut view: Vec<usize> = (0..entries.len()).filter(|&i| filter.matches(&entries[i], collection)).collect();
    if sort == LibrarySort::Alphabetical {
        view.sort_by_cached_key(|&i| normalize(&entries[i].title));
    }
    view
}

/// Zeile unter dem Titel, z. B. „Zuletzt gespielt: gestern“.
pub fn last_used_label(kind: AppKind, last: NaiveDate, today: NaiveDate) -> String {
    let verb = match kind {
        AppKind::Game => "gespielt",
        AppKind::App => "geöffnet",
    };
    let when = match (today - last).num_days() {
        ..=0 => "heute".to_string(),
        1 => "gestern".to_string(),
        days @ 2..=6 => format!("vor {days} Tagen"),
        _ => format!("am {}", last.format("%d.%m.%Y")),
    };
    format!("Zuletzt {verb}: {when}")
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

    #[test]
    fn palette_is_stable() {
        assert_eq!(palette_for("steam:730"), palette_for("steam:730"));
    }

    #[test]
    fn sorts_recent_first_then_alphabetically() {
        let mut apps = dummy_library(4);
        apps[0].title = "b".into();
        apps[1].title = "A".into();
        apps[2].last_used = Some(10);
        apps[3].last_used = Some(20);
        let mut history = History::default();
        history.record(&apps[2].id, 30);
        apply_history(&mut apps, &history);
        sort_by_recent(&mut apps);
        let titles: Vec<_> = apps.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(titles, ["Pixel Harbor", "Frostline", "A", "b"]);
    }

    #[test]
    fn library_view_filters_and_sorts() {
        let mut apps = dummy_library(4);
        apps[0].title = "Zebra".into();
        apps[1].title = "Ärger".into();
        apps[2].kind = AppKind::App;
        apps[2].title = "Browser".into();
        apps[3].kind = AppKind::App;
        apps[3].title = "anno".into();
        let none = Collection::default();
        assert_eq!(library_view(&apps, LibraryFilter::All, LibrarySort::Recent, &none), [0, 1, 2, 3]);
        assert_eq!(library_view(&apps, LibraryFilter::Games, LibrarySort::Alphabetical, &none), [1, 0]);
        assert_eq!(library_view(&apps, LibraryFilter::Apps, LibrarySort::Alphabetical, &none), [3, 2]);
        assert_eq!(library_view(&apps, LibraryFilter::Hidden, LibrarySort::Recent, &none), Vec::<usize>::new());
        assert_eq!(LibraryFilter::Apps.next(), LibraryFilter::Hidden);
        assert_eq!(LibraryFilter::Hidden.next(), LibraryFilter::All);
    }

    #[test]
    fn library_view_hides_and_keeps_pinned_in_normal_order() {
        let apps = dummy_library(4);
        let mut collection = Collection::default();
        collection.hide("dummy:1");
        collection.pin("dummy:3");
        assert_eq!(library_view(&apps, LibraryFilter::All, LibrarySort::Recent, &collection), [0, 2, 3]);
        assert_eq!(library_view(&apps, LibraryFilter::Games, LibrarySort::Recent, &collection), [0, 2, 3]);
        assert_eq!(library_view(&apps, LibraryFilter::Hidden, LibrarySort::Recent, &collection), [1]);
    }

    #[test]
    fn last_used_labels() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let day = |d| NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
        assert_eq!(last_used_label(AppKind::Game, day(25), today), "Zuletzt gespielt: heute");
        assert_eq!(last_used_label(AppKind::App, day(24), today), "Zuletzt geöffnet: gestern");
        assert_eq!(last_used_label(AppKind::Game, day(20), today), "Zuletzt gespielt: vor 5 Tagen");
        assert_eq!(last_used_label(AppKind::Game, day(1), today), "Zuletzt gespielt: am 01.10.2026");
    }
}
