//! Plattformunabhängige Logik der ConsoleUI-Shell, ohne UI-Abhängigkeiten.

mod library;
mod nav;

pub use library::{AppEntry, AppKind, dummy_library};
pub use nav::{Action, Carousel};
