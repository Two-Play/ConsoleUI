//! Wann welche App zuletzt aus der Shell gestartet wurde, gespeichert als JSON.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::xdg;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct History {
    /// App-ID → Unix-Zeit in Sekunden
    last_used: HashMap<String, u64>,
}

impl History {
    /// `$XDG_DATA_HOME/consoleui/history.json`
    pub fn default_path() -> Option<PathBuf> {
        xdg::data_home().map(|dir| dir.join("consoleui/history.json"))
    }

    /// Eine fehlende oder kaputte Datei ergibt eine leere Historie.
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|err| {
                eprintln!("Historie {} ist ungültig und wird ignoriert: {err}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Schreibt zuerst in eine temporäre Datei, damit ein Absturz die Historie nicht zerstört.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        fs::rename(tmp, path)
    }

    pub fn record(&mut self, id: &str, timestamp: u64) {
        self.last_used.insert(id.to_string(), timestamp);
    }

    pub fn get(&self, id: &str) -> Option<u64> {
        self.last_used.get(id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("consoleui-history-{}", std::process::id()));
        let path = dir.join("nested/history.json");
        let mut history = History::default();
        history.record("steam:730", 42);
        history.save(&path).unwrap();
        assert_eq!(History::load(&path).get("steam:730"), Some(42));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_or_broken_file_gives_empty_history() {
        let dir = std::env::temp_dir().join(format!("consoleui-history-broken-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.json");
        assert_eq!(History::load(&path).get("x"), None);
        fs::write(&path, "{kaputt").unwrap();
        assert_eq!(History::load(&path).get("x"), None);
        fs::remove_dir_all(dir).unwrap();
    }
}
