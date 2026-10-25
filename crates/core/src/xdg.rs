//! XDG-Pfade und Spracheinstellungen aus der Umgebung.

use std::env;
use std::path::PathBuf;

pub fn home() -> Option<PathBuf> {
    env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn from_env_or_home(var: &str, fallback: &str) -> Option<PathBuf> {
    env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(fallback)))
}

pub fn data_home() -> Option<PathBuf> {
    from_env_or_home("XDG_DATA_HOME", ".local/share")
}

pub fn config_home() -> Option<PathBuf> {
    from_env_or_home("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_DATA_HOME` gefolgt von `$XDG_DATA_DIRS`, nach Priorität sortiert und ohne Duplikate.
pub fn data_dirs() -> Vec<PathBuf> {
    let system = env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let mut dirs: Vec<PathBuf> = data_home().into_iter().collect();
    for dir in system.split(':').filter(|s| !s.is_empty()).map(PathBuf::from) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Sprachcodes für lokalisierte `.desktop`-Schlüssel, z. B. `["de_DE", "de"]`.
pub fn languages() -> Vec<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|var| env::var(var).ok().filter(|v| !v.is_empty()))
        .map(|locale| languages_from(&locale))
        .unwrap_or_default()
}

fn languages_from(locale: &str) -> Vec<String> {
    let base = locale.split(['.', '@']).next().unwrap_or_default();
    if base.is_empty() || base == "C" || base == "POSIX" {
        return Vec::new();
    }
    let mut langs = vec![base.to_string()];
    if let Some((lang, _country)) = base.split_once('_') {
        langs.push(lang.to_string());
    }
    langs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_from_locale() {
        assert_eq!(languages_from("de_DE.UTF-8"), ["de_DE", "de"]);
        assert_eq!(languages_from("de_AT@euro"), ["de_AT", "de"]);
        assert_eq!(languages_from("fr"), ["fr"]);
        assert!(languages_from("C.UTF-8").is_empty());
    }
}
