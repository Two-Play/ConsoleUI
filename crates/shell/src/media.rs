//! Media players over MPRIS on a thread of its own: the quick menu's card and the player window
//! show what plays and control it; the bus is only asked while one of them is open. A second
//! thread listens for track changes, for the "now playing" note (Spotify and every other player).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use consoleui_core::media::{MediaCommand, Player, announce, pick};
use consoleui_core::xdg;
use zbus::blocking::{Connection, MessageIterator, Proxy, fdo};
use zbus::message::Type as MessageType;
use zbus::zvariant::OwnedValue;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
/// How often the card follows a player while the menu is open.
const REFRESH: Duration = Duration::from_secs(1);
/// Players take a moment to report the result of a command.
const SETTLE: Duration = Duration::from_millis(150);

enum Message {
    Watch(bool),
    Command(MediaCommand),
}

pub struct Media {
    messages: Sender<Message>,
    /// The player to show, whenever it changes
    pub player: Receiver<Option<Player>>,
    /// A new track started playing
    pub announced: Receiver<Player>,
    watching: bool,
}

impl Media {
    /// Follows the players while `open`; costs nothing if it stays the same.
    pub fn watch(&mut self, open: bool) {
        if self.watching != open {
            self.watching = open;
            let _ = self.messages.send(Message::Watch(open));
        }
    }

    pub fn send(&self, command: MediaCommand) {
        let _ = self.messages.send(Message::Command(command));
    }
}

pub fn start() -> Media {
    let (messages, messages_rx) = mpsc::channel();
    let (players, player) = mpsc::channel();
    let (announcements, announced) = mpsc::channel();
    std::thread::spawn(move || {
        if let Err(err) = run(&messages_rx, &players) {
            eprintln!("No media control: {err}");
        }
    });
    std::thread::spawn(move || {
        if let Err(err) = listen(&announcements) {
            eprintln!("No \"now playing\" notes: {err}");
        }
    });
    Media { messages, player, announced, watching: false }
}

/// Players report a new track with `PropertiesChanged`; a few signals in a row count once.
fn listen(announcements: &Sender<Player>) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let rule = zbus::MatchRule::builder()
        .msg_type(MessageType::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path(PATH)?
        .build();
    let messages = MessageIterator::for_match_rule(rule, &connection, None)?;
    let mut last: Option<Player> = current(&connection, None)?;
    let mut quiet_until = Instant::now();
    for _ in messages {
        if Instant::now() < quiet_until {
            continue;
        }
        // The metadata settles a moment after the first signal
        std::thread::sleep(Duration::from_millis(300));
        quiet_until = Instant::now() + Duration::from_millis(400);
        let Some(mut now) = current(&connection, last.as_ref().map(|p| p.bus_name.as_str()))? else { continue };
        if announce(last.as_ref(), &now) {
            now.art = fetch_art(&now.art_url);
            if announcements.send(now.clone()).is_err() {
                return Ok(());
            }
        }
        if now.playing || last.is_none() {
            last = Some(now);
        }
    }
    Ok(())
}

/// The cover as a local file: a `file://` URL as it is, an `https://` one downloaded once into
/// the cache (`curl`, a few seconds at most).
pub fn fetch_art(url: &str) -> Option<PathBuf> {
    if let Some(path) = url.strip_prefix("file://") {
        return Some(PathBuf::from(path)).filter(|path| path.is_file());
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return None;
    }
    let dir = xdg::cache_home()?.join("consoleui/art");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    let file = dir.join(format!("{:016x}", hasher.finish()));
    if file.is_file() {
        return Some(file);
    }
    std::fs::create_dir_all(&dir).ok()?;
    let partial = file.with_extension("part");
    let fetched = Command::new("curl")
        .args(["-sfL", "--max-time", "5", "-o"])
        .arg(&partial)
        .arg(url)
        .stdin(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if fetched && std::fs::rename(&partial, &file).is_ok() {
        Some(file)
    } else {
        let _ = std::fs::remove_file(&partial);
        None
    }
}

fn run(messages: &Receiver<Message>, players: &Sender<Option<Player>>) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let mut watching = false;
    let mut shown: Option<Player> = None;
    loop {
        let message = if watching {
            messages.recv_timeout(REFRESH)
        } else {
            messages.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match message {
            Ok(Message::Watch(open)) => watching = open,
            Ok(Message::Command(command)) => {
                if let Some(player) = &shown {
                    if let Err(err) = send(&connection, &player.bus_name, command) {
                        eprintln!("Cannot control {}: {err}", player.identity);
                    }
                    std::thread::sleep(SETTLE);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        if watching {
            let mut now = current(&connection, shown.as_ref().map(|player| player.bus_name.as_str()))?;
            if let Some(player) = &mut now {
                // The cover only once per track
                player.art = match &shown {
                    Some(old) if old.art_url == player.art_url => old.art.clone(),
                    _ => fetch_art(&player.art_url),
                };
            }
            if now != shown {
                shown = now.clone();
                if players.send(now).is_err() {
                    return Ok(());
                }
            }
        }
    }
}

fn proxy<'a>(connection: &'a Connection, name: &str, interface: &'static str) -> zbus::Result<Proxy<'a>> {
    zbus::blocking::proxy::Builder::new(connection)
        .destination(name.to_string())?
        .path(PATH)?
        .interface(interface)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

fn send(connection: &Connection, name: &str, command: MediaCommand) -> zbus::Result<()> {
    let player = proxy(connection, name, PLAYER)?;
    match command {
        MediaCommand::PlayPause => player.call_method("PlayPause", &())?,
        MediaCommand::Next => player.call_method("Next", &())?,
        MediaCommand::Previous => player.call_method("Previous", &())?,
        MediaCommand::Seek(micros) => player.call_method("Seek", &(micros,))?,
        MediaCommand::Shuffle(on) => {
            player.set_property("Shuffle", on).map_err(zbus::Error::from)?;
            return Ok(());
        }
    };
    Ok(())
}

fn current(connection: &Connection, shown: Option<&str>) -> zbus::Result<Option<Player>> {
    let names = fdo::DBusProxy::new(connection)?.list_names()?;
    let players = names.iter().map(|name| name.as_str()).filter(|name| name.starts_with(PREFIX));
    // A player that does not answer properly is left out, the others still count
    Ok(pick(players.filter_map(|name| read(connection, name).ok().flatten()).collect(), shown))
}

/// `None` for a player that has nothing loaded or cannot be controlled.
fn read(connection: &Connection, name: &str) -> zbus::Result<Option<Player>> {
    let player = proxy(connection, name, PLAYER)?;
    let status: String = player.get_property("PlaybackStatus")?;
    let can_control: bool = player.get_property("CanControl").unwrap_or(true);
    if status == "Stopped" || !can_control {
        return Ok(None);
    }
    let metadata: HashMap<String, OwnedValue> = player.get_property("Metadata").unwrap_or_default();
    let text = |key: &str| metadata.get(key).and_then(|value| String::try_from(value.clone()).ok());
    let artists = metadata.get("xesam:artist").and_then(|value| Vec::<String>::try_from(value.clone()).ok());
    let identity: String = proxy(connection, name, ROOT)?.get_property("Identity").unwrap_or_default();
    let title = text("xesam:title").unwrap_or_default();
    // Players differ in the type of the track ID (object path or string) and the length
    let track = metadata
        .get("mpris:trackid")
        .and_then(|value| {
            zbus::zvariant::OwnedObjectPath::try_from(value.clone())
                .map(|path| path.to_string())
                .ok()
                .or_else(|| String::try_from(value.clone()).ok())
        })
        .unwrap_or_else(|| title.clone());
    let length = metadata.get("mpris:length").and_then(|value| {
        u64::try_from(value.clone()).ok().or_else(|| i64::try_from(value.clone()).ok().map(|n| n.max(0) as u64))
    });
    let position: i64 = player.get_property("Position").unwrap_or(0);
    Ok(Some(Player {
        bus_name: name.to_string(),
        identity,
        title,
        artist: artists.unwrap_or_default().join(", "),
        playing: status == "Playing",
        can_next: player.get_property("CanGoNext").unwrap_or(false),
        can_previous: player.get_property("CanGoPrevious").unwrap_or(false),
        album: text("xesam:album").unwrap_or_default(),
        track,
        art_url: text("mpris:artUrl").unwrap_or_default(),
        art: None,
        length: length.unwrap_or(0),
        position: position.max(0) as u64,
        can_seek: player.get_property("CanSeek").unwrap_or(false),
        shuffle: player.get_property("Shuffle").ok(),
    }))
}
