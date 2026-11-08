//! My Plugin: a plugin for ConsoleUI. The contract is `docs/plugins.md` in the ConsoleUI repository:
//! the shell starts this program once the user turned it on, then asks `Entries()` for library
//! entries and `Panel()` for the quick menu panel, and calls `Activate` when the user changes
//! something. Remove the half you do not need (and its word in `X-ConsoleUI-Plugin-Provides`).
//!
//! `cargo run` serves it on your session bus; with a ConsoleUI shell running, turn it on under
//! Settings → Plugins after installing the desktop entry (see README.md).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use zbus::blocking::connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

/// Must match `X-ConsoleUI-Plugin` in the desktop entry
const BUS_NAME: &str = "@APP_ID@";
const PATH: &str = "/org/consoleui/Plugin1";

/// A dictionary of the contract (`a{sv}`)
type Dict = HashMap<String, OwnedValue>;

fn dict<'a>(pairs: impl IntoIterator<Item = (&'static str, Value<'a>)>) -> Dict {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.try_to_owned().expect("no file descriptors here")))
        .collect()
}

/// What the panel shows; a real plugin reads and changes the system here.
struct State {
    enabled: bool,
    level: f64,
    mode: String,
    presses: u32,
}

struct Plugin {
    state: Mutex<State>,
}

#[zbus::interface(name = "org.consoleui.Plugin1")]
impl Plugin {
    /// The protocol version this plugin speaks
    #[zbus(property)]
    fn version(&self) -> u32 {
        1
    }

    /// Library entries. `command` runs on the user's machine when the entry is opened; artwork is
    /// an absolute path the host can read, or an icon name (with Flatpak: one that starts with the
    /// app ID, exported by Flatpak).
    fn entries(&self) -> Vec<Dict> {
        vec![dict([
            ("id", Value::from("hello")),
            ("title", Value::from("Hello from My Plugin")),
            ("kind", Value::from("app")),
            ("command", Value::from(vec!["xdg-open", "https://example.org"])),
        ])]
    }

    /// The quick menu panel: a title and its items.
    fn panel(&self) -> (String, Vec<Dict>) {
        let state = self.state.lock().unwrap();
        let items = vec![
            dict([
                ("id", Value::from("enabled")),
                ("kind", Value::from("toggle")),
                ("label", Value::from("Enabled")),
                ("detail", Value::from("A switch")),
                ("value", Value::from(state.enabled)),
            ]),
            dict([
                ("id", Value::from("level")),
                ("kind", Value::from("slider")),
                ("label", Value::from("Level")),
                ("value", Value::from(state.level)),
                ("min", Value::from(0.0)),
                ("max", Value::from(100.0)),
                ("step", Value::from(10.0)),
                ("unit", Value::from(" %")),
            ]),
            dict([
                ("id", Value::from("mode")),
                ("kind", Value::from("choice")),
                ("label", Value::from("Mode")),
                ("value", Value::from(state.mode.clone())),
                ("choices", Value::from(vec!["Quiet", "Normal", "Loud"])),
            ]),
            dict([
                ("id", Value::from("press")),
                ("kind", Value::from("button")),
                ("label", Value::from("Press me")),
            ]),
            dict([
                ("id", Value::from("count")),
                ("kind", Value::from("info")),
                ("label", Value::from("Pressed")),
                ("value", Value::from(format!("{} times", state.presses))),
            ]),
        ];
        ("My Plugin".to_string(), items)
    }

    /// The user changed item `id`; the shell already shows the new value.
    fn activate(&self, id: String, value: OwnedValue, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let mut state = self.state.lock().unwrap();
        match id.as_str() {
            "enabled" => state.enabled = bool::try_from(value).unwrap_or(state.enabled),
            "level" => state.level = f64::try_from(value).unwrap_or(state.level),
            "mode" => state.mode = String::try_from(value).unwrap_or_default(),
            "press" => {
                state.presses += 1;
                drop(state);
                // Something else on the panel changed: the shell asks for it again
                let _ = zbus::block_on(Self::panel_changed(&emitter));
            }
            _ => {}
        }
    }

    /// The user turned the plugin off: answer, then end.
    fn quit(&self) {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            std::process::exit(0);
        });
    }

    #[zbus(signal)]
    async fn entries_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn panel_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

fn main() -> zbus::Result<()> {
    let state = State { enabled: true, level: 50.0, mode: "Normal".into(), presses: 0 };
    let connection =
        connection::Builder::session()?.name(BUS_NAME)?.serve_at(PATH, Plugin { state: Mutex::new(state) })?.build()?;
    // Ends with the session: when the bus is gone, nothing needs this process any more
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let bus = zbus::blocking::fdo::DBusProxy::new(&connection);
        if !bus.is_ok_and(|bus| bus.get_id().is_ok()) {
            return Ok(());
        }
    }
}
