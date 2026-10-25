//! Control protocol between the shell and its own compositor (Epic 7, `crates/compositor`).
//!
//! The compositor starts the shell with `CONSOLEUI_COMPOSITOR=<path>`; the shell connects to that
//! Unix socket and both sides exchange line-delimited JSON, one message per line. This replaces
//! the gamescope tricks (`STEAM_GAME` atoms, `GAMESCOPECTRL_BASELAYER_APPID`, overlay windows):
//! the shell says which app it launched and what should be in front, the compositor maps windows
//! to those apps (with [`crate::ownership`]) and reports what it sees.
//!
//! Unknown message types or malformed lines are skipped by the reader, so either side can add
//! messages later (7.7 transitions, 7.12 screenshots) without breaking an older peer. Fields that
//! may be added later must be `Option` or `#[serde(default)]`. Optional features are announced in
//! [`CompositorMessage::Welcome`] (`features`), so a newer shell keeps working with an older
//! compositor (Story 7.6: overlay toplevels only when the compositor lists [`FEATURE_OVERLAYS`]).

use serde::{Deserialize, Serialize};

use crate::ownership::Owner;

/// Environment variable that holds the control socket path for the shell.
pub const SOCKET_ENV: &str = "CONSOLEUI_COMPOSITOR";
/// Bumped on incompatible changes; both sides log a mismatch but keep going.
pub const VERSION: u32 = 1;

/// Feature (Story 7.6): the compositor places shell toplevels announced with
/// [`ShellMessage::WindowRole`] above the app in front.
pub const FEATURE_OVERLAYS: &str = "overlay_windows";
/// Feature (Story 7.5): the compositor reserves a Guide key combination for the shell and reports
/// it as [`CompositorMessage::Guide`], whatever window has the keyboard.
pub const FEATURE_GUIDE_KEY: &str = "guide_key";

/// What a separate shell toplevel is for (Story 7.6). The shell's main window has no role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowRole {
    /// The quick menu: stacked above the app in front, gets keyboard and pointer while
    /// [`ShellMessage::Overlay`] is open; the app stays visible underneath.
    Overlay,
    /// Toasts: always on top, never focused, no pointer input.
    Notification,
}

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
    /// The quick menu is open: the shell gets keyboard and pointer, the app in front gets no input
    /// at all (Story 7.5). With a [`WindowRole::Overlay`] window only that window is placed above
    /// the app (Story 7.6); without one the whole shell window comes to the front.
    Overlay { open: bool },
    /// Gives the shell's toplevel with this title a role (Story 7.6). Only windows of the shell
    /// process are matched; the title may also be set after the window is mapped.
    WindowRole { title: String, role: WindowRole },
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
    /// Answer to [`ShellMessage::Hello`], with the optional features this compositor supports
    /// ([`FEATURE_OVERLAYS`], [`FEATURE_GUIDE_KEY`]).
    Welcome {
        version: u32,
        #[serde(default)]
        features: Vec<String>,
    },
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
    /// The Guide key combination was pressed (Story 7.5); the focused window does not get it.
    Guide,
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
            encode(&ShellMessage::WindowRole { title: "T".into(), role: WindowRole::Notification }),
            "{\"type\":\"WindowRole\",\"title\":\"T\",\"role\":\"notification\"}\n"
        );
        assert_eq!(encode(&CompositorMessage::Guide), "{\"type\":\"Guide\"}\n");
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
            ShellMessage::WindowRole { title: "ConsoleUI Overlay".into(), role: WindowRole::Overlay },
            ShellMessage::WindowRole { title: "ConsoleUI notification".into(), role: WindowRole::Notification },
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
            CompositorMessage::Welcome { version: VERSION, features: vec![FEATURE_OVERLAYS.into()] },
            CompositorMessage::Guide,
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
    fn older_peers_stay_compatible() {
        let mut reader = LineReader::default();
        // An older compositor sends no features.
        reader.push(b"{\"type\":\"Welcome\",\"version\":1}\n");
        assert_eq!(
            reader.next_message::<CompositorMessage>(),
            Some(CompositorMessage::Welcome { version: 1, features: Vec::new() })
        );
        // Unknown roles (a newer shell) are skipped like unknown messages.
        reader.push(b"{\"type\":\"WindowRole\",\"title\":\"x\",\"role\":\"from_the_future\"}\n");
        assert_eq!(reader.next_message::<ShellMessage>(), None);
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
