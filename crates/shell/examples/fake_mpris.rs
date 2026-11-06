//! A stand-in media player on the session bus for testing the quick menu's media card. Prints every
//! command it gets, so a script can check them: `cargo run -p consoleui --example fake_mpris`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use zbus::blocking::connection;
use zbus::zvariant::{OwnedValue, Value};

const TRACKS: [(&str, &str); 3] =
    [("First Song", "The Testers"), ("Second Song", "The Testers"), ("Third Song", "Mock")];

#[derive(Default)]
struct State {
    track: usize,
    paused: bool,
}

struct Root;

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    fn identity(&self) -> String {
        "Fake Player".into()
    }
}

#[derive(Default)]
struct Player(Mutex<State>);

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn play_pause(&self) {
        let mut state = self.0.lock().unwrap();
        state.paused = !state.paused;
        println!("PlayPause -> paused={}", state.paused);
    }

    fn next(&self) {
        let mut state = self.0.lock().unwrap();
        state.track = (state.track + 1).min(TRACKS.len() - 1);
        println!("Next -> {}", TRACKS[state.track].0);
    }

    fn previous(&self) {
        let mut state = self.0.lock().unwrap();
        state.track = state.track.saturating_sub(1);
        println!("Previous -> {}", TRACKS[state.track].0);
    }

    #[zbus(property)]
    fn playback_status(&self) -> String {
        if self.0.lock().unwrap().paused { "Paused" } else { "Playing" }.into()
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.0.lock().unwrap().track + 1 < TRACKS.len()
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.0.lock().unwrap().track > 0
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let (title, artist) = TRACKS[self.0.lock().unwrap().track];
        let owned = |value: Value| value.try_to_owned().expect("no file descriptors in here");
        HashMap::from([
            ("xesam:title".to_string(), owned(Value::from(title))),
            ("xesam:artist".to_string(), owned(Value::from(vec![artist]))),
        ])
    }
}

fn main() -> zbus::Result<()> {
    let _connection = connection::Builder::session()?
        .name("org.mpris.MediaPlayer2.fake")?
        .serve_at("/org/mpris/MediaPlayer2", Root)?
        .serve_at("/org/mpris/MediaPlayer2", Player::default())?
        .build()?;
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
