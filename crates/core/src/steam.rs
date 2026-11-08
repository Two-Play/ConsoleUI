//! Installed Steam games from `libraryfolders.vdf` and `appmanifest_*.acf`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::library::{AppEntry, AppKind, Artwork, Launch, Names, palette_for};
use crate::vdf;
use crate::xdg;

/// Bit in `StateFlags`: fully installed.
const STATE_FULLY_INSTALLED: u32 = 4;

/// Native installation first, then Flatpak.
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
            // The same game can be in several libraries.
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
        names: Names::Fixed,
        kind: AppKind::Game,
        accent,
        accent2,
        launch: Launch::Steam(manifest.app_id),
        last_used: manifest.last_played,
    }
}

/// Steam games of the account that are not installed: the ones the user played before (the
/// client's `localconfig.vdf` lists them with their last start), named by `appinfo.vdf`. Games
/// bought and never started are not in these files; that needs the Web API.
pub fn discover_not_installed(root: &Path, installed: &HashSet<u32>) -> Vec<AppEntry> {
    let infos = crate::steam_appinfo::read(&root.join("appcache/appinfo.vdf"));
    if infos.is_empty() {
        return Vec::new();
    }
    let mut played: HashMap<u32, Option<u64>> = HashMap::new();
    for dir in user_dirs(root) {
        let Ok(content) = fs::read_to_string(dir.join("config/localconfig.vdf")) else { continue };
        for (app_id, last) in played_apps(&content) {
            let known = played.entry(app_id).or_insert(last);
            *known = (*known).max(last);
        }
    }
    let mut entries: Vec<AppEntry> = played
        .into_iter()
        .filter(|(app_id, _)| !installed.contains(app_id))
        .filter_map(|(app_id, last)| {
            let info = infos.get(&app_id).filter(|info| info.kind == "game")?;
            let id = format!("steam:{app_id}");
            let (accent, accent2) = palette_for(&id);
            Some(AppEntry {
                artwork: artwork(root, app_id),
                id,
                title: info.name.trim().to_string(),
                description: "Steam".into(),
                names: Names::Fixed,
                kind: AppKind::Game,
                accent,
                accent2,
                launch: Launch::SteamInstall(app_id),
                last_used: last,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries
}

/// The account that logged in last (`config/loginusers.vdf`), else every account on the machine.
fn user_dirs(root: &Path) -> Vec<PathBuf> {
    let users = root.join("userdata");
    let recent = fs::read_to_string(root.join("config/loginusers.vdf")).ok().and_then(|content| {
        let doc = vdf::parse(&content).ok()?;
        let (id, _) = doc
            .get("users")?
            .entries()
            .iter()
            .find(|(_, user)| user.get("MostRecent").and_then(vdf::Value::as_str) == Some("1"))?;
        // SteamID64 → the account ID that names the folder
        let account = id.parse::<u64>().ok()?.checked_sub(76_561_197_960_265_728)?;
        Some(users.join(account.to_string()))
    });
    match recent.filter(|dir| dir.is_dir()) {
        Some(dir) => vec![dir],
        None => fs::read_dir(&users).into_iter().flatten().flatten().map(|entry| entry.path()).collect(),
    }
}

/// App IDs in `UserLocalConfigStore/Software/Valve/Steam/apps`, with the last start.
fn played_apps(content: &str) -> Vec<(u32, Option<u64>)> {
    let Ok(doc) = vdf::parse(content) else { return Vec::new() };
    let apps = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"]
        .iter()
        .try_fold(&doc, |value, key| value.get(key));
    let Some(apps) = apps else { return Vec::new() };
    apps.entries()
        .iter()
        .filter_map(|(id, app)| {
            let last = app.get("LastPlayed").and_then(vdf::Value::as_str).and_then(|t| t.parse().ok());
            Some((id.parse().ok()?, last.filter(|&t: &u64| t > 0)))
        })
        .collect()
}

/// Processes of Steam games: Steam puts `SteamAppId` into the environment of every game process
/// (also under Proton/Wine). Steam itself, its helper processes and the shell do not count.
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
        // Steam's shader processing is not the game yet
        if matches!(comm.trim(), "steam" | "steamwebhelper") || comm.starts_with("fossilize") {
            continue;
        }
        // Only processes of the own user are readable; all others drop out here.
        let Ok(environ) = fs::read(dir.join("environ")) else { continue };
        if let Some(app_id) = steam_app_id(&environ) {
            processes.push((pid, app_id));
        }
    }
    Ok(processes)
}

/// App IDs of all Steam games that currently have processes. More reliable than `RunningAppID` in
/// `~/.steam/registry.vdf`, which Steam often resets only minutes after the game ended.
/// Games whose shaders Steam is processing right now: its `fossilize_replay` works on
/// `steamapps/shadercache/<appid>/...`. Steam reports no progress, only that it happens.
pub fn shader_processing() -> HashSet<u32> {
    let Ok(dir) = fs::read_dir("/proc") else { return HashSet::new() };
    dir.flatten()
        .filter(|entry| entry.file_name().to_str().is_some_and(|name| name.bytes().all(|b| b.is_ascii_digit())))
        .filter(|entry| fs::read_to_string(entry.path().join("comm")).is_ok_and(|comm| comm.starts_with("fossilize")))
        .filter_map(|entry| fs::read(entry.path().join("cmdline")).ok())
        .filter_map(|cmdline| shader_app_id(&cmdline))
        .collect()
}

fn shader_app_id(cmdline: &[u8]) -> Option<u32> {
    let mut args = cmdline.split(|&b| b == 0).map(String::from_utf8_lossy);
    if !args.next()?.ends_with("fossilize_replay") {
        return None;
    }
    args.find_map(|arg| {
        let (_, rest) = arg.split_once("/shadercache/")?;
        rest.split('/').next()?.parse().ok()
    })
}

pub fn running_games() -> HashSet<u32> {
    game_processes().map(|p| p.into_iter().map(|(_, app_id)| app_id).collect()).unwrap_or_default()
}

/// Stops a running Steam game by sending SIGTERM to all its processes.
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

/// Download state of every Steam game in the libraries: name and whether it is complete and idle
/// (`StateFlags` exactly "fully installed", not updating).
pub fn download_states(root: &Path) -> HashMap<u32, (String, bool)> {
    let mut states = HashMap::new();
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
            if !is_tool(&manifest.name) {
                states.insert(manifest.app_id, (manifest.name, manifest.flags == STATE_FULLY_INSTALLED));
            }
        }
    }
    states
}

/// What changed between two [`download_states`]: names that started downloading (new and not
/// complete) and names that are complete now (new, or not complete before).
pub fn download_changes(
    before: &HashMap<u32, (String, bool)>,
    after: &HashMap<u32, (String, bool)>,
) -> (Vec<String>, Vec<String>) {
    let (mut started, mut finished) = (Vec::new(), Vec::new());
    for (id, (name, ready)) in after {
        match before.get(id) {
            None if *ready => finished.push(name.clone()),
            None => started.push(name.clone()),
            Some((_, false)) if *ready => finished.push(name.clone()),
            _ => {}
        }
    }
    started.sort();
    finished.sort();
    (started, finished)
}

#[derive(Debug, PartialEq)]
struct Manifest {
    app_id: u32,
    name: String,
    flags: u32,
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
        flags,
        installed: flags & STATE_FULLY_INSTALLED != 0,
        last_played: field("LastPlayed").and_then(|s| s.parse().ok()).filter(|&t| t > 0),
    })
}

/// Runtimes, redistributables and soundtracks that Steam installs like games.
fn is_tool(name: &str) -> bool {
    ["Proton", "Steam Linux Runtime", "Steamworks Common Redistributables", "SteamVR"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || name.ends_with("Soundtrack")
}

fn artwork(root: &Path, app_id: u32) -> Artwork {
    let cache = root.join("appcache/librarycache");
    let mut files: HashMap<String, PathBuf> = HashMap::new();
    // New format: librarycache/<appid>/[<hash>/]library_600x900.jpg
    collect_files(&cache.join(app_id.to_string()), 2, &mut files);
    if files.is_empty() {
        // Old, flat format: librarycache/<appid>_library_600x900.jpg
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
                flags: 4,
                installed: true,
                last_played: Some(1790020703)
            })
        );
    }

    #[test]
    fn download_changes_find_new_and_finished_games() {
        let state = |list: &[(u32, &str, bool)]| -> HashMap<u32, (String, bool)> {
            list.iter().map(|&(id, name, ready)| (id, (name.to_string(), ready))).collect()
        };
        let before = state(&[(1, "Old", true), (2, "Updating", false), (3, "Same", false)]);
        let after = state(&[
            (1, "Old", true),
            (2, "Updating", true),
            (3, "Same", false),
            (4, "Fresh", false),
            (5, "Quick", true),
        ]);
        assert_eq!(
            download_changes(&before, &after),
            (vec!["Fresh".to_string()], vec!["Quick".into(), "Updating".into()])
        );
        // A game that starts updating is not announced
        let busy = state(&[(1, "Old", false)]);
        assert_eq!(download_changes(&state(&[(1, "Old", true)]), &busy), (vec![], vec![]));
    }

    #[test]
    fn shader_processing_names_the_game() {
        let cmdline = b"/home/u/.local/share/Steam/ubuntu12_64/fossilize_replay\0--master-process\0/home/u/.local/share/Steam/steamapps/shadercache/1091500/fozpipelinesv6/steamapprun_pipeline_cache.foz\0";
        assert_eq!(shader_app_id(cmdline), Some(1091500));
        assert_eq!(shader_app_id(b"/usr/bin/cat\0/x/shadercache/1/y\0"), None);
        assert_eq!(shader_app_id(b"fossilize_replay\0--help\0"), None);
    }

    #[test]
    fn played_apps_come_from_the_local_config() {
        let content = r#""UserLocalConfigStore" { "Software" { "Valve" { "Steam" { "apps" {
            "400" { "LastPlayed" "1700000000" "Playtime" "12" }
            "7" { "cloud" { "last_sync_state" "synchronized" } }
        } } } } }"#;
        let mut apps = played_apps(content);
        apps.sort();
        assert_eq!(apps, [(7, None), (400, Some(1_700_000_000))]);
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
