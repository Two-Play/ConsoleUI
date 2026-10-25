//! Pinned and hidden entries (context menu), stored as JSON.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::library::AppEntry;
use crate::xdg;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Collection {
    /// Entry IDs in pin order (pinned first = at the very front)
    pinned: Vec<String>,
    /// Entry IDs that do not appear in home, library and search
    hidden: Vec<String>,
}

impl Collection {
    /// `$XDG_DATA_HOME/consoleui/collection.json`
    pub fn default_path() -> Option<PathBuf> {
        xdg::data_home().map(|dir| dir.join("consoleui/collection.json"))
    }

    /// A missing or broken file results in an empty collection.
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|err| {
                eprintln!("Collection {} is invalid and ignored: {err}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Writes to a temporary file first, so a crash does not destroy the collection.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        fs::rename(tmp, path)
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        self.pinned.iter().any(|p| p == id)
    }

    pub fn is_hidden(&self, id: &str) -> bool {
        self.hidden.iter().any(|h| h == id)
    }

    /// Appends the entry to the pinned ones (hidden ones become visible again).
    pub fn pin(&mut self, id: &str) {
        self.hidden.retain(|h| h != id);
        if !self.is_pinned(id) {
            self.pinned.push(id.to_string());
        }
    }

    pub fn unpin(&mut self, id: &str) {
        self.pinned.retain(|p| p != id);
    }

    /// Hiding also unpins.
    pub fn hide(&mut self, id: &str) {
        self.unpin(id);
        if !self.is_hidden(id) {
            self.hidden.push(id.to_string());
        }
    }

    pub fn show(&mut self, id: &str) {
        self.hidden.retain(|h| h != id);
    }

    pub fn has_hidden(&self) -> bool {
        !self.hidden.is_empty()
    }
}

/// Indices into `entries` for the home row: first all pinned ones in pin order, then the
/// remaining ones in the order of `entries` (recently used) until `count` entries are
/// reached. Hidden ones are left out.
pub fn home_view(entries: &[AppEntry], collection: &Collection, count: usize) -> Vec<usize> {
    let mut view: Vec<usize> =
        collection.pinned.iter().filter_map(|id| entries.iter().position(|e| &e.id == id)).collect();
    let rest = (0..entries.len())
        .filter(|&i| !collection.is_pinned(&entries[i].id) && !collection.is_hidden(&entries[i].id));
    let free = count.saturating_sub(view.len());
    view.extend(rest.take(free));
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::dummy_library;

    #[test]
    fn pin_hide_and_show() {
        let mut c = Collection::default();
        c.pin("a");
        c.pin("b");
        c.pin("a");
        assert_eq!(c.pinned, ["a", "b"]);
        c.hide("a");
        assert!(c.is_hidden("a") && !c.is_pinned("a"));
        c.hide("a");
        assert_eq!(c.hidden, ["a"]);
        c.show("a");
        assert!(!c.is_hidden("a") && !c.has_hidden());
        c.hide("c");
        c.pin("c");
        assert!(c.is_pinned("c") && !c.is_hidden("c"));
        c.unpin("b");
        assert_eq!(c.pinned, ["c"]);
    }

    #[test]
    fn home_puts_pinned_first_and_skips_hidden() {
        let entries = dummy_library(6);
        let mut c = Collection::default();
        c.pin("dummy:4");
        c.pin("dummy:2");
        c.pin("steam:gibtsnicht");
        c.hide("dummy:0");
        assert_eq!(home_view(&entries, &c, 4), [4, 2, 1, 3]);
        assert_eq!(home_view(&entries, &c, 10), [4, 2, 1, 3, 5]);
        // Pinned ones always appear, even beyond the count.
        assert_eq!(home_view(&entries, &c, 1), [4, 2]);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("consoleui-collection-{}", std::process::id()));
        let path = dir.join("nested/collection.json");
        let mut c = Collection::default();
        c.pin("steam:730");
        c.hide("desktop:firefox.desktop");
        c.save(&path).unwrap();
        assert_eq!(Collection::load(&path), c);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_partial_or_broken_file() {
        let dir = std::env::temp_dir().join(format!("consoleui-collection-broken-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("collection.json");
        assert_eq!(Collection::load(&path), Collection::default());
        fs::write(&path, r#"{"pinned": ["steam:730"]}"#).unwrap();
        assert!(Collection::load(&path).is_pinned("steam:730"));
        fs::write(&path, "{kaputt").unwrap();
        assert_eq!(Collection::load(&path), Collection::default());
        fs::remove_dir_all(dir).unwrap();
    }
}
