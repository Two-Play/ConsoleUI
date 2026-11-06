//! The running mpv: starts it, reads its IPC socket on a thread, sends it commands.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use consoleui_core::xdg;

use crate::mpv::{self, Control, Queue, Update};

/// mpv opens its socket right after the start.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long mpv gets to save the position when the app closes.
const QUIT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum PlayerEvent {
    Update(u64, Update),
    /// mpv ended; `failed` if it could not play what it got
    Gone(u64, bool),
}

/// One mpv. Events carry `id`, so those of the mpv before do not count for the next.
pub struct Player {
    pub id: u64,
    stream: Arc<Mutex<Option<UnixStream>>>,
    child: Arc<Mutex<Child>>,
    /// Lines from before the socket was there
    waiting: Arc<Mutex<Vec<String>>>,
}

fn socket_path(id: u64) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    dir.join(format!("consoleui-media-{}-{id}.sock", std::process::id()))
}

fn connect(path: &PathBuf, child: &Mutex<Child>) -> Option<UnixStream> {
    let start = Instant::now();
    while start.elapsed() < CONNECT_TIMEOUT {
        if let Ok(stream) = UnixStream::connect(path) {
            return Some(stream);
        }
        if !matches!(child.lock().ok()?.try_wait(), Ok(None)) {
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
    None
}

impl Player {
    pub fn start(id: u64, queue: &Queue, events: Sender<PlayerEvent>) -> io::Result<Self> {
        let socket = socket_path(id);
        let positions = xdg::data_home().map(|dir| dir.join("consoleui/media-positions"));
        if let Some(dir) = &positions {
            let _ = std::fs::create_dir_all(dir);
        }
        let child = Command::new("mpv")
            .args(mpv::arguments(queue, &socket, positions.as_deref()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let child = Arc::new(Mutex::new(child));
        let stream: Arc<Mutex<Option<UnixStream>>> = Arc::default();
        let waiting: Arc<Mutex<Vec<String>>> = Arc::default();
        let player = Self { id, stream: stream.clone(), child: child.clone(), waiting: waiting.clone() };
        thread::spawn(move || {
            if let Some(connected) = connect(&socket, &child) {
                read(id, connected, &stream, &waiting, &events);
            }
            let _ = std::fs::remove_file(&socket);
            // Polled, because `quit` and `Drop` need the lock in between
            let status = loop {
                match child.lock().map(|mut child| child.try_wait()) {
                    Ok(Ok(None)) => thread::sleep(Duration::from_millis(50)),
                    Ok(Ok(Some(status))) => break Some(status),
                    _ => break None,
                }
            };
            // 0 is the end of the queue and 4 a quit from outside. 3 is a queue with a broken file in
            // it, mpv goes past those. Everything else played nothing, or mpv crashed.
            let failed = status.is_some_and(|status| !matches!(status.code(), Some(0 | 3 | 4)));
            let _ = events.send(PlayerEvent::Gone(id, failed));
        });
        Ok(player)
    }

    fn send_line(&self, line: String) {
        let Ok(mut stream) = self.stream.lock() else { return };
        match stream.as_mut() {
            Some(stream) => {
                let _ = writeln!(stream, "{line}");
            }
            None => {
                if let Ok(mut waiting) = self.waiting.lock() {
                    waiting.push(line);
                }
            }
        }
    }

    pub fn control(&self, control: &Control) {
        // Several commands go as several lines
        for line in mpv::line(control).split("; ") {
            self.send_line(line.to_string());
        }
    }

    pub fn quit(&self) {
        self.send_line(mpv::QUIT.to_string());
    }

    /// Before the app closes: lets mpv save the position, then makes sure no sound stays behind.
    pub fn finish(&self) {
        self.quit();
        let start = Instant::now();
        while start.elapsed() < QUIT_TIMEOUT {
            match self.child.lock().map(|mut child| child.try_wait()) {
                Ok(Ok(None)) => thread::sleep(Duration::from_millis(20)),
                _ => return,
            }
        }
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

fn read(
    id: u64,
    connected: UnixStream,
    stream: &Mutex<Option<UnixStream>>,
    waiting: &Mutex<Vec<String>>,
    events: &Sender<PlayerEvent>,
) {
    let Ok(mut writer) = connected.try_clone() else { return };
    for line in mpv::observe_lines() {
        let _ = writeln!(writer, "{line}");
    }
    if let (Ok(mut stream), Ok(mut waiting)) = (stream.lock(), waiting.lock()) {
        for line in waiting.drain(..) {
            let _ = writeln!(writer, "{line}");
        }
        *stream = Some(writer);
    }
    for line in BufReader::new(connected).lines() {
        let Ok(line) = line else { break };
        if let Some(update) = mpv::parse(&line)
            && events.send(PlayerEvent::Update(id, update)).is_err()
        {
            break;
        }
    }
}
