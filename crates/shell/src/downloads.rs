//! Steam downloads: watches the `appmanifest_*.acf` files, so the shell can say when a game
//! starts or finishes downloading (Steam writes the state into them as it goes).

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use consoleui_core::steam;

/// How often the manifests are looked at
const POLL: Duration = Duration::from_secs(10);

/// Names of games whose download started or finished since the last look
pub struct DownloadNews {
    pub started: Vec<String>,
    pub finished: Vec<String>,
}

pub fn start() -> Receiver<DownloadNews> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let Some(root) = steam::find_root() else { return };
        let mut before = steam::download_states(&root);
        loop {
            std::thread::sleep(POLL);
            let after = steam::download_states(&root);
            let (started, finished) = steam::download_changes(&before, &after);
            before = after;
            if (!started.is_empty() || !finished.is_empty()) && sender.send(DownloadNews { started, finished }).is_err()
            {
                return;
            }
        }
    });
    receiver
}
