//! System notifications: the shell answers `org.freedesktop.Notifications` on the session bus, so
//! `notify-send` and every program that sends messages shows them as toasts (also over a running
//! game). A desktop session has its own notification service, which keeps the name; then this
//! server is not started and the shell does nothing here.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use consoleui_core::notification::Incoming;
use zbus::blocking::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::zvariant::OwnedValue;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

struct Server {
    sender: Sender<Incoming>,
    next_id: AtomicU32,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<String> {
        vec!["body".into()]
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        _actions: Vec<String>,
        _hints: HashMap<String, OwnedValue>,
        _expire_timeout: i32,
    ) -> u32 {
        if let Some(message) = Incoming::new(&app_name, &summary, &body) {
            let _ = self.sender.send(message);
        }
        if replaces_id != 0 { replaces_id } else { self.next_id.fetch_add(1, Ordering::Relaxed) }
    }

    fn close_notification(&self, _id: u32) {}

    fn get_server_information(&self) -> (String, String, String, String) {
        ("ConsoleUI".into(), "ConsoleUI".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }
}

/// Starts the server on a thread of its own; the messages arrive on the returned channel. Nothing
/// arrives if there is no session bus or another service owns the name.
pub fn start() -> Receiver<Incoming> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || match serve(sender) {
        // The connection serves requests in the background as long as it lives.
        Ok(_connection) => loop {
            std::thread::sleep(Duration::from_secs(3600));
        },
        Err(err) => eprintln!("System notifications are not shown by the shell: {err}"),
    });
    receiver
}

fn serve(sender: Sender<Incoming>) -> Result<Connection, String> {
    let err = |e: zbus::Error| e.to_string();
    let connection = Connection::session().map_err(err)?;
    connection.object_server().at(PATH, Server { sender, next_id: AtomicU32::new(1) }).map_err(err)?;
    match connection.request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into()).map_err(err)? {
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => Ok(connection),
        _ => Err(format!("{NAME} belongs to another service")),
    }
}
