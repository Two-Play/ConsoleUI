use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::NaiveDate;

use crate::collection::Collection;
use crate::desktop::DesktopNames;
use crate::history::History;
use crate::i18n::{self, Language, tr, tr_noop, trf, trn};
use crate::search::normalize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKind {
    Game,
    App,
}

/// How an entry is launched.
#[derive(Debug, Clone, PartialEq)]
pub enum Launch {
    /// Dummy entries: only the animation, no process.
    None,
    Steam(u32),
    /// Program and arguments, already without `.desktop` field codes.
    Command(Vec<String>),
}

/// Paths to images on disk; they are only loaded in the shell.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Artwork {
    pub cover: Option<PathBuf>,
    pub hero: Option<PathBuf>,
    pub logo: Option<PathBuf>,
    pub icon: Option<PathBuf>,
}

/// Where the title and description of an entry come from, so they can follow a runtime language
/// switch ([`AppEntry::localize`], Story 6.6) without discovering the library again.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Names {
    /// Fixed texts (Steam games: Steam has no localized names in the manifests)
    #[default]
    Fixed,
    /// Dummy entry: row `index` of the dummy table; `round` > 0 adds a number to the title.
    Dummy { index: usize, round: usize },
    /// Localized keys of a `.desktop` file; shared, so cloning the entry stays cheap.
    Desktop(Arc<DesktopNames>),
}

/// An entry of the app overview (Steam game, desktop app or dummy).
#[derive(Debug, Clone, PartialEq)]
pub struct AppEntry {
    /// Unique across all sources, e.g. `steam:730` or `desktop:firefox.desktop`.
    pub id: String,
    /// Title and description in the current language (resolved from `names`). Search, sorting
    /// and every other display use these.
    pub title: String,
    pub description: String,
    pub names: Names,
    pub kind: AppKind,
    /// Accent colors (RGB) for placeholder covers and the background.
    pub accent: [u8; 3],
    pub accent2: [u8; 3],
    pub launch: Launch,
    pub artwork: Artwork,
    /// Last use as Unix time in seconds.
    pub last_used: Option<u64>,
}

impl AppEntry {
    /// First alphanumeric character of the title in upper case (for placeholder covers).
    pub fn monogram(&self) -> String {
        self.title
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default()
    }

    /// Resolves `title` and `description` again for `language`; `desktop_langs` are the locale
    /// codes for `.desktop` keys ([`i18n::desktop_languages`]). Returns whether anything changed.
    pub fn localize(&mut self, language: Language, desktop_langs: &[String]) -> bool {
        let (title, description) = match &self.names {
            Names::Fixed => return false,
            Names::Dummy { index, round } => dummy_texts(*index, *round, language),
            Names::Desktop(names) => names.resolve(desktop_langs),
        };
        let changed = title != self.title || description != self.description;
        self.title = title;
        self.description = description;
        changed
    }
}

/// Resolves all entries for `language` (after a language switch). Returns whether any title or
/// description changed; the order may then be stale (see [`compare_recent`]).
pub fn localize_entries(entries: &mut [AppEntry], language: Language) -> bool {
    let desktop_langs = i18n::desktop_languages(language, crate::xdg::languages());
    entries.iter_mut().fold(false, |changed, entry| entry.localize(language, &desktop_langs) | changed)
}

/// Title, description, kind, accent color, second accent color. App titles and all descriptions
/// are translated by [`dummy_texts`].
type DummyEntry = (&'static str, &'static str, AppKind, [u8; 3], [u8; 3]);

const DUMMY: &[DummyEntry] = &[
    ("Nebula Drift", tr_noop("Space racer"), AppKind::Game, [0x5b, 0x3c, 0xff], [0x00, 0xd4, 0xff]),
    ("Ironwood Saga", tr_noop("Action RPG"), AppKind::Game, [0x8a, 0x4b, 0x1f], [0xf2, 0xa6, 0x3b]),
    ("Pixel Harbor", tr_noop("City builder"), AppKind::Game, [0x0f, 0x9d, 0x8a], [0xb4, 0xf0, 0x6b]),
    ("Frostline", tr_noop("Survival"), AppKind::Game, [0x1d, 0x4e, 0x89], [0xd6, 0xf0, 0xff]),
    ("Neon Courier", tr_noop("Arcade"), AppKind::Game, [0xff, 0x2e, 0x88], [0xff, 0xc8, 0x3d]),
    ("Echoes of Tarn", tr_noop("Adventure"), AppKind::Game, [0x3a, 0x1c, 0x5a], [0xc9, 0x7b, 0xff]),
    ("Gridlock 2099", tr_noop("Strategy"), AppKind::Game, [0x22, 0x22, 0x22], [0x00, 0xff, 0xa3]),
    (tr_noop("Media"), tr_noop("Movies & shows"), AppKind::App, [0xc0, 0x1c, 0x28], [0xff, 0x7a, 0x59]),
    (tr_noop("Music"), tr_noop("Playback"), AppKind::App, [0x10, 0x80, 0x4b], [0x7c, 0xff, 0xb2]),
    (tr_noop("Browser"), tr_noop("Browse the web"), AppKind::App, [0x1a, 0x5f, 0xd6], [0x7a, 0xc4, 0xff]),
    (tr_noop("Gallery"), tr_noop("Screenshots & clips"), AppKind::App, [0xd9, 0x7a, 0x00], [0xff, 0xe0, 0x7a]),
    (tr_noop("Settings"), tr_noop("System"), AppKind::App, [0x44, 0x4a, 0x55], [0xa9, 0xb1, 0xbd]),
];

/// Fixed accent colors per key, for entries without their own artwork.
pub fn palette_for(key: &str) -> ([u8; 3], [u8; 3]) {
    // FNV-1a: stable across runs, unlike `DefaultHasher`.
    let hash = key.bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    let (_, _, _, accent, accent2) = DUMMY[hash as usize % DUMMY.len()];
    (accent, accent2)
}

/// Title and description of a dummy entry in `language`; game titles are names and stay as they are.
fn dummy_texts(index: usize, round: usize, language: Language) -> (String, String) {
    let (title, description, kind, _, _) = DUMMY[index];
    let title = if kind == AppKind::App { i18n::tr_in(language, title) } else { title };
    let title = if round == 0 { title.to_string() } else { format!("{title} {}", round + 1) };
    (title, i18n::tr_in(language, description).to_string())
}

/// Creates `count` dummy entries. From the 13th entry on the list repeats with a number in the title.
pub fn dummy_library(count: usize) -> Vec<AppEntry> {
    (0..count)
        .map(|i| {
            let (index, round) = (i % DUMMY.len(), i / DUMMY.len());
            let (_, _, kind, accent, accent2) = DUMMY[index];
            let (title, description) = dummy_texts(index, round, i18n::language());
            AppEntry {
                id: format!("dummy:{i}"),
                title,
                description,
                names: Names::Dummy { index, round },
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

/// Takes the own history when it is newer than what the source says (e.g. Steam).
pub fn apply_history(entries: &mut [AppEntry], history: &History) {
    for entry in entries {
        entry.last_used = entry.last_used.max(history.get(&entry.id));
    }
}

/// Most recently used first, then alphabetical.
pub fn sort_by_recent(entries: &mut [AppEntry]) {
    entries.sort_by(compare_recent);
}

/// The order of [`sort_by_recent`], for sorting something that holds entries.
pub fn compare_recent(a: &AppEntry, b: &AppEntry) -> Ordering {
    b.last_used.cmp(&a.last_used).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryFilter {
    #[default]
    All,
    Games,
    Apps,
    /// Only the hidden entries, to show them again
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
            Self::All => tr("All"),
            Self::Games => tr("Games"),
            Self::Apps => tr("Apps"),
            Self::Hidden => tr("Hidden"),
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
            Self::Recent => tr("Recently used"),
            Self::Alphabetical => "A–Z",
        }
    }
}

/// Indices into `entries` for the library. `entries` is already sorted by last use. Hidden entries
/// only appear in the "Hidden" filter; pinned ones keep the normal order.
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

/// Line below the title, e.g. "Last played: yesterday".
pub fn last_used_label(kind: AppKind, last: NaiveDate, today: NaiveDate) -> String {
    let game = kind == AppKind::Game;
    match (today - last).num_days() {
        ..=0 if game => tr("Last played: today").into(),
        ..=0 => tr("Last opened: today").into(),
        1 if game => tr("Last played: yesterday").into(),
        1 => tr("Last opened: yesterday").into(),
        days @ 2..=6 if game => trn("Last played: {n} day ago", "Last played: {n} days ago", days as usize, &[]),
        days @ 2..=6 => trn("Last opened: {n} day ago", "Last opened: {n} days ago", days as usize, &[]),
        _ => {
            // The date format is translated too (German: 01.10.2026).
            let date = last.format(tr("%b %-d, %Y")).to_string();
            if game { trf("Last played: {}", &[&date]) } else { trf("Last opened: {}", &[&date]) }
        }
    }
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
    fn dummy_entries_follow_the_language() {
        let mut apps = dummy_library(DUMMY.len() + 8);
        let english: Vec<_> = apps.iter().map(|a| (a.title.clone(), a.description.clone())).collect();
        assert!(localize_entries(&mut apps, Language::German));
        assert_eq!((apps[0].title.as_str(), apps[0].description.as_str()), ("Nebula Drift", "Weltraum-Rennspiel"));
        assert_eq!(apps[7].title, "Mediathek");
        assert_eq!(apps[DUMMY.len() + 7].title, "Mediathek 2");
        assert!(localize_entries(&mut apps, Language::English));
        let back: Vec<_> = apps.iter().map(|a| (a.title.clone(), a.description.clone())).collect();
        assert_eq!(back, english);
        assert!(!localize_entries(&mut apps, Language::English));
    }

    #[test]
    fn desktop_entries_follow_the_language() {
        let content = "[Desktop Entry]\nType=Application\nName=Files\nName[de]=Dateien\nComment=Browse\n";
        let names = crate::desktop::parse("files.desktop", content).unwrap().names;
        let mut entry = dummy_library(1).remove(0);
        (entry.title, entry.description) = names.resolve(&["en".into()]);
        entry.names = Names::Desktop(Arc::new(names));
        let id = entry.id.clone();
        assert!(entry.localize(Language::German, &["de".into()]));
        assert_eq!((entry.title.as_str(), entry.description.as_str()), ("Dateien", "Browse"));
        assert!(entry.localize(Language::English, &["en".into()]));
        assert_eq!(entry.title, "Files");
        assert_eq!(entry.id, id);
    }

    #[test]
    fn fixed_names_stay() {
        let mut entry = dummy_library(1).remove(0);
        entry.names = Names::Fixed;
        entry.title = "Counter-Strike 2".into();
        assert!(!entry.localize(Language::German, &["de".into()]));
        assert_eq!(entry.title, "Counter-Strike 2");
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
        assert_eq!(last_used_label(AppKind::Game, day(25), today), "Last played: today");
        assert_eq!(last_used_label(AppKind::App, day(24), today), "Last opened: yesterday");
        assert_eq!(last_used_label(AppKind::Game, day(20), today), "Last played: 5 days ago");
        assert_eq!(last_used_label(AppKind::Game, day(1), today), "Last played: Oct 1, 2026");
    }
}
