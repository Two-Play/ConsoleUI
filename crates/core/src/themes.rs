//! Theme music of games on the home screen, like on a PS5 (settings row "Background music",
//! choice "Game themes"). There is no source for it, so like the Steam Deck plugin that does the
//! same the shell searches YouTube for "<title> theme music" with `yt-dlp` and keeps the audio
//! of a result in the cache. Per game the user can take the next result or turn it off.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Longer results are mixes or let's plays, not a theme
pub const MAX_SECONDS: u32 = 600;

/// What to search for: the title without trademark signs, plus "theme music".
pub fn search_query(title: &str) -> String {
    let clean: String = title.chars().filter(|c| !matches!(c, '™' | '®' | '©')).collect();
    format!("{} theme music", clean.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// `<cache>/<id>-<result>.opus`, e.g. `steam_400-0.opus`.
pub fn cache_file(dir: &Path, id: &str, result: u32) -> PathBuf {
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' }).collect();
    dir.join(format!("{safe}-{result}.opus"))
}

/// Marks a search that found nothing, so it is not repeated on every focus.
pub fn failed_marker(file: &Path) -> PathBuf {
    file.with_extension("failed")
}

/// `yt-dlp` arguments: result number `result` (0 is the first) of the search, audio only, as
/// Opus into `file`.
pub fn ytdlp_args(query: &str, result: u32, file: &Path) -> Vec<String> {
    let stem = file.with_extension("");
    vec![
        "--no-playlist".into(),
        "--quiet".into(),
        "--no-warnings".into(),
        "--playlist-items".into(),
        (result + 1).to_string(),
        "--match-filter".into(),
        format!("duration < {MAX_SECONDS}"),
        "--format".into(),
        "bestaudio".into(),
        "--extract-audio".into(),
        "--audio-format".into(),
        "opus".into(),
        "--output".into(),
        format!("{}.%(ext)s", stem.display()),
        // A search with more results than the one taken, so "next" has something to take
        format!("ytsearch{}:{query}", result + 1),
    ]
}

/// The choice per game: which result plays, or none. Saved as `theme_music.json` in the profile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeChoices {
    /// Entry ID → result number; missing means the first result
    #[serde(default)]
    results: BTreeMap<String, u32>,
    /// Entries without theme music
    #[serde(default)]
    off: Vec<String>,
}

impl ThemeChoices {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(io::Error::other)?)
    }

    /// The result to play, `None` when the user turned it off for this game.
    pub fn result(&self, id: &str) -> Option<u32> {
        (!self.is_off(id)).then(|| self.results.get(id).copied().unwrap_or(0))
    }

    pub fn is_off(&self, id: &str) -> bool {
        self.off.iter().any(|off| off == id)
    }

    /// The next search result ("this is not the right music"); turns it on again.
    pub fn next(&mut self, id: &str) {
        self.off.retain(|off| off != id);
        *self.results.entry(id.to_string()).or_insert(0) += 1;
    }

    pub fn set_off(&mut self, id: &str, off: bool) {
        self.off.retain(|other| other != id);
        if off {
            self.off.push(id.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_drops_trademark_signs() {
        assert_eq!(search_query("Need for Speed™ Heat"), "Need for Speed Heat theme music");
        assert_eq!(search_query("Call of Duty®:  Black Ops"), "Call of Duty: Black Ops theme music");
    }

    #[test]
    fn cache_files_are_plain_names() {
        let file = cache_file(Path::new("/c"), "steam:400", 2);
        assert_eq!(file, Path::new("/c/steam_400-2.opus"));
        assert_eq!(failed_marker(&file), Path::new("/c/steam_400-2.failed"));
        let args = ytdlp_args("Portal theme music", 2, &file);
        assert_eq!(args.last().unwrap(), "ytsearch3:Portal theme music");
        assert!(args.windows(2).any(|w| w[0] == "--playlist-items" && w[1] == "3"));
        assert!(args.windows(2).any(|w| w[0] == "--output" && w[1] == "/c/steam_400-2.%(ext)s"));
    }

    #[test]
    fn choices_step_through_results_and_turn_off() {
        let mut choices = ThemeChoices::default();
        assert_eq!(choices.result("steam:400"), Some(0));
        choices.next("steam:400");
        assert_eq!(choices.result("steam:400"), Some(1));
        choices.set_off("steam:400", true);
        assert_eq!(choices.result("steam:400"), None);
        choices.next("steam:400");
        assert_eq!(choices.result("steam:400"), Some(2));
        let text = serde_json::to_string(&choices).unwrap();
        assert_eq!(serde_json::from_str::<ThemeChoices>(&text).unwrap(), choices);
    }
}
