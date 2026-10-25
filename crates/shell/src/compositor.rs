//! Control of our own compositor (`crates/compositor`, Epic 7) over the Unix socket in
//! `CONSOLEUI_COMPOSITOR`; the counterpart of `gamescope.rs`. The shell says which apps it
//! launched and what should be in front, the compositor maps the windows and reports back
//! (`consoleui_core::protocol`).

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use consoleui_core::protocol::{self, CompositorMessage, LineReader, ShellMessage, Target};

pub struct Compositor {
    stream: UnixStream,
    reader: LineReader,
    outgoing: Vec<u8>,
    connected: bool,
    /// Output size reported by the compositor
    output: Option<(u32, u32)>,
    /// Last overlay state sent (the quick menu re-renders often)
    overlay: bool,
    /// Keymap the compositor reports for the seat keyboard, e.g. `de (nodeadkeys)`
    keymap: Option<String>,
}

impl Compositor {
    /// Only inside our compositor, which sets `CONSOLEUI_COMPOSITOR`.
    pub fn connect() -> Option<Self> {
        let path = std::env::var_os(protocol::SOCKET_ENV)?;
        let stream = match UnixStream::connect(&path) {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("compositor control socket {} not available: {err}", path.to_string_lossy());
                return None;
            }
        };
        if let Err(err) = stream.set_nonblocking(true) {
            eprintln!("compositor control socket: {err}");
            return None;
        }
        let mut compositor = Self {
            stream,
            reader: LineReader::default(),
            outgoing: Vec::new(),
            connected: true,
            output: None,
            overlay: false,
            keymap: None,
        };
        compositor.send(&ShellMessage::Hello { version: protocol::VERSION, pid: std::process::id() });
        Some(compositor)
    }

    pub fn send(&mut self, message: &ShellMessage) {
        if !self.connected {
            return;
        }
        self.outgoing.extend_from_slice(protocol::encode(message).as_bytes());
        self.flush();
    }

    pub fn focus(&mut self, app_id: Option<u32>) {
        let target = match app_id {
            Some(app_id) => Target::App { app_id },
            None => Target::Shell,
        };
        self.send(&ShellMessage::Focus { target });
    }

    pub fn set_overlay(&mut self, open: bool) {
        if self.overlay != open {
            self.overlay = open;
            self.send(&ShellMessage::Overlay { open });
        }
    }

    pub fn output_size(&self) -> Option<(u32, u32)> {
        self.output
    }

    pub fn keymap(&self) -> Option<String> {
        self.keymap.clone()
    }

    /// Messages that arrived since the last call.
    pub fn poll(&mut self) -> Vec<CompositorMessage> {
        self.flush();
        let mut buffer = [0u8; 4096];
        while self.connected {
            match self.stream.read(&mut buffer) {
                Ok(0) => self.disconnect("closed by the compositor"),
                Ok(n) => self.reader.push(&buffer[..n]),
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) => self.disconnect(&err.to_string()),
            }
        }
        let mut messages = Vec::new();
        while let Some(message) = self.reader.next_message::<CompositorMessage>() {
            match &message {
                CompositorMessage::Output { width, height } => self.output = Some((*width, *height)),
                CompositorMessage::Keymap { layout, variant } => {
                    let keymap = consoleui_core::keymap::Keymap {
                        layout: layout.clone(),
                        variant: variant.clone(),
                        ..Default::default()
                    };
                    self.keymap = Some(keymap.describe());
                }
                CompositorMessage::Welcome { version } if *version != protocol::VERSION => {
                    eprintln!("compositor speaks protocol {version}, shell {}", protocol::VERSION);
                }
                _ => {}
            }
            messages.push(message);
        }
        messages
    }

    fn flush(&mut self) {
        while self.connected && !self.outgoing.is_empty() {
            match self.stream.write(&self.outgoing) {
                Ok(0) => self.disconnect("closed by the compositor"),
                Ok(n) => {
                    self.outgoing.drain(..n);
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) => self.disconnect(&err.to_string()),
            }
        }
    }

    fn disconnect(&mut self, reason: &str) {
        if self.connected {
            eprintln!("compositor control socket: {reason}");
        }
        self.connected = false;
        self.outgoing.clear();
    }
}
