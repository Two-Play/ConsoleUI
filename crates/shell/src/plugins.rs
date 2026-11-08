//! Talks to the plugins the user turned on (Epic 9, `docs/plugins.md`) on a thread of its own:
//! starts them, asks for their library entries and panels, passes the user's changes on and
//! listens for their signals. The UI thread only sends [`Request`]s and reads [`Event`]s.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use consoleui_core::launch::{self, ProcessHandle};
use consoleui_core::plugins::{
    INTERFACE, ItemKind, PATH, Panel, PanelItem, PluginArtwork, PluginEntry, PluginInfo, Provides, Value,
};
use consoleui_core::{AppKind, Launch};
use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator, Proxy, fdo::DBusProxy};
use zbus::message::Type as MessageType;
use zbus::zvariant::{OwnedValue, Value as ZValue};

/// How long a plugin gets to own its bus name after it was started
const START_TIMEOUT: Duration = Duration::from_secs(5);

pub enum Request {
    /// These plugins are on (the others are stopped)
    Run(Vec<PluginInfo>),
    Panel(String),
    /// The plugin's own settings page
    Settings(String),
    /// Pictures for these entries (ID, title) from the artwork plugins
    Artwork(Vec<(String, String)>),
    /// Theme music for this entry (ID, title) from the theme plugins
    Theme {
        entry: String,
        title: String,
    },
    Activate {
        plugin: String,
        item: String,
        value: Value,
    },
    /// A signal of a plugin: ask for its entries again
    Entries(String),
    /// The shell quits
    Stop,
}

pub enum Event {
    Entries {
        plugin: String,
        entries: Vec<PluginEntry>,
    },
    Panel {
        plugin: String,
        panel: Panel,
    },
    /// The plugin wants its open panel asked again
    PanelChanged(String),
    Artwork {
        entry: String,
        artwork: PluginArtwork,
    },
    /// `None`: no theme plugin has music for it
    Theme {
        entry: String,
        path: Option<String>,
    },
    Failed {
        plugin: String,
        error: String,
    },
}

pub struct Host {
    requests: Sender<Request>,
    pub events: Receiver<Event>,
    thread: std::thread::JoinHandle<()>,
}

impl Host {
    pub fn start() -> Self {
        let (requests, incoming) = mpsc::channel();
        let (events, received) = mpsc::channel();
        let again = requests.clone();
        let thread = std::thread::spawn(move || {
            let connection = match Connection::session() {
                Ok(connection) => connection,
                Err(err) => {
                    eprintln!("Plugins need the session bus: {err}");
                    return;
                }
            };
            let mut runner = Runner { connection, running: HashMap::new(), events, again };
            // Ends when the shell stops the host (the signal listeners hold `again` until then)
            while let Ok(request) = incoming.recv() {
                if matches!(request, Request::Stop) {
                    break;
                }
                runner.handle(request);
            }
            runner.stop_all();
        });
        Self { requests, events: received, thread }
    }

    /// Stops the plugins this host started and waits for that.
    pub fn stop(self) {
        let _ = self.requests.send(Request::Stop);
        let _ = self.thread.join();
    }

    pub fn send(&self, request: Request) {
        let _ = self.requests.send(request);
    }
}

struct Running {
    info: PluginInfo,
    /// The process we started (a plugin that ran already is not ours to stop)
    process: Option<ProcessHandle>,
}

struct Runner {
    connection: Connection,
    running: HashMap<String, Running>,
    events: Sender<Event>,
    again: Sender<Request>,
}

impl Runner {
    fn handle(&mut self, request: Request) {
        match request {
            Request::Run(plugins) => self.run(plugins),
            Request::Panel(id) => self.panel(&id, "Panel"),
            Request::Settings(id) => self.panel(&id, "Settings"),
            Request::Artwork(entries) => self.artwork(&entries),
            Request::Theme { entry, title } => self.theme(entry, &title),
            Request::Activate { plugin, item, value } => self.activate(&plugin, &item, value),
            Request::Entries(id) => self.entries(&id),
            Request::Stop => {}
        }
    }

    fn fail(&self, plugin: &str, error: impl ToString) {
        let error = error.to_string();
        eprintln!("Plugin {plugin}: {error}");
        let _ = self.events.send(Event::Failed { plugin: plugin.to_string(), error });
    }

    fn proxy(&self, id: &str) -> Option<Proxy<'_>> {
        let running = self.running.get(id)?;
        Proxy::new(&self.connection, running.info.bus_name.clone(), PATH, INTERFACE).ok()
    }

    fn has_owner(&self, name: &str) -> bool {
        let Ok(bus) = DBusProxy::new(&self.connection) else { return false };
        name.try_into().ok().is_some_and(|name| bus.name_has_owner(name).unwrap_or(false))
    }

    fn run(&mut self, plugins: Vec<PluginInfo>) {
        let gone: Vec<String> =
            self.running.keys().filter(|id| !plugins.iter().any(|p| &p.id == *id)).cloned().collect();
        for id in gone {
            self.stop(&id);
        }
        for info in plugins {
            if self.running.contains_key(&info.id) {
                continue;
            }
            let id = info.id.clone();
            let process = if self.has_owner(&info.bus_name) {
                None
            } else {
                match launch::spawn(&Launch::Command(info.exec.clone())) {
                    Ok(process) => Some(process),
                    Err(err) => {
                        self.fail(&id, format!("cannot start {:?}: {err}", info.exec));
                        continue;
                    }
                }
            };
            let start = Instant::now();
            while !self.has_owner(&info.bus_name) && start.elapsed() < START_TIMEOUT {
                std::thread::sleep(Duration::from_millis(50));
            }
            if !self.has_owner(&info.bus_name) {
                if let Some(process) = &process {
                    let _ = process.stop();
                }
                self.fail(&id, format!("did not show up on the bus as {}", info.bus_name));
                continue;
            }
            self.listen(&info);
            let library = info.provides(Provides::Library);
            self.running.insert(id.clone(), Running { info, process });
            if library {
                self.entries(&id);
            }
        }
    }

    /// Forwards the plugin's signals as requests to this thread.
    fn listen(&self, info: &PluginInfo) {
        let rule = MatchRule::builder()
            .msg_type(MessageType::Signal)
            .sender(info.bus_name.as_str())
            .and_then(|rule| rule.interface(INTERFACE))
            .map(|rule| rule.build());
        let Ok(rule) = rule else { return };
        let Ok(messages) = MessageIterator::for_match_rule(rule, &self.connection, None) else { return };
        let (again, events, id) = (self.again.clone(), self.events.clone(), info.id.clone());
        std::thread::spawn(move || {
            for message in messages.flatten() {
                let member = message.header().member().map(|m| m.to_string()).unwrap_or_default();
                let sent = match member.as_str() {
                    "EntriesChanged" => again.send(Request::Entries(id.clone())).is_ok(),
                    "PanelChanged" => events.send(Event::PanelChanged(id.clone())).is_ok(),
                    _ => true,
                };
                if !sent {
                    return;
                }
            }
        });
    }

    fn stop(&mut self, id: &str) {
        if let Some(proxy) = self.proxy(id) {
            let _ = proxy.call_method("Quit", &());
        }
        if let Some(running) = self.running.remove(id)
            && let Some(process) = running.process
        {
            std::thread::sleep(Duration::from_millis(200));
            if !process.has_exited() {
                let _ = process.stop();
            }
        }
        // Its entries leave the library
        let _ = self.events.send(Event::Entries { plugin: id.to_string(), entries: Vec::new() });
    }

    fn stop_all(&mut self) {
        let ids: Vec<String> = self.running.keys().cloned().collect();
        for id in ids {
            self.stop(&id);
        }
    }

    fn entries(&self, id: &str) {
        let Some(proxy) = self.proxy(id) else { return };
        match proxy.call::<_, _, Vec<HashMap<String, OwnedValue>>>("Entries", &()) {
            Ok(list) => {
                let entries = list.iter().filter_map(parse_entry).collect();
                let _ = self.events.send(Event::Entries { plugin: id.to_string(), entries });
            }
            Err(err) => self.fail(id, err),
        }
    }

    /// `method`: `Panel` (quick menu) or `Settings` (its settings page); both look the same.
    fn panel(&self, id: &str, method: &str) {
        let Some(proxy) = self.proxy(id) else { return self.fail(id, "is not running") };
        match proxy.call::<_, _, (String, Vec<HashMap<String, OwnedValue>>)>(method, &()) {
            Ok((title, items)) => {
                let panel = Panel { title, items: items.iter().filter_map(parse_item).collect() };
                let _ = self.events.send(Event::Panel { plugin: id.to_string(), panel });
            }
            Err(err) => self.fail(id, err),
        }
    }

    /// The running plugins that offer `what`.
    fn offering(&self, what: Provides) -> Vec<String> {
        let mut ids: Vec<String> =
            self.running.values().filter(|r| r.info.provides(what)).map(|r| r.info.id.clone()).collect();
        ids.sort();
        ids
    }

    fn artwork(&self, entries: &[(String, String)]) {
        let plugins = self.offering(Provides::Artwork);
        for (entry, title) in entries {
            // The first plugin with something wins
            let found = plugins.iter().find_map(|plugin| {
                let proxy = self.proxy(plugin)?;
                let pictures: HashMap<String, String> =
                    proxy.call("Artwork", &(entry.as_str(), title.as_str())).ok()?;
                let get = |key: &str| pictures.get(key).filter(|v| !v.is_empty()).cloned();
                let artwork = PluginArtwork { cover: get("cover"), hero: get("hero"), logo: get("logo") };
                (!artwork.is_empty()).then_some(artwork)
            });
            if let Some(artwork) = found {
                let _ = self.events.send(Event::Artwork { entry: entry.clone(), artwork });
            }
        }
    }

    fn theme(&self, entry: String, title: &str) {
        let path = self.offering(Provides::Themes).iter().find_map(|plugin| {
            let path: String = self.proxy(plugin)?.call("Theme", &(entry.as_str(), title)).ok()?;
            Some(path).filter(|path| !path.is_empty())
        });
        let _ = self.events.send(Event::Theme { entry, path });
    }

    fn activate(&self, id: &str, item: &str, value: Value) {
        let Some(proxy) = self.proxy(id) else { return };
        let value: ZValue = match value {
            Value::Bool(on) => on.into(),
            Value::Number(number) => number.into(),
            Value::Text(text) => text.into(),
        };
        if let Err(err) = proxy.call_method("Activate", &(item, value)) {
            self.fail(id, err);
        }
    }
}

fn text(dict: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    dict.get(key).and_then(|value| String::try_from(value.clone()).ok())
}

fn number(dict: &HashMap<String, OwnedValue>, key: &str) -> Option<f64> {
    let value = dict.get(key)?;
    f64::try_from(value.clone())
        .ok()
        .or_else(|| i64::try_from(value.clone()).ok().map(|n| n as f64))
        .or_else(|| i32::try_from(value.clone()).ok().map(f64::from))
        .or_else(|| u32::try_from(value.clone()).ok().map(f64::from))
}

fn parse_entry(dict: &HashMap<String, OwnedValue>) -> Option<PluginEntry> {
    Some(PluginEntry {
        id: text(dict, "id")?,
        title: text(dict, "title")?,
        kind: if text(dict, "kind").as_deref() == Some("app") { AppKind::App } else { AppKind::Game },
        command: dict.get("command").and_then(|value| Vec::<String>::try_from(value.clone()).ok()).unwrap_or_default(),
        cover: text(dict, "cover"),
        hero: text(dict, "hero"),
        logo: text(dict, "logo"),
        icon: text(dict, "icon"),
        last_played: number(dict, "last_played").filter(|t| *t > 0.0).map(|t| t as u64),
    })
}

fn parse_item(dict: &HashMap<String, OwnedValue>) -> Option<PanelItem> {
    let id = text(dict, "id")?;
    let label = text(dict, "label").unwrap_or_default();
    let detail = text(dict, "detail").unwrap_or_default();
    let value = dict.get("value");
    let bool_value = || value.and_then(|v| bool::try_from(v.clone()).ok()).unwrap_or(false);
    let text_value = || value.and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default();
    let (kind, value) = match text(dict, "kind")?.as_str() {
        "toggle" => (ItemKind::Toggle, Value::Bool(bool_value())),
        "slider" => {
            let min = number(dict, "min").unwrap_or(0.0);
            let max = number(dict, "max").unwrap_or(100.0);
            let step = number(dict, "step").unwrap_or(1.0);
            let unit = text(dict, "unit").unwrap_or_default();
            (ItemKind::Slider { min, max, step, unit }, Value::Number(number(dict, "value").unwrap_or(min)))
        }
        "choice" => {
            let choices =
                dict.get("choices").and_then(|value| Vec::<String>::try_from(value.clone()).ok()).unwrap_or_default();
            (ItemKind::Choice(choices), Value::Text(text_value()))
        }
        "button" => (ItemKind::Button, Value::Bool(false)),
        "info" => (ItemKind::Info, Value::Text(text_value())),
        // A kind from a newer protocol: left out
        _ => return None,
    };
    Some(PanelItem { id, label, detail, kind, value })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value as V;

    fn dict(pairs: &[(&str, V<'static>)]) -> HashMap<String, OwnedValue> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap())).collect()
    }

    #[test]
    fn reads_entries_and_items_of_the_contract() {
        let entry = parse_entry(&dict(&[
            ("id", V::from("1")),
            ("title", V::from("EA App")),
            ("command", V::from(vec!["lutris", "lutris:rungameid/1"])),
            ("last_played", V::from(1_700_000_000i64)),
            ("future_key", V::from(true)),
        ]))
        .unwrap();
        assert_eq!(entry.command, ["lutris", "lutris:rungameid/1"]);
        assert_eq!(entry.last_played, Some(1_700_000_000));
        assert_eq!(entry.kind, AppKind::Game);
        let slider = parse_item(&dict(&[
            ("id", V::from("tdp")),
            ("kind", V::from("slider")),
            ("value", V::from(15.0f64)),
            ("max", V::from(30i32)),
            ("unit", V::from(" W")),
        ]))
        .unwrap();
        assert_eq!(slider.kind, ItemKind::Slider { min: 0.0, max: 30.0, step: 1.0, unit: " W".into() });
        assert_eq!(slider.value, Value::Number(15.0));
        assert!(parse_item(&dict(&[("id", V::from("x")), ("kind", V::from("hologram"))])).is_none());
    }
}
