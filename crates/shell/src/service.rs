//! The shell's service for apps (Story 8.7, names in `consoleui_core::service`): apps in a sandbox
//! get the look of the active profile from here and can show toasts. The first shell owns the name.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use consoleui_core::notification::Incoming;
use consoleui_core::service::{NAME, PATH, color_to_hex};
use zbus::blocking::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};

/// What apps can read; the shell keeps it current.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Published {
    pub accent: [u8; 3],
    pub animations: bool,
    pub sounds: bool,
    pub profile: String,
}

/// What apps ask the shell to do.
#[derive(Debug)]
pub enum AppRequest {
    /// Apps were installed or removed
    RefreshLibrary,
    /// Start this entry of the library
    Launch(String),
}

struct Server {
    published: Arc<Mutex<Published>>,
    toasts: Sender<Incoming>,
    requests: Sender<AppRequest>,
}

impl Server {
    fn read<T>(&self, get: impl FnOnce(&Published) -> T) -> T {
        get(&self.published.lock().unwrap())
    }
}

#[zbus::interface(name = "org.consoleui.Shell1")]
impl Server {
    #[zbus(property)]
    fn accent_color(&self) -> String {
        self.read(|p| color_to_hex(p.accent))
    }

    #[zbus(property)]
    fn animations(&self) -> bool {
        self.read(|p| p.animations)
    }

    #[zbus(property)]
    fn sounds(&self) -> bool {
        self.read(|p| p.sounds)
    }

    #[zbus(property)]
    fn profile_name(&self) -> String {
        self.read(|p| p.profile.clone())
    }

    fn toast(&self, title: String, text: String) {
        if let Some(message) = Incoming::new("", &title, &text) {
            let _ = self.toasts.send(message);
        }
    }

    fn refresh_library(&self) {
        let _ = self.requests.send(AppRequest::RefreshLibrary);
    }

    fn launch(&self, entry: String) {
        let _ = self.requests.send(AppRequest::Launch(entry));
    }
}

/// The running service, seen from the shell.
pub struct Service {
    published: Arc<Mutex<Published>>,
    changes: Sender<()>,
    /// Toasts that apps asked for
    pub toasts: Receiver<Incoming>,
    pub requests: Receiver<AppRequest>,
}

impl Service {
    /// Keeps what apps read current and tells them what changed.
    pub fn publish(&self, now: Published) {
        let mut published = self.published.lock().unwrap();
        if *published != now {
            *published = now;
            let _ = self.changes.send(());
        }
    }
}

/// Starts the service on a thread of its own.
pub fn start(values: Published) -> Service {
    let published = Arc::new(Mutex::new(values));
    let (toasts, toasts_rx) = mpsc::channel();
    let (requests, requests_rx) = mpsc::channel();
    let (changes, changes_rx) = mpsc::channel();
    let shared = published.clone();
    std::thread::spawn(move || {
        // The connection serves requests in the background as long as it lives.
        let server = Server { published: shared.clone(), toasts, requests };
        let served = serve(server).and_then(|connection| announce(&connection, &shared, changes_rx));
        if let Err(err) = served {
            eprintln!("No service for apps: {err}");
        }
    });
    Service { published, changes, toasts: toasts_rx, requests: requests_rx }
}

/// Sends `PropertiesChanged` for what differs after each change, until the shell ends.
fn announce(connection: &Connection, published: &Mutex<Published>, changes: Receiver<()>) -> Result<(), String> {
    let interface = connection.object_server().interface::<_, Server>(PATH).map_err(|err| err.to_string())?;
    let mut last = published.lock().unwrap().clone();
    for () in changes {
        let now = published.lock().unwrap().clone();
        let server = interface.get();
        let emitter = interface.signal_emitter();
        let sent = zbus::block_on(async {
            if now.accent != last.accent {
                server.accent_color_changed(emitter).await?;
            }
            if now.animations != last.animations {
                server.animations_changed(emitter).await?;
            }
            if now.sounds != last.sounds {
                server.sounds_changed(emitter).await?;
            }
            if now.profile != last.profile {
                server.profile_name_changed(emitter).await?;
            }
            zbus::Result::Ok(())
        });
        if let Err(err) = sent {
            eprintln!("Cannot tell the apps about a change: {err}");
        }
        last = now;
    }
    Ok(())
}

fn serve(server: Server) -> Result<Connection, String> {
    let err = |e: zbus::Error| e.to_string();
    let connection = Connection::session().map_err(err)?;
    connection.object_server().at(PATH, server).map_err(err)?;
    match connection.request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into()).map_err(err)? {
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => Ok(connection),
        _ => Err(format!("{NAME} belongs to another shell")),
    }
}
