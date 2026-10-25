//! Wi-Fi and Bluetooth (Stories 5.3/5.4): requests to a backend that runs on its own thread, and
//! the responses back to the UI. The real backend (NetworkManager and BlueZ over D-Bus) lives in
//! the shell, the simulated one in [`crate::fakenet`].

use std::sync::mpsc::{self, Receiver, Sender};

use crate::bluetooth::BluetoothState;
use crate::i18n::{tr, trf};
use crate::wifi::WifiState;

/// A request to the backend. Everything except `Refresh` changes the system state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Read the Wi-Fi and Bluetooth state again
    Refresh,
    WifiEnable(bool),
    WifiScan,
    /// Without a password: open network or saved profile
    WifiConnect { ssid: String, password: Option<String> },
    WifiDisconnect { ssid: String },
    BtPower(bool),
    BtDiscovery(bool),
    /// Pair, trust and connect
    BtPair { address: String, name: String },
    BtConnect { address: String, name: String },
    BtDisconnect { address: String, name: String },
    BtRemove { address: String, name: String },
}

/// Notice for the UI (shown as a toast)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub symbol: &'static str,
    pub title: String,
    pub text: String,
}

impl Notice {
    fn new(symbol: &'static str, title: impl Into<String>, text: impl Into<String>) -> Self {
        Self { symbol, title: title.into(), text: text.into() }
    }
}

impl Request {
    /// SSID or device address where the page shows "in progress …"
    pub fn target(&self) -> Option<&str> {
        match self {
            Self::WifiConnect { ssid, .. } | Self::WifiDisconnect { ssid } => Some(ssid),
            Self::BtPair { address, .. }
            | Self::BtConnect { address, .. }
            | Self::BtDisconnect { address, .. }
            | Self::BtRemove { address, .. } => Some(address),
            _ => None,
        }
    }

    /// Text in the row while the request runs
    pub fn busy_label(&self) -> &'static str {
        match self {
            Self::WifiConnect { .. } | Self::BtConnect { .. } => tr("Connecting …"),
            Self::WifiDisconnect { .. } | Self::BtDisconnect { .. } => tr("Disconnecting …"),
            Self::BtPair { .. } => tr("Pairing …"),
            Self::BtRemove { .. } => tr("Removing …"),
            Self::WifiScan | Self::BtDiscovery(true) => tr("Searching …"),
            _ => tr("Please wait …"),
        }
    }

    /// Notice after the request; successes without a visible effect stay silent.
    pub fn notice(&self, result: &Result<(), String>) -> Option<Notice> {
        let ok = result.is_ok();
        let err = result.as_ref().err().map(String::as_str).unwrap_or_default();
        Some(match self {
            Self::Refresh => return None,
            Self::WifiConnect { ssid, .. } if ok => Notice::new("✓", trf("Connected to {}", &[ssid]), tr("Wi-Fi")),
            Self::WifiConnect { ssid, .. } => {
                Notice::new("⚠", tr("Connection failed"), trf("{}: {}", &[ssid, &err]))
            }
            Self::WifiDisconnect { ssid } if ok => Notice::new("✓", tr("Wi-Fi disconnected"), ssid.clone()),
            Self::BtPair { name, .. } if ok => Notice::new("✓", trf("Paired: {}", &[name]), tr("Bluetooth")),
            Self::BtPair { name, .. } => Notice::new("⚠", tr("Pairing failed"), trf("{}: {}", &[name, &err])),
            Self::BtConnect { name, .. } if ok => Notice::new("✓", trf("Connected to {}", &[name]), tr("Bluetooth")),
            Self::BtConnect { name, .. } => Notice::new("⚠", tr("Connection failed"), trf("{}: {}", &[name, &err])),
            Self::BtDisconnect { name, .. } if ok => Notice::new("✓", tr("Disconnected"), name.clone()),
            Self::BtRemove { name, .. } if ok => Notice::new("✓", tr("Device removed"), name.clone()),
            _ if ok => return None,
            Self::WifiDisconnect { .. } | Self::BtDisconnect { .. } => Notice::new("⚠", tr("Disconnecting failed"), err),
            Self::BtRemove { .. } => Notice::new("⚠", tr("Removing failed"), err),
            Self::WifiScan => Notice::new("⚠", tr("Wi-Fi scan failed"), err),
            Self::BtDiscovery(_) => Notice::new("⚠", tr("Bluetooth search failed"), err),
            Self::WifiEnable(_) => Notice::new("⚠", tr("Couldn't switch Wi-Fi"), err),
            Self::BtPower(_) => Notice::new("⚠", tr("Couldn't switch Bluetooth"), err),
        })
    }
}

/// Response of the backend thread
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Wifi(Result<WifiState, String>),
    Bluetooth(Result<BluetoothState, String>),
    /// A request (not `Refresh`) is done
    Done(Request, Result<(), String>),
}

/// NetworkManager/BlueZ or the simulation. Runs on the backend thread, so it may block.
pub trait Backend: Send {
    fn wifi(&mut self) -> Result<WifiState, String>;
    fn bluetooth(&mut self) -> Result<BluetoothState, String>;
    /// Runs a request other than `Refresh`.
    fn execute(&mut self, request: &Request) -> Result<(), String>;
}

/// Starts the backend thread. Requests that pile up while one runs (e.g. a pairing) are batched:
/// several `Refresh` give a single read. After every batch both states come back.
pub fn spawn(mut backend: Box<dyn Backend>) -> (Sender<Request>, Receiver<Response>) {
    let (request_tx, request_rx) = mpsc::channel::<Request>();
    let (response_tx, response_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("consoleui-network".into())
        .spawn(move || {
            while let Ok(first) = request_rx.recv() {
                let batch: Vec<Request> = std::iter::once(first).chain(request_rx.try_iter()).collect();
                for request in batch.into_iter().filter(|r| *r != Request::Refresh) {
                    let result = backend.execute(&request);
                    if let Err(err) = &result {
                        eprintln!("Network request {request:?} failed: {err}");
                    }
                    if response_tx.send(Response::Done(request, result)).is_err() {
                        return;
                    }
                }
                let wifi = Response::Wifi(backend.wifi());
                let bluetooth = Response::Bluetooth(backend.bluetooth());
                if response_tx.send(wifi).is_err() || response_tx.send(bluetooth).is_err() {
                    return;
                }
            }
        })
        .expect("cannot start the network thread");
    (request_tx, response_rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakenet::FakeBackend;
    use std::time::Duration;

    #[test]
    fn notices() {
        let connect = Request::WifiConnect { ssid: "Zuhause".into(), password: None };
        assert_eq!(connect.notice(&Ok(())).unwrap().title, "Connected to Zuhause");
        let failed = connect.notice(&Err("Wrong password".into())).unwrap();
        assert_eq!((failed.title.as_str(), failed.text.as_str()), ("Connection failed", "Zuhause: Wrong password"));
        let pair = Request::BtPair { address: "A".into(), name: "8BitDo".into() };
        assert_eq!(pair.notice(&Err("x".into())).unwrap().title, "Pairing failed");
        assert_eq!(Request::WifiScan.notice(&Ok(())), None);
        assert_eq!(Request::Refresh.notice(&Err("x".into())), None);
        assert_eq!(pair.target(), Some("A"));
    }

    #[test]
    fn worker_answers_with_states() {
        let (tx, rx) = spawn(Box::new(FakeBackend::new(Duration::ZERO)));
        tx.send(Request::Refresh).unwrap();
        tx.send(Request::WifiDisconnect { ssid: "Zuhause".into() }).unwrap();
        let mut done = false;
        let mut wifi = None;
        while wifi.is_none() || !done {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Response::Done(request, result) => {
                    assert_eq!(request, Request::WifiDisconnect { ssid: "Zuhause".into() });
                    assert!(result.is_ok());
                    done = true;
                }
                Response::Wifi(state) if done => wifi = Some(state.unwrap()),
                _ => {}
            }
        }
        assert!(wifi.unwrap().connected().is_none());
    }
}
