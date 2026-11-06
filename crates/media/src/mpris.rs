//! The player on the session bus, so the quick menu's media card (Story 4.8) can pause and skip while
//! the app is behind a game. Read only while the card is shown, so it sends no change signals.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use consoleui_core::media::MediaCommand;
use zbus::blocking::{Connection, connection};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const PATH: &str = "/org/mpris/MediaPlayer2";

/// What the bus shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shown {
    pub loaded: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub can_next: bool,
    pub can_previous: bool,
}

struct Root;

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    fn identity(&self) -> String {
        "ConsoleUI Media".into()
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
}

struct Control {
    shown: Arc<Mutex<Shown>>,
    commands: Sender<MediaCommand>,
}

impl Control {
    fn shown(&self) -> Shown {
        self.shown.lock().map(|shown| shown.clone()).unwrap_or_default()
    }

    fn send(&self, command: MediaCommand) {
        let _ = self.commands.send(command);
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Control {
    fn play_pause(&self) {
        self.send(MediaCommand::PlayPause);
    }

    fn play(&self) {
        if !self.shown().playing {
            self.send(MediaCommand::PlayPause);
        }
    }

    fn pause(&self) {
        if self.shown().playing {
            self.send(MediaCommand::PlayPause);
        }
    }

    fn next(&self) {
        self.send(MediaCommand::Next);
    }

    fn previous(&self) {
        self.send(MediaCommand::Previous);
    }

    #[zbus(property)]
    fn playback_status(&self) -> String {
        let shown = self.shown();
        match (shown.loaded, shown.playing) {
            (false, _) => "Stopped",
            (true, true) => "Playing",
            (true, false) => "Paused",
        }
        .into()
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.shown().loaded
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.shown().loaded
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.shown().can_next
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.shown().can_previous
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let shown = self.shown();
        let owned = |value: Value| value.try_to_owned().expect("no file descriptors in here");
        let id = ObjectPath::from_static_str_unchecked("/org/consoleui/Media/Current");
        HashMap::from([
            ("mpris:trackid".to_string(), owned(Value::from(id))),
            ("xesam:title".to_string(), owned(Value::from(shown.title))),
            ("xesam:artist".to_string(), owned(Value::from(vec![shown.artist]))),
            ("xesam:album".to_string(), owned(Value::from(shown.album))),
        ])
    }
}

/// On the bus for as long as it lives.
pub struct Mpris {
    shown: Arc<Mutex<Shown>>,
    pub commands: Receiver<MediaCommand>,
    _connection: Connection,
}

impl Mpris {
    /// `None` without a session bus: the app works, only the media card does not know it.
    pub fn start() -> Option<Self> {
        let shown: Arc<Mutex<Shown>> = Arc::default();
        let (sender, commands) = channel();
        let control = Control { shown: shown.clone(), commands: sender };
        let name = format!("org.mpris.MediaPlayer2.consoleui.media.instance{}", std::process::id());
        let build = || {
            connection::Builder::session()?.name(name.as_str())?.serve_at(PATH, Root)?.serve_at(PATH, control)?.build()
        };
        match build() {
            Ok(connection) => Some(Self { shown, commands, _connection: connection }),
            Err(err) => {
                eprintln!("No MPRIS: {err}");
                None
            }
        }
    }

    pub fn show(&self, new: Shown) {
        if let Ok(mut shown) = self.shown.lock() {
            *shown = new;
        }
    }
}
