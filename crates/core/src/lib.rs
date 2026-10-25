//! Plattformunabhängige Logik der ConsoleUI-Shell, ohne UI-Abhängigkeiten.

pub mod accent;
pub mod desktop;
mod history;
mod icons;
pub mod launch;
mod library;
mod nav;
pub mod steam;
pub mod vdf;
pub mod xdg;

pub use history::History;
pub use library::{
    AppEntry, AppKind, Artwork, Launch, apply_history, dummy_library, last_used_label, palette_for, sort_by_recent,
};
pub use nav::{Action, Carousel};

/// Alle installierten Steam-Spiele und konsolentauglichen Desktop-Apps (unsortiert).
pub fn discover_library() -> Vec<AppEntry> {
    let mut entries = steam::find_root().map(|root| steam::discover(&root)).unwrap_or_default();
    entries.extend(desktop::discover(&xdg::data_dirs(), &xdg::languages()));
    entries
}
