//! Profiles (Story 5.7): several players share one Linux user, like the users of a PS5. The list
//! is stored in `$XDG_DATA_HOME/consoleui/profiles.json`; every profile keeps its own history,
//! pinned/hidden entries, game options and settings in `$XDG_DATA_HOME/consoleui/profiles/<id>/`.
//! The first profile takes over the files from before profiles existed (copied, not moved, so an
//! older ConsoleUI still finds them).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::search::normalize;
use crate::xdg;

/// More would not fit on one row of the profile screen.
pub const MAX_PROFILES: usize = 8;
/// Longest profile name in characters
pub const MAX_NAME: usize = 20;

/// Avatar colors; a new profile gets the first one not in use yet.
pub const COLORS: [[u8; 3]; 8] = [
    [0x3d, 0x8b, 0xff],
    [0xff, 0x6b, 0x5a],
    [0x3c, 0xd0, 0x8f],
    [0xff, 0xb8, 0x3d],
    [0xb0, 0x7c, 0xff],
    [0xff, 0x6f, 0xb5],
    [0x2f, 0xc6, 0xd8],
    [0x9a, 0xa4, 0xb5],
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Directory name below `profiles/`; never changes, also not when the profile is renamed
    pub id: String,
    pub name: String,
    /// Index into [`COLORS`]
    #[serde(default)]
    pub color: usize,
}

impl Profile {
    /// First letter of the name, upper case, for the avatar
    pub fn initial(&self) -> String {
        self.name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default()
    }

    pub fn rgb(&self) -> [u8; 3] {
        COLORS[self.color % COLORS.len()]
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profiles {
    profiles: Vec<Profile>,
    /// ID of the profile chosen last; the shell starts with it
    last: Option<String>,
}

/// Where a profile keeps its files
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfilePaths {
    pub dir: PathBuf,
    pub history: PathBuf,
    pub collection: PathBuf,
    pub game_options: PathBuf,
    pub settings: PathBuf,
}

impl ProfilePaths {
    pub fn new(root: &Path, id: &str) -> Self {
        let dir = root.join("profiles").join(id);
        Self {
            history: dir.join("history.json"),
            collection: dir.join("collection.json"),
            game_options: dir.join("game_options.json"),
            settings: dir.join("settings.json"),
            dir,
        }
    }
}

impl Profiles {
    /// `$XDG_DATA_HOME/consoleui`: `profiles.json` and the `profiles/` directory live here.
    pub fn default_root() -> Option<PathBuf> {
        xdg::data_home().map(|dir| dir.join("consoleui"))
    }

    fn file(root: &Path) -> PathBuf {
        root.join("profiles.json")
    }

    /// A missing or broken file gives no profiles (see [`Profiles::ensure_one`]).
    pub fn load(root: &Path) -> Self {
        let path = Self::file(root);
        let mut profiles: Self = match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|err| {
                eprintln!("Profiles {} are invalid and ignored: {err}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        };
        // Hand-edited files: no empty or duplicate IDs, no path tricks.
        let mut seen = Vec::new();
        profiles.profiles.retain(|p| {
            let ok = !p.id.is_empty() && p.id == slug(&p.id) && !seen.contains(&p.id);
            seen.push(p.id.clone());
            ok
        });
        profiles.profiles.truncate(MAX_PROFILES);
        profiles
    }

    /// Writes to a temporary file first, so a crash does not destroy the list.
    pub fn save(&self, root: &Path) -> io::Result<()> {
        fs::create_dir_all(root)?;
        let path = Self::file(root);
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        fs::rename(tmp, path)
    }

    /// Creates the first profile if there is none. It takes over the history, collection and game
    /// options from `root` and the settings from `legacy_settings` (where they were before
    /// profiles). Returns whether a profile was created.
    pub fn ensure_one(&mut self, root: &Path, name: &str, legacy_settings: Option<&Path>) -> bool {
        if !self.profiles.is_empty() {
            return false;
        }
        let Some(id) = self.add(name) else { return false };
        let paths = ProfilePaths::new(root, &id);
        let legacy = [
            (root.join("history.json"), &paths.history),
            (root.join("collection.json"), &paths.collection),
            (root.join("game_options.json"), &paths.game_options),
        ];
        let settings = legacy_settings.map(|from| (from.to_path_buf(), &paths.settings));
        for (from, to) in legacy.into_iter().chain(settings) {
            if from.is_file() && !to.exists() {
                let copied = fs::create_dir_all(&paths.dir).and_then(|()| fs::copy(&from, to));
                if let Err(err) = copied {
                    eprintln!("Cannot copy {} into the profile: {err}", from.display());
                }
            }
        }
        true
    }

    pub fn list(&self) -> &[Profile] {
        &self.profiles
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.profiles.len() >= MAX_PROFILES
    }

    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// A profile by ID or (ignoring case and umlauts) by name, for `--profile`.
    pub fn find(&self, id_or_name: &str) -> Option<&Profile> {
        self.get(id_or_name).or_else(|| self.profiles.iter().find(|p| normalize(&p.name) == normalize(id_or_name)))
    }

    /// The profile chosen last, otherwise the first one.
    pub fn last(&self) -> Option<&Profile> {
        self.last.as_deref().and_then(|id| self.get(id)).or_else(|| self.profiles.first())
    }

    pub fn set_last(&mut self, id: &str) {
        self.last = Some(id.to_string());
    }

    /// Adds a profile and returns its ID; `None` if the name is empty or the list is full.
    pub fn add(&mut self, name: &str) -> Option<String> {
        let name = clean_name(name)?;
        if self.is_full() {
            return None;
        }
        let base = slug(&name);
        let base = if base.is_empty() { "profile".to_string() } else { base };
        let id = (1..)
            .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
            .find(|id| self.get(id).is_none())
            .unwrap_or(base);
        let color = (0..COLORS.len()).find(|c| !self.profiles.iter().any(|p| p.color == *c)).unwrap_or(0);
        self.profiles.push(Profile { id: id.clone(), name, color });
        Some(id)
    }

    /// Returns false for an unknown ID or an empty name.
    pub fn rename(&mut self, id: &str, name: &str) -> bool {
        match (clean_name(name), self.profiles.iter_mut().find(|p| p.id == id)) {
            (Some(name), Some(profile)) => {
                profile.name = name;
                true
            }
            _ => false,
        }
    }

    /// Switches the avatar to the next color.
    pub fn next_color(&mut self, id: &str) {
        if let Some(profile) = self.profiles.iter_mut().find(|p| p.id == id) {
            profile.color = (profile.color + 1) % COLORS.len();
        }
    }

    /// Removes a profile from the list (its files stay; see [`remove_files`]). The last remaining
    /// profile cannot be removed.
    pub fn remove(&mut self, id: &str) -> bool {
        if self.profiles.len() <= 1 || self.get(id).is_none() {
            return false;
        }
        self.profiles.retain(|p| p.id != id);
        if self.last.as_deref() == Some(id) {
            self.last = None;
        }
        true
    }
}

/// Deletes the files of a removed profile.
pub fn remove_files(root: &Path, id: &str) -> io::Result<()> {
    let dir = ProfilePaths::new(root, id).dir;
    if dir.exists() { fs::remove_dir_all(dir) } else { Ok(()) }
}

/// The name for the first profile: the login name, capitalized ("philippe" → "Philippe").
pub fn default_name() -> String {
    let user = std::env::var("USER").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| "Player".into());
    let mut chars = user.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or(user)
}

/// Trimmed, inner whitespace collapsed, at most [`MAX_NAME`] characters; `None` if empty.
fn clean_name(name: &str) -> Option<String> {
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_NAME).collect();
    let name = name.trim_end().to_string();
    (!name.is_empty()).then_some(name)
}

/// Directory-safe ID from a name: lower case ASCII letters, digits and single dashes.
fn slug(name: &str) -> String {
    let mut id = String::new();
    for c in normalize(name).chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c);
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
    }
    id.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("consoleui-profiles-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn ids_are_directory_safe_and_unique() {
        let mut profiles = Profiles::default();
        assert_eq!(profiles.add("Jürgen Müller").as_deref(), Some("jurgen-muller"));
        assert_eq!(profiles.add("  jürgen   müller ").as_deref(), Some("jurgen-muller-2"));
        assert_eq!(profiles.add("../..").as_deref(), Some("profile"));
        assert_eq!(profiles.add("   "), None);
        assert_eq!(profiles.get("jurgen-muller-2").unwrap().name, "jürgen müller");
        // Colors: the first unused one
        let colors: Vec<usize> = profiles.list().iter().map(|p| p.color).collect();
        assert_eq!(colors, [0, 1, 2]);
    }

    #[test]
    fn names_are_cleaned_and_limited() {
        assert_eq!(clean_name("  Anna \t Lena "), Some("Anna Lena".into()));
        assert_eq!(clean_name(&"x".repeat(30)).map(|n| n.chars().count()), Some(MAX_NAME));
        assert_eq!(clean_name(""), None);
    }

    #[test]
    fn list_is_limited() {
        let mut profiles = Profiles::default();
        for n in 0..MAX_PROFILES {
            assert!(profiles.add(&format!("P{n}")).is_some());
        }
        assert!(profiles.is_full());
        assert_eq!(profiles.add("One more"), None);
    }

    #[test]
    fn rename_color_and_remove() {
        let mut profiles = Profiles::default();
        let anna = profiles.add("Anna").unwrap();
        let ben = profiles.add("Ben").unwrap();
        assert!(profiles.rename(&anna, "Annabel"));
        assert!(!profiles.rename(&anna, " "));
        assert!(!profiles.rename("nobody", "X"));
        // The ID stays when renaming
        assert_eq!(profiles.get(&anna).unwrap().name, "Annabel");
        assert_eq!(profiles.get(&anna).unwrap().initial(), "A");
        profiles.next_color(&anna);
        assert_eq!(profiles.get(&anna).unwrap().color, 1);
        for _ in 0..COLORS.len() {
            profiles.next_color(&anna);
        }
        assert_eq!(profiles.get(&anna).unwrap().color, 1);

        profiles.set_last(&ben);
        assert_eq!(profiles.last().unwrap().id, ben);
        assert!(profiles.remove(&ben));
        assert_eq!(profiles.last().unwrap().id, anna);
        assert!(!profiles.remove(&anna), "the last profile stays");
        assert!(!profiles.remove("nobody"));
    }

    #[test]
    fn find_by_id_or_name() {
        let mut profiles = Profiles::default();
        profiles.add("Zoë").unwrap();
        assert_eq!(profiles.find("zoe").unwrap().name, "Zoë");
        assert_eq!(profiles.find("ZOË").unwrap().name, "Zoë");
        assert!(profiles.find("Max").is_none());
    }

    #[test]
    fn save_load_and_reject_bad_entries() {
        let root = temp("roundtrip");
        let mut profiles = Profiles::default();
        let id = profiles.add("Anna").unwrap();
        profiles.set_last(&id);
        profiles.save(&root).unwrap();
        assert_eq!(Profiles::load(&root), profiles);

        let json = r#"{"profiles":[{"id":"a","name":"A"},{"id":"a","name":"Dup"},{"id":"../x","name":"Bad"},{"id":"","name":"Empty"}]}"#;
        fs::write(root.join("profiles.json"), json).unwrap();
        let loaded = Profiles::load(&root);
        assert_eq!(loaded.list().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["A"]);

        fs::write(root.join("profiles.json"), "{broken").unwrap();
        assert!(Profiles::load(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn first_profile_takes_over_the_old_files() {
        let root = temp("migrate");
        let config = root.join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(root.join("history.json"), "{\"last_used\":{}}").unwrap();
        fs::write(root.join("collection.json"), "{}").unwrap();
        fs::write(config.join("settings.json"), "{}").unwrap();

        let mut profiles = Profiles::default();
        assert!(profiles.ensure_one(&root, "Philippe", Some(&config.join("settings.json"))));
        let paths = ProfilePaths::new(&root, "philippe");
        assert!(paths.history.is_file() && paths.collection.is_file() && paths.settings.is_file());
        assert!(!paths.game_options.exists(), "missing files are skipped");
        assert!(root.join("history.json").is_file(), "copied, not moved");
        assert!(!profiles.ensure_one(&root, "Other", None));
        assert_eq!(profiles.len(), 1);

        remove_files(&root, "philippe").unwrap();
        assert!(!paths.dir.exists());
        remove_files(&root, "philippe").unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
