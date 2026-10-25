//! Steuerung von gamescope im Steam-Modus (`-e`) über X11-Atome, siehe docs/spike-gamescope.md:
//! Fenster bekommen per `STEAM_GAME` eine App-ID, `GAMESCOPECTRL_BASELAYER_APPID` legt fest, wer
//! vorne ist, und das Quick-Menü ist ein Fenster mit `STEAM_OVERLAY`, Hinweise über Spielen ein
//! schmaleres ohne `STEAM_INPUT_FOCUS` (Benachrichtigungsfenster).

use std::collections::HashMap;
use std::error::Error;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, InputFocus, MapState, PropMode, Window, WindowClass};
use x11rb::CURRENT_TIME;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// `_NET_WM_WINDOW_OPACITY` für voll deckend
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
    /// Fenster mit bekannter App-ID (von uns oder von Steam selbst gesetzt)
    known: HashMap<Window, u32>,
    /// Eigene Fenster, die keine App-ID bekommen (Overlay und Benachrichtigungsfenster)
    ignored: Vec<Window>,
}

impl Gamescope {
    /// Nur in der Konsolen-Session aktiv; `session/consoleui-session` setzt `CONSOLEUI_GAMESCOPE=steam`.
    pub fn connect() -> Option<Self> {
        if std::env::var("CONSOLEUI_GAMESCOPE").as_deref() != Ok("steam") {
            return None;
        }
        Self::try_connect().map_err(|err| eprintln!("gamescope-Steuerung nicht verfügbar: {err}")).ok()
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

    /// Reihenfolge der App-IDs, die gamescope zeigen soll; die erste vorhandene ist vorne.
    pub fn set_baselayer(&self, app_ids: &[u32]) {
        report(self.change_cardinal(self.root, self.atoms.baselayer, app_ids));
    }

    /// Tastaturfokus auf ein Fenster setzen. gamescope gibt ihn nach dem Schließen des Overlays
    /// nicht von selbst an die Shell zurück, obwohl sie vorne ist.
    pub fn focus(&self, window: Window) {
        report((|| {
            self.conn.set_input_focus(InputFocus::POINTER_ROOT, window, CURRENT_TIME)?;
            self.conn.flush()?;
            Ok(())
        })());
    }

    /// Fenster als Overlay vorbereiten. gamescope liest die Atome beim Mappen, deshalb muss das
    /// Fenster danach (neu) angezeigt werden.
    pub fn prepare_overlay(&mut self, window: Window) {
        self.ignored.push(window);
        report((|| {
            self.change_cardinal(window, self.atoms.steam_overlay, &[1])?;
            // 1: Tastatur und Maus gehen an das Overlay, solange es offen ist.
            self.change_cardinal(window, self.atoms.steam_input_focus, &[1])?;
            self.change_cardinal(window, self.atoms.opacity, &[OPAQUE])
        })());
    }

    /// Fenster als Benachrichtigungsfenster vorbereiten (Hinweise über Spielen): `STEAM_OVERLAY`
    /// ohne `STEAM_INPUT_FOCUS`. Ist es zusätzlich schmaler als das Root-Fenster, wählt gamescope
    /// es nicht als interaktives Overlay, sondern als `notificationWindow`: immer obenauf, nie
    /// fokussiert. Wie beim Overlay danach (neu) mappen.
    pub fn prepare_notification(&mut self, window: Window) {
        self.ignored.push(window);
        report((|| {
            self.change_cardinal(window, self.atoms.steam_overlay, &[1])?;
            self.conn.delete_property(window, self.atoms.steam_input_focus)?;
            self.change_cardinal(window, self.atoms.opacity, &[OPAQUE])
        })());
    }

    /// Größe des Root-Fensters (in gamescope die Spielauflösung)
    pub fn root_size(&self) -> Option<(u32, u32)> {
        let geometry = self.conn.get_geometry(self.root).ok()?.reply().ok()?;
        Some((geometry.width.into(), geometry.height.into()))
    }

    /// Gibt neuen, sichtbaren Fenstern eine App-ID. `resolve` ordnet die PID eines Fensters einer
    /// App zu; Fenster ohne Zuordnung bekommen `fallback` (die gerade startende App) oder werden beim
    /// nächsten Durchlauf erneut versucht. Gibt die App-IDs der neu markierten Fenster zurück.
    pub fn tag_windows(&mut self, resolve: impl Fn(u32) -> Option<u32>, fallback: Option<u32>) -> Vec<u32> {
        match self.try_tag_windows(resolve, fallback) {
            Ok(tagged) => tagged,
            Err(err) => {
                eprintln!("gamescope: Fenster konnten nicht zugeordnet werden: {err}");
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

/// X11-Fenster-ID eines Slint-Fensters (erst nach dem ersten Anzeigen vorhanden).
pub fn x11_window_id(window: &slint::Window) -> Option<Window> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = window.window_handle();
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => Some(h.window as Window),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}
