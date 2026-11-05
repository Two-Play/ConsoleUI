//! The shell's service for apps (Story 8.7, names in `consoleui_core::service`): apps in a sandbox
//! get the look of the active profile from here and can show toasts. The first shell owns the name.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use consoleui_core::notification::Incoming;
use consoleui_core::service::{NAME, PATH, color_to_hex};
use zbus::blocking::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};

/// What apps can read; the shell keeps it current.
#[derive(Debug, Clone, Default)]
pub struct Published {
    pub accent: [u8; 3],
    pub animations: bool,
    pub sounds: bool,
    pub profile: String,
}

struct Server {
    published: Arc<Mutex<Published>>,
    toasts: Sender<Incoming>,
}

impl Server {
    fn read<T>(&self, get: impl FnOnce(&Published) -> T) -> T {
        get(&self.published.lock().unwrap())
    }
}

#[zbus::interface(name = "org.consoleui.Shell1")]
impl Server {
    #[zbus(property(emits_changed_signal = "false"))]
    fn accent_color(&self) -> String {
        self.read(|p| color_to_hex(p.accent))
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn animations(&self) -> bool {
        self.read(|p| p.animations)
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn sounds(&self) -> bool {
        self.read(|p| p.sounds)
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn profile_name(&self) -> String {
        self.read(|p| p.profile.clone())
    }

    fn toast(&self, title: String, text: String) {
        if let Some(message) = Incoming::new("", &title, &text) {
            let _ = self.toasts.send(message);
        }
    }
}

/// Starts the service on a thread of its own. Toasts arrive on the returned channel.
pub fn start(published: Arc<Mutex<Published>>) -> Receiver<Incoming> {
    let (toasts, receiver) = mpsc::channel();
    std::thread::spawn(move || match serve(published, toasts) {
        // The connection serves requests in the background as long as it lives.
        Ok(_connection) => loop {
            std::thread::sleep(Duration::from_secs(3600));
        },
        Err(err) => eprintln!("No service for apps: {err}"),
    });
    receiver
}

fn serve(published: Arc<Mutex<Published>>, toasts: Sender<Incoming>) -> Result<Connection, String> {
    let err = |e: zbus::Error| e.to_string();
    let connection = Connection::session().map_err(err)?;
    connection.object_server().at(PATH, Server { published, toasts }).map_err(err)?;
    match connection.request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into()).map_err(err)? {
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => Ok(connection),
        _ => Err(format!("{NAME} belongs to another shell")),
    }
}
