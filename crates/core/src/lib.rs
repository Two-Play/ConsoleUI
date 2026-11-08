//! Platform-independent logic of the ConsoleUI shell, without UI dependencies.

pub mod accent;
pub mod backdrop;
pub mod bluetooth;
mod collection;
pub mod desktop;
pub mod dialog;
pub mod fakenet;
pub mod game_options;
mod grid;
mod history;
pub mod i18n;
mod icons;
pub mod image_format;
pub mod input;
pub mod keymap;
pub mod launch;
mod library;
pub mod media;
pub mod music;
mod nav;
pub mod netpages;
pub mod network;
pub mod notification;
pub mod osk;
pub mod ownership;
pub mod placement;
pub mod pointer;
pub mod profile_picker;
pub mod profiles;
pub mod protocol;
pub mod running;
pub mod search;
pub mod service;
pub mod settings;
pub mod sound;
pub mod status;
pub mod steam;
pub mod steam_appinfo;
pub mod switcher;
pub mod system;
pub mod themes;
pub mod updates;
pub mod vdf;
pub mod wifi;
pub mod xdg;

pub use collection::{Collection, home_view};
pub use grid::Grid;
pub use history::History;
pub use library::{
    AppEntry, AppKind, Artwork, Launch, LibraryFilter, LibrarySort, Names, apply_history, compare_recent,
    dummy_library, last_used_label, library_view, localize_entries, palette_for, sort_by_recent,
};
pub use nav::{Action, Carousel, HomeArea, HomeOutcome};

/// All installed Steam games and console-friendly desktop apps (unsorted). Localized desktop
/// names follow the current [`i18n::language`]; [`localize_entries`] switches them later.
pub fn discover_library() -> Vec<AppEntry> {
    let mut entries = Vec::new();
    if let Some(root) = steam::find_root() {
        entries = steam::discover(&root);
        let installed = entries.iter().filter_map(|entry| match entry.launch {
            Launch::Steam(app_id) => Some(app_id),
            _ => None,
        });
        let installed = installed.collect();
        entries.extend(steam::discover_not_installed(&root, &installed));
    }
    entries.extend(desktop::discover(&xdg::data_dirs(), &i18n::desktop_languages(i18n::language(), xdg::languages())));
    entries
}
