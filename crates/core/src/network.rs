//! Anbindung von WLAN und Bluetooth (Stories 5.3/5.4): Aufträge an ein Backend, das in einem
//! eigenen Thread läuft, und die Antworten zurück an die UI. Das echte Backend (NetworkManager
//! und BlueZ über D-Bus) steckt in der Shell, das simulierte in [`crate::fakenet`].

use std::sync::mpsc::{self, Receiver, Sender};

use crate::bluetooth::BluetoothState;
use crate::wifi::WifiState;

/// Ein Auftrag an das Backend. Alles außer `Refresh` ändert den Systemzustand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// WLAN- und Bluetooth-Zustand neu lesen
    Refresh,
    WifiEnable(bool),
    WifiScan,
    /// Ohne Passwort: offenes Netz oder gespeichertes Profil
    WifiConnect { ssid: String, password: Option<String> },
    WifiDisconnect { ssid: String },
    BtPower(bool),
    BtDiscovery(bool),
    /// Koppeln, vertrauen und verbinden
    BtPair { address: String, name: String },
    BtConnect { address: String, name: String },
    BtDisconnect { address: String, name: String },
    BtRemove { address: String, name: String },
}

/// Hinweis für die UI (wird als Toast angezeigt)
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
    /// SSID bzw. Geräteadresse, an der die Seite „läuft gerade …“ anzeigt
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

    /// Text in der Zeile, solange der Auftrag läuft
    pub fn busy_label(&self) -> &'static str {
        match self {
            Self::WifiConnect { .. } | Self::BtConnect { .. } => "Verbinde …",
            Self::WifiDisconnect { .. } | Self::BtDisconnect { .. } => "Trenne …",
            Self::BtPair { .. } => "Kopple …",
            Self::BtRemove { .. } => "Entferne …",
            Self::WifiScan | Self::BtDiscovery(true) => "Suche …",
            _ => "Bitte warten …",
        }
    }

    /// Hinweis nach dem Auftrag; Erfolge ohne sichtbare Wirkung bleiben still.
    pub fn notice(&self, result: &Result<(), String>) -> Option<Notice> {
        let ok = result.is_ok();
        let err = result.as_ref().err().map(String::as_str).unwrap_or_default();
        Some(match self {
            Self::Refresh => return None,
            Self::WifiConnect { ssid, .. } if ok => Notice::new("✓", format!("Verbunden mit {ssid}"), "WLAN"),
            Self::WifiConnect { ssid, .. } => Notice::new("⚠", "Verbindung fehlgeschlagen", format!("{ssid}: {err}")),
            Self::WifiDisconnect { ssid } if ok => Notice::new("✓", "WLAN getrennt", ssid.clone()),
            Self::BtPair { name, .. } if ok => Notice::new("✓", format!("Gekoppelt: {name}"), "Bluetooth"),
            Self::BtPair { name, .. } => Notice::new("⚠", "Kopplung fehlgeschlagen", format!("{name}: {err}")),
            Self::BtConnect { name, .. } if ok => Notice::new("✓", format!("Verbunden mit {name}"), "Bluetooth"),
            Self::BtConnect { name, .. } => Notice::new("⚠", "Verbindung fehlgeschlagen", format!("{name}: {err}")),
            Self::BtDisconnect { name, .. } if ok => Notice::new("✓", "Getrennt", name.clone()),
            Self::BtRemove { name, .. } if ok => Notice::new("✓", "Gerät entfernt", name.clone()),
            _ if ok => return None,
            Self::WifiDisconnect { .. } | Self::BtDisconnect { .. } => Notice::new("⚠", "Trennen fehlgeschlagen", err),
            Self::BtRemove { .. } => Notice::new("⚠", "Entfernen fehlgeschlagen", err),
            Self::WifiScan => Notice::new("⚠", "WLAN-Suche fehlgeschlagen", err),
            Self::BtDiscovery(_) => Notice::new("⚠", "Bluetooth-Suche fehlgeschlagen", err),
            Self::WifiEnable(_) => Notice::new("⚠", "WLAN lässt sich nicht schalten", err),
            Self::BtPower(_) => Notice::new("⚠", "Bluetooth lässt sich nicht schalten", err),
        })
    }
}

/// Antwort des Backend-Threads
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Wifi(Result<WifiState, String>),
    Bluetooth(Result<BluetoothState, String>),
    /// Ein Auftrag (nicht `Refresh`) ist fertig
    Done(Request, Result<(), String>),
}

/// NetworkManager/BlueZ oder die Simulation. Läuft im Backend-Thread, darf also blockieren.
pub trait Backend: Send {
    fn wifi(&mut self) -> Result<WifiState, String>;
    fn bluetooth(&mut self) -> Result<BluetoothState, String>;
    /// Führt einen Auftrag außer `Refresh` aus.
    fn execute(&mut self, request: &Request) -> Result<(), String>;
}

/// Startet den Backend-Thread. Aufträge, die sich während eines laufenden Auftrags (z. B. einer
/// Kopplung) ansammeln, werden zusammengefasst: mehrere `Refresh` ergeben nur ein Neulesen.
/// Nach jedem Schub kommen beide Zustände zurück.
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
                        eprintln!("Netzwerk-Auftrag {request:?} fehlgeschlagen: {err}");
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
        .expect("Netzwerk-Thread lässt sich nicht starten");
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
        assert_eq!(connect.notice(&Ok(())).unwrap().title, "Verbunden mit Zuhause");
        let failed = connect.notice(&Err("Falsches Passwort".into())).unwrap();
        assert_eq!((failed.title.as_str(), failed.text.as_str()), ("Verbindung fehlgeschlagen", "Zuhause: Falsches Passwort"));
        let pair = Request::BtPair { address: "A".into(), name: "8BitDo".into() };
        assert_eq!(pair.notice(&Err("x".into())).unwrap().title, "Kopplung fehlgeschlagen");
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
