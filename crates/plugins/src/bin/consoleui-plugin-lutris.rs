//! Lutris games in the ConsoleUI library (Epic 9). Asks `lutris --list-games --json --installed`
//! and watches Lutris' database for changes. `--fake`: two made-up games, Lutris is not asked.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use chrono::{Local, NaiveDateTime, TimeZone};
use consoleui_plugins::{Dict, PATH, dict, fake};
use zbus::blocking::connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::Value;

const NAME: &str = "org.consoleui.plugins.Lutris";
/// How often the database is looked at
const WATCH: Duration = Duration::from_secs(20);

struct Game {
    id: i64,
    name: String,
    cover: Option<String>,
    last_played: Option<i64>,
}

/// Lutris itself, or its Flatpak.
fn lutris() -> Vec<String> {
    let installed = std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join("lutris").is_file()));
    if installed { vec!["lutris".into()] } else { ["flatpak", "run", "net.lutris.Lutris"].map(String::from).to_vec() }
}

fn database() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    [data.join("lutris/pga.db"), home.join(".var/app/net.lutris.Lutris/data/lutris/pga.db")]
        .into_iter()
        .find(|path| path.is_file())
}

fn parse(json: &str) -> Vec<Game> {
    let Ok(serde_json::Value::Array(games)) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    games
        .iter()
        .filter_map(|game| {
            let text = |key: &str| game.get(key).and_then(serde_json::Value::as_str).map(str::to_string);
            let last_played = text("lastplayed")
                .and_then(|t| NaiveDateTime::parse_from_str(&t, "%Y-%m-%d %H:%M:%S").ok())
                .and_then(|t| Local.from_local_datetime(&t).single())
                .map(|t| t.timestamp());
            Some(Game {
                id: game.get("id")?.as_i64()?,
                name: text("name")?,
                cover: text("coverPath").filter(|path| std::path::Path::new(path).is_file()),
                last_played,
            })
        })
        .collect()
}

fn read_games() -> Vec<Game> {
    if fake() {
        return vec![
            Game { id: 1, name: "Fake Racer".into(), cover: None, last_played: Some(1_700_000_000) },
            Game { id: 2, name: "Fake Quest".into(), cover: None, last_played: None },
        ];
    }
    let argv = lutris();
    let output = Command::new(&argv[0])
        .args(&argv[1..])
        .args(["--list-games", "--json", "--installed"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(output) => parse(&String::from_utf8_lossy(&output.stdout)),
        Err(err) => {
            eprintln!("Cannot ask Lutris: {err}");
            Vec::new()
        }
    }
}

struct Plugin {
    games: Arc<Mutex<Vec<Game>>>,
}

#[zbus::interface(name = "org.consoleui.Plugin1")]
impl Plugin {
    #[zbus(property)]
    fn version(&self) -> u32 {
        1
    }

    fn entries(&self) -> Vec<Dict> {
        let games = self.games.lock().unwrap();
        let lutris = lutris();
        games
            .iter()
            .map(|game| {
                let mut command = lutris.clone();
                command.push(format!("lutris:rungameid/{}", game.id));
                let mut entry = dict([
                    ("id", Value::from(game.id.to_string())),
                    ("title", Value::from(game.name.clone())),
                    ("kind", Value::from("game")),
                    ("command", Value::from(command)),
                ]);
                if let Some(cover) = &game.cover {
                    entry.insert("cover".into(), consoleui_plugins::owned(cover.clone()));
                }
                if let Some(time) = game.last_played {
                    entry.insert("last_played".into(), consoleui_plugins::owned(time));
                }
                entry
            })
            .collect()
    }

    /// Answers first, then exits
    fn quit(&self) {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            std::process::exit(0);
        });
    }

    #[zbus(signal)]
    async fn entries_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

fn changed_at(path: &Option<PathBuf>) -> Option<SystemTime> {
    path.as_ref().and_then(|path| std::fs::metadata(path).ok()).and_then(|meta| meta.modified().ok())
}

fn main() -> zbus::Result<()> {
    let games = Arc::new(Mutex::new(read_games()));
    let connection =
        connection::Builder::session()?.name(NAME)?.serve_at(PATH, Plugin { games: games.clone() })?.build()?;
    let database = database();
    let mut seen = changed_at(&database);
    consoleui_plugins::serve(&connection, WATCH, || {
        let now = changed_at(&database);
        if now != seen {
            seen = now;
            *games.lock().unwrap() = read_games();
            if let Ok(interface) = connection.object_server().interface::<_, Plugin>(PATH) {
                let _ = zbus::block_on(Plugin::entries_changed(interface.signal_emitter()));
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_json_of_lutris() {
        let games = parse(
            r#"[{"id": 1, "slug": "ea-app", "name": "EA App", "lastplayed": "2026-11-06 01:11:42",
                 "coverPath": "/nonexistent/ea-app.jpg"},
                {"id": 2, "name": "No Date"}, {"name": "No ID"}]"#,
        );
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].name, "EA App");
        assert!(games[0].last_played.is_some());
        assert_eq!(games[0].cover, None, "a cover that is not there is left out");
        assert_eq!(games[1].last_played, None);
        assert!(parse("not json").is_empty());
    }
}
