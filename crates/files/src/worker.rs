//! File work on a thread of its own: a large copy must not stop the UI.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::entries::{self, Entry};
use crate::ops;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    List { dir: PathBuf, hidden: bool },
    Copy { from: PathBuf, into: PathBuf },
    Move { from: PathBuf, into: PathBuf },
    Rename { path: PathBuf, name: String },
    NewFolder { dir: PathBuf, name: String },
    Trash(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Listed(PathBuf, Result<Vec<Entry>, String>),
    /// A change finished; the list needs a new look
    Done(Result<(), String>),
}

pub fn spawn() -> (Sender<Request>, Receiver<Response>) {
    let (requests, requests_rx) = mpsc::channel();
    let (responses, responses_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for request in requests_rx {
            let response = match request {
                Request::List { dir, hidden } => {
                    let listed = entries::list(&dir, hidden).map_err(|err| err.to_string());
                    Response::Listed(dir, listed)
                }
                Request::Copy { from, into } => Response::Done(ops::copy(&from, &into)),
                Request::Move { from, into } => Response::Done(ops::move_to(&from, &into)),
                Request::Rename { path, name } => Response::Done(ops::rename(&path, &name)),
                Request::NewFolder { dir, name } => Response::Done(ops::new_folder(&dir, &name)),
                Request::Trash(path) => {
                    let bin = ops::home_trash().ok_or_else(|| "No trash".to_string());
                    Response::Done(bin.and_then(|bin| ops::trash(&path, &bin)))
                }
            };
            if responses.send(response).is_err() {
                return;
            }
        }
    });
    (requests, responses_rx)
}
