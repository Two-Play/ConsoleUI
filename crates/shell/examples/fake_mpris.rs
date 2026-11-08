//! A stand-in media player on the session bus for testing the quick menu's media card, the player
//! window and the "now playing" note. Prints every command it gets, so a script can check them:
//! `cargo run -p consoleui --example fake_mpris [cover.png]`. Next/Previous announce the new track
//! with `PropertiesChanged`, like Spotify does.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use zbus::blocking::connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const TRACKS: [(&str, &str, &str); 3] = [
    ("First Song", "The Testers", "Test Record"),
    ("Second Song", "The Testers", "Test Record"),
    ("Third Song", "Mock", "B-Sides"),
];

#[derive(Default)]
struct State {
    track: usize,
    paused: bool,
    shuffle: bool,
    position: i64,
}

struct Root;

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    fn identity(&self) -> String {
        "Fake Player".into()
    }
}

struct Player {
    state: Mutex<State>,
    art: String,
}

impl Player {
    fn step(&self, forward: bool) {
        let mut state = self.state.lock().unwrap();
        state.track = if forward { (state.track + 1).min(TRACKS.len() - 1) } else { state.track.saturating_sub(1) };
        state.position = 0;
        println!("{} -> {}", if forward { "Next" } else { "Previous" }, TRACKS[state.track].0);
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    async fn play_pause(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let paused = {
            let mut state = self.state.lock().unwrap();
            state.paused = !state.paused;
            state.paused
        };
        println!("PlayPause -> paused={paused}");
        let _ = self.playback_status_changed(&emitter).await;
    }

    async fn next(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        self.step(true);
        let _ = self.metadata_changed(&emitter).await;
    }

    async fn previous(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        self.step(false);
        let _ = self.metadata_changed(&emitter).await;
    }

    fn seek(&self, offset: i64) {
        let mut state = self.state.lock().unwrap();
        state.position = (state.position + offset).max(0);
        println!("Seek {offset} -> {}", state.position);
    }

    #[zbus(property)]
    fn playback_status(&self) -> String {
        if self.state.lock().unwrap().paused { "Paused" } else { "Playing" }.into()
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        self.state.lock().unwrap().position
    }

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        self.state.lock().unwrap().shuffle
    }

    #[zbus(property)]
    fn set_shuffle(&self, on: bool) {
        self.state.lock().unwrap().shuffle = on;
        println!("Shuffle -> {on}");
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.state.lock().unwrap().track + 1 < TRACKS.len()
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.state.lock().unwrap().track > 0
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let track = self.state.lock().unwrap().track;
        let (title, artist, album) = TRACKS[track];
        let owned = |value: Value| value.try_to_owned().expect("no file descriptors in here");
        let id = format!("/org/consoleui/fake/track{track}");
        let mut metadata = HashMap::from([
            ("mpris:trackid".to_string(), owned(Value::from(ObjectPath::try_from(id).unwrap()))),
            ("xesam:title".to_string(), owned(Value::from(title))),
            ("xesam:artist".to_string(), owned(Value::from(vec![artist]))),
            ("xesam:album".to_string(), owned(Value::from(album))),
            ("mpris:length".to_string(), owned(Value::from(200_000_000u64))),
        ]);
        if !self.art.is_empty() {
            metadata.insert("mpris:artUrl".to_string(), owned(Value::from(format!("file://{}", self.art))));
        }
        metadata
    }
}

fn main() -> zbus::Result<()> {
    let art = std::env::args().nth(1).unwrap_or_default();
    let player = Player { state: Mutex::new(State { position: 63_000_000, ..State::default() }), art };
    let connection = connection::Builder::session()?
        .name("org.mpris.MediaPlayer2.fake")?
        .serve_at("/org/mpris/MediaPlayer2", Root)?
        .serve_at("/org/mpris/MediaPlayer2", player)?
        .build()?;
    // Ends with its session bus (a test's private one goes away with the test)
    loop {
        std::thread::sleep(Duration::from_secs(2));
        if !zbus::blocking::fdo::DBusProxy::new(&connection).is_ok_and(|bus| bus.get_id().is_ok()) {
            return Ok(());
        }
    }
}
