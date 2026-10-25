//! Installierte Steam-Spiele aus `libraryfolders.vdf` und `appmanifest_*.acf`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::library::{AppEntry, AppKind, Artwork, Launch, palette_for};
use crate::vdf;
use crate::xdg;

/// Bit in `StateFlags`: vollständig installiert.
const STATE_FULLY_INSTALLED: u32 = 4;

/// Native Installation zuerst, danach Flatpak.
pub fn find_root() -> Option<PathBuf> {
    let home = xdg::home()?;
    [".local/share/Steam", ".steam/steam", ".var/app/com.valvesoftware.Steam/.local/share/Steam"]
        .iter()
        .map(|p| home.join(p))
        .find(|p| p.join("steamapps").is_dir())
}

pub fn discover(root: &Path) -> Vec<AppEntry> {
    let mut games: HashMap<u32, Manifest> = HashMap::new();
    for library in library_paths(root) {
        let Ok(dir) = fs::read_dir(library.join("steamapps")) else { continue };
        for file in dir.flatten() {
            let name = file.file_name();
            let Some(name) = name.to_str() else { continue };
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            let Some(manifest) = fs::read_to_string(file.path()).ok().and_then(|c| parse_manifest(&c)) else {
                continue;
            };
            if !manifest.installed || is_tool(&manifest.name) {
                continue;
            }
            // Dasselbe Spiel kann in mehreren Bibliotheken liegen.
            match games.get(&manifest.app_id) {
                Some(known) if known.last_played >= manifest.last_played => {}
                _ => {
                    games.insert(manifest.app_id, manifest);
                }
            }
        }
    }
    games.into_values().map(|m| to_entry(root, m)).collect()
}

fn to_entry(root: &Path, manifest: Manifest) -> AppEntry {
    let id = format!("steam:{}", manifest.app_id);
    let (accent, accent2) = palette_for(&id);
    AppEntry {
        artwork: artwork(root, manifest.app_id),
        id,
        title: manifest.name,
        description: "Steam".into(),
        kind: AppKind::Game,
        accent,
        accent2,
        launch: Launch::Steam(manifest.app_id),
        last_used: manifest.last_played,
    }
}

/// Prozesse von Steam-Spielen: Steam gibt jedem Spielprozess (auch unter Proton/Wine)
/// `SteamAppId` in die Umgebung. Steam selbst, seine Hilfsprozesse und die Shell zählen nicht.
fn game_processes() -> io::Result<Vec<(u32, u32)>> {
    let own_pid = std::process::id();
    let mut processes = Vec::new();
    for entry in fs::read_dir("/proc")?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == own_pid {
            continue;
        }
        let dir = entry.path();
        let comm = fs::read_to_string(dir.join("comm")).unwrap_or_default();
        if matches!(comm.trim(), "steam" | "steamwebhelper") {
            continue;
        }
        // Nur Prozesse des eigenen Benutzers sind lesbar; alle anderen fallen hier heraus.
        let Ok(environ) = fs::read(dir.join("environ")) else { continue };
        if let Some(app_id) = steam_app_id(&environ) {
            processes.push((pid, app_id));
        }
    }
    Ok(processes)
}

/// App-IDs aller Steam-Spiele, die gerade Prozesse haben. Zuverlässiger als `RunningAppID` in
/// `~/.steam/registry.vdf`, das Steam oft erst Minuten nach dem Spielende zurücksetzt.
pub fn running_games() -> HashSet<u32> {
    game_processes().map(|p| p.into_iter().map(|(_, app_id)| app_id).collect()).unwrap_or_default()
}

/// Beendet ein laufendes Steam-Spiel per SIGTERM an alle seine Prozesse.
pub fn stop_game(app_id: u32) -> io::Result<usize> {
    let mut stopped = 0;
    for (pid, _) in game_processes()?.into_iter().filter(|&(_, id)| id == app_id) {
        if std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status()?.success() {
            stopped += 1;
        }
    }
    Ok(stopped)
}

fn steam_app_id(environ: &[u8]) -> Option<u32> {
    environ
        .split(|&b| b == 0)
        .find_map(|var| var.strip_prefix(b"SteamAppId="))
        .and_then(|v| std::str::from_utf8(v).ok()?.parse().ok())
        .filter(|&id| id != 0)
}

fn library_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = vec![root.to_path_buf()];
    let doc = fs::read_to_string(root.join("steamapps/libraryfolders.vdf"))
        .ok()
        .and_then(|content| vdf::parse(&content).ok());
    if let Some(folders) = doc.as_ref().and_then(|d| d.get("libraryfolders")) {
        for (_, folder) in folders.entries() {
            if let Some(path) = folder.get("path").and_then(vdf::Value::as_str).map(PathBuf::from)
                && !paths.contains(&path)
            {
                paths.push(path);
            }
        }
    }
    paths
}

#[derive(Debug, PartialEq)]
struct Manifest {
    app_id: u32,
    name: String,
    installed: bool,
    last_played: Option<u64>,
}

fn parse_manifest(content: &str) -> Option<Manifest> {
    let doc = vdf::parse(content).ok()?;
    let state = doc.get("AppState")?;
    let field = |key: &str| state.get(key).and_then(vdf::Value::as_str);
    let flags: u32 = field("StateFlags").and_then(|s| s.parse().ok()).unwrap_or(0);
    Some(Manifest {
        app_id: field("appid")?.parse().ok()?,
        name: field("name")?.trim().to_string(),
        installed: flags & STATE_FULLY_INSTALLED != 0,
        last_played: field("LastPlayed").and_then(|s| s.parse().ok()).filter(|&t| t > 0),
    })
}

/// Laufzeitumgebungen, Redistributables und Soundtracks, die Steam wie Spiele installiert.
fn is_tool(name: &str) -> bool {
    ["Proton", "Steam Linux Runtime", "Steamworks Common Redistributables", "SteamVR"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || name.ends_with("Soundtrack")
}

fn artwork(root: &Path, app_id: u32) -> Artwork {
    let cache = root.join("appcache/librarycache");
    let mut files: HashMap<String, PathBuf> = HashMap::new();
    // Neues Format: librarycache/<appid>/[<hash>/]library_600x900.jpg
    collect_files(&cache.join(app_id.to_string()), 2, &mut files);
    if files.is_empty() {
        // Altes, flaches Format: librarycache/<appid>_library_600x900.jpg
        let prefix = format!("{app_id}_");
        for file in fs::read_dir(&cache).into_iter().flatten().flatten() {
            if let Some(name) = file.file_name().to_str().and_then(|n| n.strip_prefix(&prefix)) {
                files.entry(name.to_string()).or_insert_with(|| file.path());
            }
        }
    }
    let pick = |names: &[&str]| names.iter().find_map(|n| files.get(*n).cloned());
    Artwork {
        cover: pick(&["library_600x900.jpg", "library_capsule.jpg"]),
        hero: pick(&["library_hero.jpg"]),
        logo: pick(&["logo.png"]),
        icon: None,
    }
}

fn collect_files(dir: &Path, depth: u32, files: &mut HashMap<String, PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 1 {
                collect_files(&path, depth - 1, files);
            }
        } else if let Some(name) = entry.file_name().to_str() {
            files.entry(name.to_string()).or_insert(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_installed_manifest() {
        let manifest = parse_manifest(
            r#""AppState" { "appid" "2695940" "name" "PANICORE" "StateFlags" "4" "LastPlayed" "1790020703" }"#,
        );
        assert_eq!(
            manifest,
            Some(Manifest {
                app_id: 2695940,
                name: "PANICORE".into(),
                installed: true,
                last_played: Some(1790020703),
            })
        );
    }

    #[test]
    fn never_played_and_updating_games() {
        let manifest =
            parse_manifest(r#""AppState" { "appid" "1" "name" "X" "StateFlags" "1026" "LastPlayed" "0" }"#).unwrap();
        assert!(!manifest.installed);
        assert_eq!(manifest.last_played, None);
    }

    #[test]
    fn detects_and_stops_game_processes() {
        let mut child = std::process::Command::new("sleep").arg("30").env("SteamAppId", "987654").spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !running_games().contains(&987654) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(running_games().contains(&987654));
        assert_eq!(stop_game(987654).unwrap(), 1);
        child.wait().unwrap();
        assert!(!running_games().contains(&987654));
    }

    #[test]
    fn finds_app_id_in_environment() {
        assert_eq!(steam_app_id(b"HOME=/home/x\0SteamAppId=730\0SteamGameId=730\0"), Some(730));
        assert_eq!(steam_app_id(b"SteamAppId=0\0"), None);
        assert_eq!(steam_app_id(b"SteamGameId=730\0"), None);
    }

    #[test]
    fn detects_tools() {
        assert!(is_tool("Proton Experimental"));
        assert!(is_tool("Steam Linux Runtime 3.0 (sniper)"));
        assert!(is_tool("112 Operator Soundtrack"));
        assert!(!is_tool("Portal 2"));
    }
}
