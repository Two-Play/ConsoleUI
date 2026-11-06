//! Screenshots of the detail page. The catalog only has their addresses, so they are downloaded in
//! the background and kept in the cache directory. Without a network the page simply has none.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use slint::{Rgba8Pixel, SharedPixelBuffer};

/// Images are not kept for longer than this.
const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);
/// Larger images only cost memory: even the full view is at most this big in most cases.
const MAX_SIZE: (u32, u32) = (1600, 900);
const WORKERS: usize = 2;

pub enum Loaded {
    Ready(String, SharedPixelBuffer<Rgba8Pixel>),
    Failed(String),
}

#[derive(Default)]
struct Queue {
    urls: Mutex<VecDeque<String>>,
    added: Condvar,
}

pub struct Shots {
    queue: Arc<Queue>,
    pub loaded: Receiver<Loaded>,
}

impl Shots {
    /// `fake`: painted images instead of downloads.
    pub fn start(fake: bool) -> Self {
        let queue = Arc::new(Queue::default());
        let (sender, loaded) = mpsc::channel();
        let cache = consoleui_core::xdg::cache_home().map(|dir| dir.join("consoleui/store"));
        if let Some(dir) = cache.as_deref().filter(|_| !fake) {
            let dir = dir.to_path_buf();
            std::thread::spawn(move || prune(&dir));
        }
        for _ in 0..WORKERS {
            let (queue, sender, cache) = (queue.clone(), sender.clone(), cache.clone());
            std::thread::spawn(move || work(&queue, &sender, cache.as_deref(), fake));
        }
        Self { queue, loaded }
    }

    /// Replaces what is waiting: only the page on the screen counts.
    pub fn want(&self, urls: impl Iterator<Item = String>) {
        *self.queue.urls.lock().unwrap() = urls.collect();
        self.queue.added.notify_all();
    }
}

fn work(queue: &Queue, sender: &Sender<Loaded>, cache: Option<&Path>, fake: bool) {
    loop {
        let url = {
            let mut urls = queue.urls.lock().unwrap();
            loop {
                match urls.pop_front() {
                    Some(url) => break url,
                    None => urls = queue.added.wait(urls).unwrap(),
                }
            }
        };
        let image = if fake { paint(&url) } else { cache.and_then(|dir| fetch(dir, &url)) };
        let loaded = image.map_or_else(|| Loaded::Failed(url.clone()), |image| Loaded::Ready(url.clone(), image));
        if sender.send(loaded).is_err() {
            return;
        }
    }
}

/// FNV-1a: stable across builds, so the cache stays valid.
fn file_name(url: &str) -> String {
    let hash = url
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3));
    format!("{hash:016x}")
}

fn fetch(dir: &Path, url: &str) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let path = dir.join(file_name(url));
    if !path.exists() {
        download(url, &path)?;
    }
    let data = fs::read(&path).ok()?;
    let decoded = image::load_from_memory(&data);
    if decoded.is_err() {
        // Not an image: do not keep it
        let _ = fs::remove_file(&path);
    }
    let image = decoded.ok()?;
    let image = if image.width() > MAX_SIZE.0 || image.height() > MAX_SIZE.1 {
        image.thumbnail(MAX_SIZE.0, MAX_SIZE.1)
    } else {
        image
    };
    let rgba = image.into_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
}

fn download(url: &str, path: &Path) -> Option<()> {
    fs::create_dir_all(path.parent()?).ok()?;
    let part: PathBuf = path.with_extension("part");
    let status = Command::new("curl")
        .args(["--fail", "--silent", "--location", "--proto", "=https", "--max-time", "30"])
        .args(["--max-filesize", "30000000", "--output"])
        .arg(&part)
        .arg(url)
        .stdin(Stdio::null())
        .status();
    if status.is_ok_and(|status| status.success()) {
        return fs::rename(&part, path).ok();
    }
    let _ = fs::remove_file(&part);
    None
}

fn prune(dir: &Path) {
    let Ok(files) = fs::read_dir(dir) else { return };
    for file in files.flatten() {
        let old = file.metadata().and_then(|data| data.modified()).ok();
        if old.and_then(|time| time.elapsed().ok()).is_some_and(|age| age > MAX_AGE) {
            let _ = fs::remove_file(file.path());
        }
    }
}

/// `fake:N` paints a gradient after a moment; anything else fails.
fn paint(url: &str) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    std::thread::sleep(Duration::from_millis(100));
    let index: u32 = url.strip_prefix("fake:")?.parse().ok()?;
    let (width, height) = (640, 360);
    let mut image = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    for (position, pixel) in image.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = (position as u32 % width, position as u32 / width);
        let shade = |base: u32| ((base + index * 60 + x / 4 + y / 3) % 256) as u8;
        *pixel = Rgba8Pixel { r: shade(30), g: shade(90), b: shade(150), a: 255 };
    }
    Some(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_names_are_stable_and_differ() {
        assert_eq!(file_name("a"), "af63dc4c8601ec8c");
        assert_ne!(file_name("https://x/1.png"), file_name("https://x/2.png"));
    }

    #[test]
    fn only_fake_addresses_paint() {
        assert!(paint("fake:1").is_some());
        assert!(paint("fake:broken").is_none());
    }
}
