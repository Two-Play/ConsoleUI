//! The power profile in the ConsoleUI quick menu (Epic 9): power saver, balanced or performance,
//! through `org.freedesktop.UPower.PowerProfiles` (power-profiles-daemon or tuned-ppd).
//! `--fake`: a profile in memory, the system stays as it is.

use std::collections::HashMap;
use std::sync::Mutex;

use consoleui_plugins::{Dict, PATH, dict, fake, text};
use zbus::blocking::Connection;
use zbus::blocking::connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const NAME: &str = "org.consoleui.plugins.Power";
const SERVICE: &str = "org.freedesktop.UPower.PowerProfiles";
const SERVICE_PATH: &str = "/org/freedesktop/UPower/PowerProfiles";

/// The daemon's profile names, as the user reads them
fn label(profile: &str) -> String {
    match profile {
        "power-saver" => text("Power saver", "Stromsparen").into(),
        "balanced" => text("Balanced", "Ausgeglichen").into(),
        "performance" => text("Performance", "Leistung").into(),
        other => other.into(),
    }
}

trait Profiles: Send + Sync {
    fn list(&self) -> Vec<String>;
    fn active(&self) -> Option<String>;
    fn set(&self, profile: &str) -> Result<(), String>;
}

struct System(Connection);

impl System {
    fn proxy(&self) -> zbus::Result<zbus::blocking::Proxy<'_>> {
        zbus::blocking::Proxy::new(&self.0, SERVICE, SERVICE_PATH, SERVICE)
    }
}

impl Profiles for System {
    fn list(&self) -> Vec<String> {
        let Ok(proxy) = self.proxy() else { return Vec::new() };
        let profiles: Vec<HashMap<String, OwnedValue>> = proxy.get_property("Profiles").unwrap_or_default();
        profiles.iter().filter_map(|profile| String::try_from(profile.get("Profile")?.clone()).ok()).collect()
    }

    fn active(&self) -> Option<String> {
        self.proxy().ok()?.get_property("ActiveProfile").ok()
    }

    fn set(&self, profile: &str) -> Result<(), String> {
        let proxy = self.proxy().map_err(|err| err.to_string())?;
        proxy.set_property("ActiveProfile", profile).map_err(|err| err.to_string())
    }
}

struct Fake(Mutex<String>);

impl Profiles for Fake {
    fn list(&self) -> Vec<String> {
        ["power-saver", "balanced", "performance"].map(String::from).to_vec()
    }

    fn active(&self) -> Option<String> {
        Some(self.0.lock().unwrap().clone())
    }

    fn set(&self, profile: &str) -> Result<(), String> {
        println!("set {profile}");
        *self.0.lock().unwrap() = profile.to_string();
        Ok(())
    }
}

struct Plugin {
    profiles: Box<dyn Profiles>,
    /// The last thing that went wrong, shown in the panel
    error: Mutex<Option<String>>,
}

#[zbus::interface(name = "org.consoleui.Plugin1")]
impl Plugin {
    #[zbus(property)]
    fn version(&self) -> u32 {
        1
    }

    fn panel(&self) -> (String, Vec<Dict>) {
        let list = self.profiles.list();
        let title = text("Power", "Energie").to_string();
        if list.is_empty() {
            let info = dict([
                ("id", Value::from("missing")),
                ("kind", Value::from("info")),
                ("label", Value::from(text("No power profiles", "Keine Energieprofile"))),
                (
                    "value",
                    Value::from(text("power-profiles-daemon is not running", "power-profiles-daemon läuft nicht")),
                ),
            ]);
            return (title, vec![info]);
        }
        let active = self.profiles.active().unwrap_or_default();
        let mut items = vec![dict([
            ("id", Value::from("profile")),
            ("kind", Value::from("choice")),
            ("label", Value::from(text("Power profile", "Energieprofil"))),
            ("detail", Value::from(text("Battery life or speed", "Akkulaufzeit oder Leistung"))),
            ("value", Value::from(label(&active))),
            ("choices", Value::from(list.iter().map(|p| label(p)).collect::<Vec<_>>())),
        ])];
        if let Some(error) = self.error.lock().unwrap().clone() {
            items.push(dict([
                ("id", Value::from("error")),
                ("kind", Value::from("info")),
                ("label", Value::from(text("Could not switch", "Umschalten ging nicht"))),
                ("value", Value::from(error)),
            ]));
        }
        (title, items)
    }

    fn activate(&self, id: String, value: OwnedValue, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let Ok(chosen) = String::try_from(value) else { return };
        if id != "profile" {
            return;
        }
        // The panel shows the names, the daemon wants its own
        let Some(profile) = self.profiles.list().into_iter().find(|p| label(p) == chosen) else { return };
        let result = self.profiles.set(&profile);
        let changed = {
            let mut error = self.error.lock().unwrap();
            let before = error.clone();
            *error = result.err();
            before != *error
        };
        if changed {
            let _ = zbus::block_on(Self::panel_changed(&emitter));
        }
    }

    /// Answers first, then exits
    fn quit(&self) {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            std::process::exit(0);
        });
    }

    #[zbus(signal)]
    async fn panel_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

fn main() -> zbus::Result<()> {
    let profiles: Box<dyn Profiles> =
        if fake() { Box::new(Fake(Mutex::new("balanced".into()))) } else { Box::new(System(Connection::system()?)) };
    let connection = connection::Builder::session()?
        .name(NAME)?
        .serve_at(PATH, Plugin { profiles, error: Mutex::new(None) })?
        .build()?;
    consoleui_plugins::serve(&connection, std::time::Duration::from_secs(5), || {});
    Ok(())
}
