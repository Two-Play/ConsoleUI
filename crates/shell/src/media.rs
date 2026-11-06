//! Media players over MPRIS on a thread of its own: the quick menu's card shows what plays and
//! controls it. The bus is only asked while the menu is open.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use consoleui_core::media::{MediaCommand, Player, pick};
use zbus::blocking::{Connection, Proxy, fdo};
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
    std::thread::spawn(move || {
        if let Err(err) = run(&messages_rx, &players) {
            eprintln!("No media control: {err}");
        }
    });
    Media { messages, player, watching: false }
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
            let now = current(&connection, shown.as_ref().map(|player| player.bus_name.as_str()))?;
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
    let method = match command {
        MediaCommand::PlayPause => "PlayPause",
        MediaCommand::Next => "Next",
        MediaCommand::Previous => "Previous",
    };
    proxy(connection, name, PLAYER)?.call_method(method, &())?;
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
    Ok(Some(Player {
        bus_name: name.to_string(),
        identity,
        title: text("xesam:title").unwrap_or_default(),
        artist: artists.unwrap_or_default().join(", "),
        playing: status == "Playing",
        can_next: player.get_property("CanGoNext").unwrap_or(false),
        can_previous: player.get_property("CanGoPrevious").unwrap_or(false),
    }))
}
