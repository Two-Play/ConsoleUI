//! Control of gamescope in Steam mode (`-e`) via X11 atoms, see docs/spike-gamescope.md:
//! windows get an app ID via `STEAM_GAME`, `GAMESCOPECTRL_BASELAYER_APPID` decides who is in
//! front, the quick menu is a window with `STEAM_OVERLAY`, and toasts over games use a narrower
//! one without `STEAM_INPUT_FOCUS` (notification window).

use std::collections::HashMap;
use std::error::Error;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, InputFocus, MapState, PropMode, Window, WindowClass};
use x11rb::CURRENT_TIME;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// `_NET_WM_WINDOW_OPACITY` for fully opaque
const OPAQUE: u32 = u32::MAX;

struct Atoms {
    steam_game: u32,
    steam_overlay: u32,
    steam_input_focus: u32,
    opacity: u32,
    wm_pid: u32,
    baselayer: u32,
}

pub struct Gamescope {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
    /// Windows with a known app ID (set by us or by Steam itself)
    known: HashMap<Window, u32>,
    /// Own windows that get no app ID (overlay and notification window)
    ignored: Vec<Window>,
}

impl Gamescope {
    /// Only active in the console session; `session/consoleui-session` sets `CONSOLEUI_GAMESCOPE=steam`.
    pub fn connect() -> Option<Self> {
        if std::env::var("CONSOLEUI_GAMESCOPE").as_deref() != Ok("steam") {
            return None;
        }
        Self::try_connect().map_err(|err| eprintln!("gamescope control not available: {err}")).ok()
    }

    fn try_connect() -> Result<Self, Box<dyn Error>> {
        let (conn, screen) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen].root;
        let intern = |name: &[u8]| -> Result<u32, Box<dyn Error>> { Ok(conn.intern_atom(false, name)?.reply()?.atom) };
        let atoms = Atoms {
            steam_game: intern(b"STEAM_GAME")?,
            steam_overlay: intern(b"STEAM_OVERLAY")?,
            steam_input_focus: intern(b"STEAM_INPUT_FOCUS")?,
            opacity: intern(b"_NET_WM_WINDOW_OPACITY")?,
            wm_pid: intern(b"_NET_WM_PID")?,
            baselayer: intern(b"GAMESCOPECTRL_BASELAYER_APPID")?,
        };
        Ok(Self { conn, root, atoms, known: HashMap::new(), ignored: Vec::new() })
    }

    /// Order of the app IDs gamescope should show; the first one present is in front.
    pub fn set_baselayer(&self, app_ids: &[u32]) {
        report(self.change_cardinal(self.root, self.atoms.baselayer, app_ids));
    }

    /// Sets the keyboard focus to a window. gamescope does not hand it back to the shell by itself
    /// after the overlay closes, although the shell is in front.
    pub fn focus(&self, window: Window) {
        report((|| {
            self.conn.set_input_focus(InputFocus::POINTER_ROOT, window, CURRENT_TIME)?;
            self.conn.flush()?;
            Ok(())
        })());
    }

    /// Gives the keyboard focus back to the app unless it is in one of its windows (or their
    /// subwindows). After the overlay closes it otherwise ends up on an internal gamescope window,
    /// and the game gets no more keyboard input.
    pub fn focus_app(&self, app_id: u32) {
        let windows: Vec<Window> =
            self.known.iter().filter(|&(_, &id)| id == app_id).map(|(&window, _)| window).collect();
        let Some(&first) = windows.first() else { return };
        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut current = self.conn.get_input_focus()?.reply()?.focus;
            // Walk up from the focused window to the root: if one of the app's windows is on the way,
            // everything is fine (games often focus a subwindow).
            while current != self.root && current > 1 {
                if windows.contains(&current) {
                    return Ok(());
                }
                current = self.conn.query_tree(current)?.reply()?.parent;
            }
            self.conn.set_input_focus(InputFocus::POINTER_ROOT, first, CURRENT_TIME)?;
            self.conn.flush()?;
            Ok(())
        })();
        report(result);
    }

    /// Prepares a window as an overlay. gamescope reads the atoms when mapping, so the window must
    /// be (re)shown afterwards.
    pub fn prepare_overlay(&mut self, window: Window) {
        self.ignored.push(window);
        report((|| {
            self.change_cardinal(window, self.atoms.steam_overlay, &[1])?;
            // 1: keyboard and mouse go to the overlay while it is open.
            self.change_cardinal(window, self.atoms.steam_input_focus, &[1])?;
            self.change_cardinal(window, self.atoms.opacity, &[OPAQUE])
        })());
    }

    /// Prepares a window as notification window (toasts over games): `STEAM_OVERLAY` without
    /// `STEAM_INPUT_FOCUS`. If it is also narrower than the root window, gamescope picks it not as
    /// an interactive overlay but as `notificationWindow`: always on top, never focused. As with
    /// the overlay, (re)map it afterwards.
    pub fn prepare_notification(&mut self, window: Window) {
        self.ignored.push(window);
        report((|| {
            self.change_cardinal(window, self.atoms.steam_overlay, &[1])?;
            self.conn.delete_property(window, self.atoms.steam_input_focus)?;
            self.change_cardinal(window, self.atoms.opacity, &[OPAQUE])
        })());
    }

    /// Size of the root window (in gamescope the game resolution)
    pub fn root_size(&self) -> Option<(u32, u32)> {
        let geometry = self.conn.get_geometry(self.root).ok()?.reply().ok()?;
        Some((geometry.width.into(), geometry.height.into()))
    }

    /// Gives new, visible windows an app ID. `resolve` maps a window's PID to an app; windows
    /// without a match get `fallback` (the app being launched) or are retried on the next pass.
    /// Returns the app IDs of the newly tagged windows.
    pub fn tag_windows(&mut self, resolve: impl Fn(u32) -> Option<u32>, fallback: Option<u32>) -> Vec<u32> {
        match self.try_tag_windows(resolve, fallback) {
            Ok(tagged) => tagged,
            Err(err) => {
                eprintln!("gamescope: cannot assign windows: {err}");
                Vec::new()
            }
        }
    }

    fn try_tag_windows(
        &mut self,
        resolve: impl Fn(u32) -> Option<u32>,
        fallback: Option<u32>,
    ) -> Result<Vec<u32>, Box<dyn Error>> {
        let children = self.conn.query_tree(self.root)?.reply()?.children;
        self.known.retain(|window, _| children.contains(window));
        let mut tagged = Vec::new();
        for window in children {
            if self.known.contains_key(&window) || self.ignored.contains(&window) {
                continue;
            }
            let Ok(attributes) = self.conn.get_window_attributes(window)?.reply() else { continue };
            if attributes.map_state != MapState::VIEWABLE || attributes.class != WindowClass::INPUT_OUTPUT {
                continue;
            }
            if let Some(existing) = self.cardinal(window, self.atoms.steam_game)? {
                self.known.insert(window, existing);
                continue;
            }
            let pid = self.cardinal(window, self.atoms.wm_pid)?;
            if let Some(app_id) = pid.and_then(&resolve).or(fallback) {
                self.change_cardinal(window, self.atoms.steam_game, &[app_id])?;
                self.known.insert(window, app_id);
                tagged.push(app_id);
            }
        }
        self.conn.flush()?;
        Ok(tagged)
    }

    fn cardinal(&self, window: Window, property: u32) -> Result<Option<u32>, Box<dyn Error>> {
        let reply = self.conn.get_property(false, window, property, AtomEnum::CARDINAL, 0, 1)?.reply()?;
        Ok(reply.value32().and_then(|mut values| values.next()))
    }

    fn change_cardinal(&self, window: Window, property: u32, values: &[u32]) -> Result<(), Box<dyn Error>> {
        self.conn.change_property32(PropMode::REPLACE, window, property, AtomEnum::CARDINAL, values)?;
        self.conn.flush()?;
        Ok(())
    }
}

fn report(result: Result<(), Box<dyn Error>>) {
    if let Err(err) = result {
        eprintln!("gamescope: {err}");
    }
}

/// X11 window ID of a Slint window (exists only after it was first shown).
pub fn x11_window_id(window: &slint::Window) -> Option<Window> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = window.window_handle();
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => Some(h.window as Window),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}
