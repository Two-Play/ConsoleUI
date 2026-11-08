//! What the plugins that come with ConsoleUI share: serving `org.consoleui.Plugin1` and building
//! the dictionaries of `docs/plugins.md`. Nothing here depends on the ConsoleUI crates.

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

pub const PATH: &str = "/org/consoleui/Plugin1";

/// A dictionary of the contract (`a{sv}`).
pub type Dict = HashMap<String, OwnedValue>;

pub fn owned<'a>(value: impl Into<Value<'a>>) -> OwnedValue {
    value.into().try_to_owned().expect("no file descriptors in plugin values")
}

/// Builds a dictionary from `(key, value)` pairs.
pub fn dict<'a>(pairs: impl IntoIterator<Item = (&'static str, Value<'a>)>) -> Dict {
    pairs.into_iter().map(|(key, value)| (key.to_string(), owned(value))).collect()
}

/// German when the session is German, else English (the plugins carry their few texts themselves).
pub fn german() -> bool {
    ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
        .is_some_and(|value| value.starts_with("de"))
}

pub fn text(english: &'static str, german_text: &'static str) -> &'static str {
    if german() { german_text } else { english }
}

/// `--fake` on the command line: change nothing on the system.
pub fn fake() -> bool {
    std::env::args().any(|arg| arg == "--fake")
}

/// Keeps the plugin alive while its session bus is there, calling `tick` every `every`; a plugin
/// that outlives its session (the shell was killed, the user logged out) ends here.
pub fn serve(connection: &zbus::blocking::Connection, every: std::time::Duration, mut tick: impl FnMut()) {
    loop {
        std::thread::sleep(every);
        let alive = zbus::blocking::fdo::DBusProxy::new(connection).is_ok_and(|bus| bus.get_id().is_ok());
        if !alive {
            return;
        }
        tick();
    }
}
