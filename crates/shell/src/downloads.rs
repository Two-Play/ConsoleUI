//! Steam downloads: watches the `appmanifest_*.acf` files, so the shell can say when a game
//! starts or finishes downloading (Steam writes the state into them as it goes).

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use consoleui_core::steam;

/// How often the manifests are looked at
const POLL: Duration = Duration::from_secs(10);

/// Names of games whose download started or finished since the last look
pub struct DownloadNews {
    /// The Steam app IDs of `started`
    pub started_ids: Vec<u32>,
    pub started: Vec<String>,
    /// Games whose shaders Steam began to process
    pub shaders: Vec<String>,
    pub finished: Vec<String>,
}

pub fn start() -> Receiver<DownloadNews> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let Some(root) = steam::find_root() else { return };
        let mut before = steam::download_states(&root);
        let mut shaders_before = steam::shader_processing();
        loop {
            std::thread::sleep(POLL);
            let after = steam::download_states(&root);
            let (started, finished) = steam::download_changes(&before, &after);
            let started_ids = after.iter().filter(|(id, (_, ready))| !ready && !before.contains_key(id));
            let started_ids = started_ids.map(|(id, _)| *id).collect();
            let shaders_now = steam::shader_processing();
            let mut shaders: Vec<String> = shaders_now
                .difference(&shaders_before)
                .filter_map(|id| after.get(id).map(|(name, _)| name.clone()))
                .collect();
            shaders.sort();
            shaders_before = shaders_now;
            before = after;
            let news = DownloadNews { started_ids, started, shaders, finished };
            let any = !news.started.is_empty() || !news.finished.is_empty() || !news.shaders.is_empty();
            if any && sender.send(news).is_err() {
                return;
            }
        }
    });
    receiver
}
