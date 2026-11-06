//! The work behind the store on a thread of its own: loading the catalog, installing, removing.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use crate::appstream::Listing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Install(String),
    Remove(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Catalog(Result<Vec<Listing>, String>),
    /// In percent; `None` while nothing is known
    Progress(String, Option<u8>),
    Installed(String),
    Removed(String),
    Failed(String, String),
}

pub trait Backend: Send + 'static {
    fn load(&mut self) -> Result<Vec<Listing>, String>;
    fn install(&mut self, id: &str, progress: &mut dyn FnMut(Option<u8>)) -> Result<(), String>;
    fn remove(&mut self, id: &str) -> Result<(), String>;
}

/// Starts the backend; it loads the catalog at once and then works through the requests in order.
pub fn spawn(mut backend: impl Backend) -> (Sender<Request>, Receiver<Response>) {
    let (requests, requests_rx) = mpsc::channel();
    let (responses, responses_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = responses.send(Response::Catalog(backend.load()));
        for request in requests_rx {
            let response = match request {
                Request::Install(id) => {
                    let mut progress = |percent| {
                        let _ = responses.send(Response::Progress(id.clone(), percent));
                    };
                    match backend.install(&id, &mut progress) {
                        Ok(()) => Response::Installed(id),
                        Err(err) => Response::Failed(id, err),
                    }
                }
                Request::Remove(id) => match backend.remove(&id) {
                    Ok(()) => Response::Removed(id),
                    Err(err) => Response::Failed(id, err),
                },
            };
            let _ = responses.send(response);
        }
    });
    (requests, responses_rx)
}

/// Made-up apps for `--fake`: nothing is downloaded or changed. An ID with "broken" fails.
pub struct FakeBackend;

impl Backend for FakeBackend {
    fn load(&mut self) -> Result<Vec<Listing>, String> {
        let names = [
            "Astro Rally",
            "Bloom",
            "Cave Runner",
            "Drift Kings",
            "Echo Valley",
            "Frostbite",
            "Glider",
            "Harbor Town",
            "Iron Tactics",
            "Jade Temple",
            "Kite Fighter",
            "Lumen",
            "Moon Miner",
            "Nightfall",
            "Orbit",
            "Pixel Quest",
            "Quartz",
            "Red Canyon",
            "Star Forge",
            "Tide",
            "Umbra",
            "Vector",
            "Wild Roads",
            "Xenon",
            "Yeti",
            "Zephyr",
        ];
        let game = |(index, name): (usize, &&str)| Listing {
            id: format!("org.example.{}", name.replace(' ', "")),
            name: name.to_string(),
            summary: "A made-up game for trying out the store".into(),
            description: "This game does not exist. Installing it only pretends to download something.\n\n\
                • Works with a controller\n\n• Nothing changes on your system"
                .into(),
            developer: "ConsoleUI".into(),
            icon: None,
            game: true,
            gamepad: index % 3 == 0,
            verified: index % 2 == 0,
            installed: index == 1,
        };
        let mut listings: Vec<Listing> = names.iter().enumerate().map(game).collect();
        let app = |id: &str, name: &str| Listing {
            id: id.into(),
            name: name.into(),
            summary: "A made-up app".into(),
            developer: "ConsoleUI".into(),
            ..Listing::default()
        };
        listings.push(app("org.example.Player", "Player"));
        listings.push(app("org.example.broken", "Broken App"));
        Ok(listings)
    }

    fn install(&mut self, id: &str, progress: &mut dyn FnMut(Option<u8>)) -> Result<(), String> {
        for percent in (0..=100).step_by(10) {
            progress(Some(percent));
            std::thread::sleep(Duration::from_millis(60));
            if percent == 50 && id.contains("broken") {
                return Err("The download was interrupted.".into());
            }
        }
        Ok(())
    }

    fn remove(&mut self, _id: &str) -> Result<(), String> {
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }
}
