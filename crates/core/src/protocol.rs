//! Control protocol between the shell and its own compositor (Epic 7, `crates/compositor`).
//!
//! The compositor starts the shell with `CONSOLEUI_COMPOSITOR=<path>`; the shell connects to that
//! Unix socket and both sides exchange line-delimited JSON, one message per line. This replaces
//! the gamescope tricks (`STEAM_GAME` atoms, `GAMESCOPECTRL_BASELAYER_APPID`, overlay windows):
//! the shell says which app it launched and what should be in front, the compositor maps windows
//! to those apps (with [`crate::ownership`]) and reports what it sees.
//!
//! Unknown message types or malformed lines are skipped by the reader, so either side can add
//! messages later (7.5 input grab, 7.6 overlay toplevels, 7.7 transitions, 7.12 screenshots)
//! without breaking an older peer. Fields that may be added later must be `Option` or
//! `#[serde(default)]`.

use serde::{Deserialize, Serialize};

use crate::ownership::Owner;

/// Environment variable that holds the control socket path for the shell.
pub const SOCKET_ENV: &str = "CONSOLEUI_COMPOSITOR";
/// Bumped on incompatible changes; both sides log a mismatch but keep going.
pub const VERSION: u32 = 1;

/// What should be in front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Shell,
    /// A launched app (the `app_id` from [`ShellMessage::Launched`])
    App {
        app_id: u32,
    },
}

/// Shell → compositor
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ShellMessage {
    /// First message after connecting. `pid` is the shell process itself, so the compositor
    /// recognizes the shell's windows even when it was started through a wrapper script.
    Hello { version: u32, pid: u32 },
    /// The shell started an app. Its windows are recognized like in the gamescope session: by
    /// `SteamAppId` in the environment, the systemd scope unit, or the process and its children.
    Launched {
        app_id: u32,
        /// The launched process (`systemd-run` keeps the PID)
        #[serde(default)]
        pid: Option<u32>,
        /// Transient systemd scope unit, e.g. `consoleui-app-123.scope`
        #[serde(default)]
        scope: Option<String>,
        /// Steam game (`steam://rungameid/<id>`); its processes carry `SteamAppId`
        #[serde(default)]
        steam_app_id: Option<u32>,
    },
    /// The app has exited; the compositor forgets it.
    Exited { app_id: u32 },
    /// Bring the shell or an app to the front. An app without a window yet stays behind the
    /// shell until its first window shows content.
    Focus { target: Target },
    /// The quick menu is open: the shell is drawn above the app and gets the keyboard. Story 7.5
    /// will also cut the app's input here, 7.6 will show only the shell's overlay toplevel.
    Overlay { open: bool },
    /// Save the composed output as PNG (development, basis for Story 7.12).
    Screenshot { path: String },
    /// Layout of the physical keyboard (settings row "Keyboard layout"): an xkb layout name such
    /// as `de`, or `None` for the system layout the compositor detected. The shell sends it after
    /// `Hello` and on every change; the compositor answers with [`CompositorMessage::Keymap`].
    KeyboardLayout {
        #[serde(default)]
        layout: Option<String>,
    },
}

/// Compositor → shell
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CompositorMessage {
    /// Answer to [`ShellMessage::Hello`]
    Welcome { version: u32 },
    /// Size of the output in pixels (sent on connect and when it changes)
    Output { width: u32, height: u32 },
    /// A window of a launched app shows content for the first time.
    WindowMapped {
        app_id: u32,
        #[serde(default)]
        title: String,
    },
    /// The last window of an app is gone (the process may still run).
    AppGone { app_id: u32 },
    /// What is actually in front now (e.g. the shell again after the app closed its window).
    FocusChanged { target: Target },
    /// Result of [`ShellMessage::Screenshot`]
    ScreenshotDone {
        path: String,
        #[serde(default)]
        error: Option<String>,
    },
    /// The keymap the seat keyboard uses now (sent on connect and when it changes). `layout` may
    /// be a comma-separated list; empty means xkbcommon's default (`us`).
    Keymap {
        layout: String,
        #[serde(default)]
        variant: String,
    },
}

impl ShellMessage {
    /// How the compositor recognizes the processes of a [`ShellMessage::Launched`] app.
    pub fn launched_owner(&self) -> Option<Owner> {
        let ShellMessage::Launched { pid, scope, steam_app_id, .. } = self else { return None };
        steam_app_id.map(Owner::Steam).or_else(|| scope.clone().map(Owner::Scope)).or_else(|| pid.map(Owner::Process))
    }
}

/// One message as a line (JSON plus `\n`).
pub fn encode<T: Serialize>(message: &T) -> String {
    // Serializing these plain enums cannot fail.
    let mut line = serde_json::to_string(message).unwrap_or_default();
    line.push('\n');
    line
}

/// Collects bytes from a non-blocking socket and yields complete messages.
#[derive(Debug, Default)]
pub struct LineReader {
    buffer: Vec<u8>,
}

/// Longest line we accept; anything longer is dropped (protects against a stuck peer).
const MAX_LINE: usize = 64 * 1024;

impl LineReader {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > MAX_LINE && !self.buffer.contains(&b'\n') {
            self.buffer.clear();
        }
    }

    /// Next complete message; malformed lines and unknown message types are skipped.
    pub fn next_message<T: for<'de> Deserialize<'de>>(&mut self) -> Option<T> {
        while let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = &line[..line.len() - 1];
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            if let Ok(message) = serde_json::from_slice(line) {
                return Some(message);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_have_a_stable_json_shape() {
        assert_eq!(
            encode(&ShellMessage::Focus { target: Target::Shell }),
            "{\"type\":\"Focus\",\"target\":{\"kind\":\"shell\"}}\n"
        );
        assert_eq!(
            encode(&ShellMessage::Focus { target: Target::App { app_id: 7 } }),
            "{\"type\":\"Focus\",\"target\":{\"kind\":\"app\",\"app_id\":7}}\n"
        );
        assert_eq!(encode(&CompositorMessage::AppGone { app_id: 3 }), "{\"type\":\"AppGone\",\"app_id\":3}\n");
        assert_eq!(
            encode(&ShellMessage::KeyboardLayout { layout: Some("ch".into()) }),
            "{\"type\":\"KeyboardLayout\",\"layout\":\"ch\"}\n"
        );
    }

    #[test]
    fn round_trips_every_message() {
        let shell = [
            ShellMessage::Hello { version: VERSION, pid: 42 },
            ShellMessage::Launched {
                app_id: 1,
                pid: Some(9),
                scope: Some("consoleui-app-1.scope".into()),
                steam_app_id: None,
            },
            ShellMessage::Exited { app_id: 1 },
            ShellMessage::Focus { target: Target::App { app_id: 1 } },
            ShellMessage::Overlay { open: true },
            ShellMessage::Screenshot { path: "/tmp/a.png".into() },
            ShellMessage::KeyboardLayout { layout: Some("de".into()) },
            ShellMessage::KeyboardLayout { layout: None },
        ];
        let mut reader = LineReader::default();
        for message in &shell {
            reader.push(encode(message).as_bytes());
        }
        for message in &shell {
            assert_eq!(reader.next_message::<ShellMessage>().as_ref(), Some(message));
        }
        assert_eq!(reader.next_message::<ShellMessage>(), None);

        let compositor = [
            CompositorMessage::Welcome { version: VERSION },
            CompositorMessage::Output { width: 1920, height: 1080 },
            CompositorMessage::WindowMapped { app_id: 5, title: "glxgears".into() },
            CompositorMessage::AppGone { app_id: 5 },
            CompositorMessage::FocusChanged { target: Target::Shell },
            CompositorMessage::ScreenshotDone { path: "/tmp/a.png".into(), error: None },
            CompositorMessage::Keymap { layout: "de".into(), variant: "nodeadkeys".into() },
        ];
        for message in &compositor {
            reader.push(encode(message).as_bytes());
            assert_eq!(reader.next_message::<CompositorMessage>().as_ref(), Some(message));
        }
    }

    #[test]
    fn reads_partial_lines_and_skips_garbage() {
        let mut reader = LineReader::default();
        let line = encode(&ShellMessage::Overlay { open: false });
        let (a, b) = line.split_at(5);
        reader.push(a.as_bytes());
        assert_eq!(reader.next_message::<ShellMessage>(), None);
        reader.push(b.as_bytes());
        assert_eq!(reader.next_message::<ShellMessage>(), Some(ShellMessage::Overlay { open: false }));
        reader.push(b"not json\n\n{\"type\":\"FromTheFuture\",\"x\":1}\n");
        reader.push(line.as_bytes());
        assert_eq!(reader.next_message::<ShellMessage>(), Some(ShellMessage::Overlay { open: false }));
        assert_eq!(reader.next_message::<ShellMessage>(), None);
    }

    #[test]
    fn optional_fields_may_be_missing() {
        let mut reader = LineReader::default();
        reader.push(b"{\"type\":\"Launched\",\"app_id\":4}\n{\"type\":\"WindowMapped\",\"app_id\":4}\n");
        reader.push(b"{\"type\":\"KeyboardLayout\"}\n{\"type\":\"Keymap\",\"layout\":\"de\"}\n");
        assert_eq!(
            reader.next_message::<ShellMessage>(),
            Some(ShellMessage::Launched { app_id: 4, pid: None, scope: None, steam_app_id: None })
        );
        assert_eq!(
            reader.next_message::<CompositorMessage>(),
            Some(CompositorMessage::WindowMapped { app_id: 4, title: String::new() })
        );
        assert_eq!(reader.next_message::<ShellMessage>(), Some(ShellMessage::KeyboardLayout { layout: None }));
        assert_eq!(
            reader.next_message::<CompositorMessage>(),
            Some(CompositorMessage::Keymap { layout: "de".into(), variant: String::new() })
        );
    }

    #[test]
    fn owner_prefers_steam_then_scope_then_process() {
        let launched = |pid, scope: Option<&str>, steam| ShellMessage::Launched {
            app_id: 1,
            pid,
            scope: scope.map(Into::into),
            steam_app_id: steam,
        };
        assert_eq!(launched(Some(5), Some("a.scope"), Some(730)).launched_owner(), Some(Owner::Steam(730)));
        assert_eq!(launched(Some(5), Some("a.scope"), None).launched_owner(), Some(Owner::Scope("a.scope".into())));
        assert_eq!(launched(Some(5), None, None).launched_owner(), Some(Owner::Process(5)));
        assert_eq!(launched(None, None, None).launched_owner(), None);
        assert_eq!(ShellMessage::Exited { app_id: 1 }.launched_owner(), None);
    }
}
