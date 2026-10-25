//! UI-Klänge über einen dauerhaft laufenden `pw-cat`, der rohes PCM von stdin abspielt.
//! Ein eigener Thread schreibt die Klänge, damit die UI nie auf die Pipe warten muss.

use std::collections::HashMap;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use consoleui_core::sound::{self, SAMPLE_RATE, Sound};

/// Schnelles Navigieren soll die Klänge nicht stauen; dichter folgende Fokusklänge fallen weg.
const MIN_MOVE_GAP: Duration = Duration::from_millis(60);

pub struct Audio {
    child: Child,
    sender: SyncSender<Sound>,
    last_move: Option<Instant>,
}

impl Audio {
    pub fn start() -> Option<Self> {
        let rate = SAMPLE_RATE.to_string();
        let mut child = Command::new("pw-cat")
            .args(["--playback", "--raw", "--format", "s16", "--rate", &rate, "--channels", "2"])
            .args(["--latency", "20ms", "--media-role", "Notification", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| eprintln!("Klänge nicht verfügbar (pw-cat): {err}"))
            .ok()?;
        let mut stdin = child.stdin.take()?;
        // Kleine Warteschlange: Was nicht hineinpasst, wird verworfen statt verzögert.
        let (sender, receiver) = mpsc::sync_channel::<Sound>(2);
        std::thread::spawn(move || {
            let mut cache: HashMap<Sound, Vec<u8>> = HashMap::new();
            for sound in receiver {
                let pcm = cache.entry(sound).or_insert_with(|| sound::render(sound));
                if stdin.write_all(pcm).and_then(|()| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        Some(Self { child, sender, last_move: None })
    }

    pub fn play(&mut self, sound: Sound) {
        if sound == Sound::Move {
            let now = Instant::now();
            if self.last_move.is_some_and(|last| now - last < MIN_MOVE_GAP) {
                return;
            }
            self.last_move = Some(now);
        }
        if let Err(TrySendError::Disconnected(_)) = self.sender.try_send(sound) {
            eprintln!("Klänge: pw-cat ist beendet");
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
