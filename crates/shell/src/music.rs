//! Background music on the home screen (settings row "Background music"), like the menu music of
//! a console. It plays only while the shell is in front and stops while an app is.
//!
//! A thread of its own does the work, so the UI never waits for a pipe:
//! - Ambient: the loop from `consoleui_core::music` (rendered once on that thread) goes to a
//!   `pw-cat --raw` process in small chunks; it fades in and out over [`FADE`].
//! - My music: `pw-cat` plays the files of the music folder one after another (shuffled); it is
//!   paused with `SIGSTOP` and resumed with `SIGCONT`.
//! - Game themes (own setting): `ffmpeg` decodes the focused game's theme (from the cache, see
//!   `themes.rs`) and it goes to `pw-cat --raw` like the ambient loop, so it fades in, out and
//!   over to the next. While it plays the background music fades out (ambient) or pauses (files),
//!   and comes back when there is no theme.

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
/// Loudness of the themes: a little below what streaming services aim at, under the UI sounds
const LOUDNESS: &str = "loudnorm=I=-18:TP=-2:LRA=11";
/// Fade of a game's theme: shorter than the ambient one, the focus moves on quickly
const THEME_FADE: Duration = Duration::from_millis(900);

enum Control {
    Choice(MusicChoice),
    Track(Option<String>),
    Active(bool),
    /// The theme of the focused game on the home screen ("Game themes")
    Theme(Option<PathBuf>),
}

pub struct Music {
    sender: Sender<Control>,
    choice: MusicChoice,
    track: Option<String>,
    active: bool,
    theme: Option<PathBuf>,
}

impl Music {
    pub fn start() -> Self {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || run(receiver));
        Self { sender, choice: MusicChoice::Off, track: None, active: false, theme: None }
    }

    /// What should play (`track`: the one file of the music folder to repeat, else all shuffled),
    /// and whether the shell is in front (only then it plays).
    pub fn set(&mut self, choice: MusicChoice, track: Option<&str>, active: bool) {
        if choice != self.choice {
            self.choice = choice;
            let _ = self.sender.send(Control::Choice(choice));
        }
        if track != self.track.as_deref() {
            self.track = track.map(str::to_owned);
            let _ = self.sender.send(Control::Track(self.track.clone()));
        }
        if active != self.active {
            self.active = active;
            let _ = self.sender.send(Control::Active(active));
        }
    }

    /// The theme to play over the background music; `None` fades it out and the background
    /// music comes back.
    pub fn set_theme(&mut self, theme: Option<PathBuf>) {
        if theme != self.theme {
            self.theme = theme.clone();
            let _ = self.sender.send(Control::Theme(theme));
        }
    }
}

fn run(receiver: Receiver<Control>) {
    let mut choice = MusicChoice::Off;
    let mut track: Option<String> = None;
    let mut active = false;
    let mut ambient: Option<Ambient> = None;
    let mut folder: Option<Folder> = None;
    let mut theme_file: Option<PathBuf> = None;
    let mut theme: Option<Theme> = None;
    loop {
        // Writing the ambient loop paces itself on the pipe; otherwise wait for news.
        let writing = ambient.as_ref().is_some_and(|a| active || a.gain > 0.0) || theme.is_some();
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
            Some(Control::Track(new)) => {
                track = new;
                // The new track starts at once
                folder = None;
            }
            Some(Control::Active(new)) => active = new,
            Some(Control::Theme(new)) => theme_file = new,
            None => {}
        }

        // A theme takes the place of the background music while it plays
        let wanted_theme = theme_file.clone().filter(|_| active);
        let background = active && wanted_theme.is_none();
        if choice != MusicChoice::Ambient {
            ambient = None;
        }
        if choice != MusicChoice::Folder {
            folder = None;
        }
        match choice {
            MusicChoice::Off => {}
            MusicChoice::Ambient => {
                if ambient.is_none() && background {
                    ambient = Ambient::start();
                }
                if let Some(player) = &mut ambient
                    && (background || player.gain > 0.0)
                    && !player.write_chunk(background)
                {
                    eprintln!("Music: pw-cat has exited");
                    ambient = None;
                    choice = MusicChoice::Off;
                }
            }
            MusicChoice::Folder => {
                if folder.is_none() && background {
                    folder = Folder::new(track.as_deref());
                    if folder.is_none() {
                        eprintln!("Music: no playable files in {:?}", music::music_dir());
                        choice = MusicChoice::Off;
                    }
                }
                if let Some(player) = &mut folder {
                    player.update(background);
                }
            }
        }
        if let Some(player) = &mut theme {
            // Another game: this one fades out first, then the next one starts
            let keep = wanted_theme.as_ref() == Some(&player.file);
            if !player.write_chunk(keep) || (!keep && player.gain == 0.0) {
                theme = None;
            }
        }
        if theme.is_none()
            && let Some(file) = &wanted_theme
        {
            theme = Theme::start(file);
            if theme.is_none() {
                // Unplayable: do not try it again on every turn
                theme_file = None;
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

/// A game's theme: `ffmpeg` decodes it to raw PCM, on repeat, and it goes to `pw-cat --raw`
struct Theme {
    file: PathBuf,
    player: Child,
    stdin: ChildStdin,
    decoder: Child,
    gain: f32,
}

impl Theme {
    fn start(file: &std::path::Path) -> Option<Self> {
        let rate = SAMPLE_RATE.to_string();
        let mut player = Command::new("pw-cat")
            .args(["--playback", "--raw", "--format", "s16", "--rate", &rate, "--channels", "2"])
            .args(["--latency", "100ms", "--media-role", "Music", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| eprintln!("Music not available (pw-cat): {err}"))
            .ok()?;
        let stdin = player.stdin.take()?;
        let decoder = match decode(file) {
            Ok(decoder) => decoder,
            Err(err) => {
                eprintln!("Theme music needs ffmpeg: {err}");
                let _ = player.kill();
                let _ = player.wait();
                return None;
            }
        };
        Some(Self { file: file.to_path_buf(), player, stdin, decoder, gain: 0.0 })
    }

    /// Writes the next chunk while fading towards `on`; false if `pw-cat` is gone.
    fn write_chunk(&mut self, on: bool) -> bool {
        use std::io::Read;
        let mut pcm = vec![0u8; CHUNK * 4];
        let mut filled = 0;
        while filled < pcm.len() {
            let read = self.decoder.stdout.as_mut().map_or(Ok(0), |out| out.read(&mut pcm[filled..]));
            match read {
                Ok(0) | Err(_) => {
                    // The end: again from the start (the rest of this chunk stays silent)
                    let _ = self.decoder.kill();
                    let _ = self.decoder.wait();
                    match decode(&self.file) {
                        Ok(decoder) => self.decoder = decoder,
                        Err(_) => return false,
                    }
                    break;
                }
                Ok(count) => filled += count,
            }
        }
        let step = 1.0 / (THEME_FADE.as_secs_f32() * SAMPLE_RATE as f32);
        let mut bytes = Vec::with_capacity(pcm.len());
        for frame in pcm.as_chunks::<4>().0 {
            self.gain = if on { (self.gain + step).min(1.0) } else { (self.gain - step).max(0.0) };
            let gain = (self.gain * std::f32::consts::FRAC_PI_2).sin() * VOLUME;
            for sample in frame.as_chunks::<2>().0 {
                let value = (i16::from_le_bytes([sample[0], sample[1]]) as f32 * gain) as i16;
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        self.stdin.write_all(&bytes).and_then(|()| self.stdin.flush()).is_ok()
    }
}

fn decode(file: &std::path::Path) -> std::io::Result<Child> {
    let rate = SAMPLE_RATE.to_string();
    let mut command = Command::new("ffmpeg");
    crate::themes::low_priority(&mut command)
        .args(["-v", "quiet", "-i"])
        .arg(file)
        // Every theme about as loud as the others (EBU R128), whatever the video was mixed at
        .args(["-af", LOUDNESS])
        .args(["-f", "s16le", "-ar", &rate, "-ac", "2", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

impl Drop for Theme {
    fn drop(&mut self) {
        for child in [&mut self.decoder, &mut self.player] {
            let _ = child.kill();
            let _ = child.wait();
        }
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
    /// `track`: only the file with this name, on repeat; else all files, shuffled.
    fn new(track: Option<&str>) -> Option<Self> {
        let mut tracks = music::tracks(&music::music_dir()?);
        if tracks.is_empty() {
            return None;
        }
        if let Some(only) = track.and_then(|name| tracks.iter().find(|t| music::track_name(t).as_deref() == Some(name)))
        {
            return Some(Self { tracks: vec![only.clone()], next: 0, current: None, paused: false });
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
