//! Control of our own compositor (`crates/compositor`, Epic 7) over the Unix socket in
//! `CONSOLEUI_COMPOSITOR`; the counterpart of `gamescope.rs`. The shell says which apps it
//! launched and what should be in front, the compositor maps the windows and reports back
//! (`consoleui_core::protocol`).
//!
//! Story 7.6: the quick menu and toasts are separate toplevels (`OverlayWindow`,
//! `NotificationWindow`); their titles are announced with `WindowRole`, and the compositor stacks
//! them above the app in front. They are used only if the compositor lists the feature in its
//! `Welcome`; with an older compositor the whole shell window comes to the front as before.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use consoleui_core::protocol::{
    self, BlurRegion, CompositorMessage, LineReader, ShellMessage, Target, Transition, WindowRole,
};

/// Title of `OverlayWindow` (`ui/overlay.slint`); the compositor recognizes the window by it.
pub const OVERLAY_TITLE: &str = "ConsoleUI Overlay";
/// Title of `NotificationWindow` (`ui/notification.slint`)
pub const NOTIFICATION_TITLE: &str = "ConsoleUI notification";

pub struct Compositor {
    stream: UnixStream,
    reader: LineReader,
    outgoing: Vec<u8>,
    connected: bool,
    /// Output size reported by the compositor
    output: Option<(u32, u32)>,
    /// Last overlay state sent (the quick menu re-renders often)
    overlay: bool,
    /// Optional features from the compositor's `Welcome`
    features: Vec<String>,
    /// Keymap the compositor reports for the seat keyboard, e.g. `de (nodeadkeys)`
    keymap: Option<String>,
    /// Blur regions last sent per window role (Story 7.13)
    blur: Vec<(WindowRole, Vec<BlurRegion>)>,
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
            features: Vec::new(),
            keymap: None,
            blur: Vec::new(),
        };
        compositor.send(&ShellMessage::Hello { version: protocol::VERSION, pid: std::process::id() });
        // An older compositor skips these; the windows are then never shown (see `overlays`).
        for (title, role) in [(OVERLAY_TITLE, WindowRole::Overlay), (NOTIFICATION_TITLE, WindowRole::Notification)] {
            compositor.send(&ShellMessage::WindowRole { title: title.to_owned(), role });
        }
        Some(compositor)
    }

    pub fn send(&mut self, message: &ShellMessage) {
        if !self.connected {
            return;
        }
        self.outgoing.extend_from_slice(protocol::encode(message).as_bytes());
        self.flush();
    }

    /// Brings the app (or the shell) to the front, animated if the compositor supports it and a
    /// transition is given (Story 7.7).
    pub fn focus(&mut self, app_id: Option<u32>, transition: Option<Transition>) {
        let target = match app_id {
            Some(app_id) => Target::App { app_id },
            None => Target::Shell,
        };
        let transition = transition.filter(|_| self.transitions());
        self.send(&ShellMessage::Focus { target, transition });
    }

    pub fn set_overlay(&mut self, open: bool) {
        if self.overlay != open {
            self.overlay = open;
            self.send(&ShellMessage::Overlay { open });
        }
    }

    /// The compositor blurs what is below the panels of the overlay and notification windows
    /// (Story 7.13).
    pub fn blur(&self) -> bool {
        self.features.iter().any(|f| f == protocol::FEATURE_BLUR)
    }

    /// Announces the blurred panels of the window with `role`, if they changed.
    pub fn set_blur(&mut self, role: WindowRole, regions: Vec<BlurRegion>, fade_ms: u32) {
        match self.blur.iter_mut().find(|(r, _)| *r == role) {
            Some((_, sent)) if *sent == regions => return,
            Some((_, sent)) => *sent = regions.clone(),
            None => self.blur.push((role, regions.clone())),
        }
        self.send(&ShellMessage::Blur { role, regions, fade_ms });
    }

    pub fn output_size(&self) -> Option<(u32, u32)> {
        self.output
    }

    /// The compositor places the overlay and notification windows above the app (Story 7.6).
    pub fn overlays(&self) -> bool {
        self.features.iter().any(|f| f == protocol::FEATURE_OVERLAYS)
    }

    /// The compositor animates front changes (Story 7.7).
    pub fn transitions(&self) -> bool {
        self.features.iter().any(|f| f == protocol::FEATURE_TRANSITIONS)
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
                CompositorMessage::Welcome { version, features } => {
                    if *version != protocol::VERSION {
                        eprintln!("compositor speaks protocol {version}, shell {}", protocol::VERSION);
                    }
                    self.features = features.clone();
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
