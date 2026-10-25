//! Background music on the home screen (settings row "Background music"), like the menu music of
//! a console. It plays only while the shell is in front and stops while an app is.
//!
//! A thread of its own does the work, so the UI never waits for a pipe:
//! - Ambient: the loop from `consoleui_core::music` (rendered once on that thread) goes to a
//!   `pw-cat --raw` process in small chunks; it fades in and out over [`FADE`].
//! - My music: `pw-cat` plays the files of the music folder one after another (shuffled); it is
//!   paused with `SIGSTOP` and resumed with `SIGCONT`.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

use consoleui_core::music;
use consoleui_core::settings::MusicChoice;
use consoleui_core::sound::SAMPLE_RATE;

/// Fade in/out of the ambient loop
const FADE: Duration = Duration::from_millis(1500);
/// Volume of the music relative to full scale (the UI sounds stay audible above it)
const VOLUME: f32 = 0.7;
/// Frames written per chunk; the pipe and `pw-cat` buffer a few of them.
const CHUNK: usize = 2400;

enum Control {
    Choice(MusicChoice),
    Active(bool),
}

pub struct Music {
    sender: Sender<Control>,
    choice: MusicChoice,
    active: bool,
}

impl Music {
    pub fn start() -> Self {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || run(receiver));
        Self { sender, choice: MusicChoice::Off, active: false }
    }

    /// What should play, and whether the shell is in front (only then it plays).
    pub fn set(&mut self, choice: MusicChoice, active: bool) {
        if choice != self.choice {
            self.choice = choice;
            let _ = self.sender.send(Control::Choice(choice));
        }
        if active != self.active {
            self.active = active;
            let _ = self.sender.send(Control::Active(active));
        }
    }
}

fn run(receiver: Receiver<Control>) {
    let mut choice = MusicChoice::Off;
    let mut active = false;
    let mut ambient: Option<Ambient> = None;
    let mut folder: Option<Folder> = None;
    loop {
        // Writing the ambient loop paces itself on the pipe; otherwise wait for news.
        let writing = ambient.as_ref().is_some_and(|a| active || a.gain > 0.0);
        let control = if writing {
            match receiver.try_recv() {
                Ok(control) => Some(control),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else {
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(control) => Some(control),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        match control {
            Some(Control::Choice(new)) => choice = new,
            Some(Control::Active(new)) => active = new,
            None => {}
        }

        if choice != MusicChoice::Ambient {
            ambient = None;
        }
        if choice != MusicChoice::Folder {
            folder = None;
        }
        match choice {
            MusicChoice::Off => {}
            MusicChoice::Ambient => {
                if ambient.is_none() && active {
                    ambient = Ambient::start();
                }
                if let Some(player) = &mut ambient
                    && (active || player.gain > 0.0)
                    && !player.write_chunk(active)
                {
                    eprintln!("Music: pw-cat has exited");
                    ambient = None;
                    choice = MusicChoice::Off;
                }
            }
            MusicChoice::Folder => {
                if folder.is_none() && active {
                    folder = Folder::new();
                    if folder.is_none() {
                        eprintln!("Music: no playable files in {:?}", music::music_dir());
                        choice = MusicChoice::Off;
                    }
                }
                if let Some(player) = &mut folder {
                    player.update(active);
                }
            }
        }
    }
}

/// The generated loop through `pw-cat --raw`
struct Ambient {
    child: Child,
    stdin: ChildStdin,
    samples: Vec<i16>,
    /// Next frame of the loop
    position: usize,
    gain: f32,
}

impl Ambient {
    fn start() -> Option<Self> {
        let rate = SAMPLE_RATE.to_string();
        let mut child = Command::new("pw-cat")
            .args(["--playback", "--raw", "--format", "s16", "--rate", &rate, "--channels", "2"])
            .args(["--latency", "100ms", "--media-role", "Music", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| eprintln!("Music not available (pw-cat): {err}"))
            .ok()?;
        let stdin = child.stdin.take()?;
        let samples = music::ambient_loop(music::CHORD_SECONDS);
        Some(Self { child, stdin, samples, position: 0, gain: 0.0 })
    }

    /// Writes the next chunk while fading towards `active`; false if `pw-cat` is gone.
    fn write_chunk(&mut self, active: bool) -> bool {
        let frames = self.samples.len() / 2;
        let step = 1.0 / (FADE.as_secs_f32() * SAMPLE_RATE as f32);
        let mut bytes = Vec::with_capacity(CHUNK * 4);
        for _ in 0..CHUNK {
            self.gain = if active { (self.gain + step).min(1.0) } else { (self.gain - step).max(0.0) };
            // Equal-power curve: the fade sounds even
            let gain = (self.gain * std::f32::consts::FRAC_PI_2).sin() * VOLUME;
            for channel in 0..2 {
                let sample = (self.samples[self.position * 2 + channel] as f32 * gain) as i16;
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
            self.position = (self.position + 1) % frames;
        }
        self.stdin.write_all(&bytes).and_then(|()| self.stdin.flush()).is_ok()
    }
}

impl Drop for Ambient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The user's files, one `pw-cat` per track
struct Folder {
    tracks: Vec<PathBuf>,
    next: usize,
    current: Option<Child>,
    paused: bool,
}

impl Folder {
    fn new() -> Option<Self> {
        let mut tracks = music::tracks(&music::music_dir()?);
        if tracks.is_empty() {
            return None;
        }
        // Shuffled with the clock as seed (Fisher–Yates with a small LCG)
        let mut seed =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() | 1);
        for i in (1..tracks.len()).rev() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            tracks.swap(i, (seed >> 8) as usize % (i + 1));
        }
        Some(Self { tracks, next: 0, current: None, paused: false })
    }

    fn update(&mut self, active: bool) {
        if let Some(child) = &mut self.current
            && !matches!(child.try_wait(), Ok(None))
        {
            self.current = None;
        }
        match (&self.current, active) {
            (None, true) => self.play_next(),
            (Some(child), _) if active == self.paused => {
                let signal = if active { libc::SIGCONT } else { libc::SIGSTOP };
                // Safety: plain kill(2) on our own child process.
                unsafe {
                    libc::kill(child.id() as libc::pid_t, signal);
                }
                self.paused = !active;
            }
            _ => {}
        }
    }

    fn play_next(&mut self) {
        let track = &self.tracks[self.next % self.tracks.len()];
        self.next += 1;
        let volume = VOLUME.to_string();
        match Command::new("pw-cat")
            .args(["--playback", "--media-role", "Music", "--volume", &volume])
            .arg(track)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                self.current = Some(child);
                self.paused = false;
            }
            Err(err) => eprintln!("Music: cannot play {}: {err}", track.display()),
        }
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        if let Some(child) = &mut self.current {
            // Resume a paused track first, so its PipeWire stream goes away cleanly.
            // Safety: plain kill(2) on our own child process.
            unsafe {
                libc::kill(child.id() as libc::pid_t, libc::SIGCONT);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
