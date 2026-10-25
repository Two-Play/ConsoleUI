//! Loads hero images (large background artwork) off the UI thread (Story 6.5).
//!
//! Steam heroes are JPEGs up to 3840 px wide; decoding them on the UI thread caused visible
//! stutter while scrolling. Two workers decode and downscale them into `SharedPixelBuffer`s, the
//! UI thread collects the results while something is pending and fades the image in once it is
//! ready. Until then the previous image stays (no detour via an empty background). Neighbours
//! of the focused tile are prefetched; the cache is limited.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::AppWindow;

/// No hero is decoded wider than this (the background is darkened anyway).
const MAX_SIZE: (u32, u32) = (1920, 1080);
/// This many heroes stay decoded (about 5 MB each at 1920×620).
const CACHE_SIZE: usize = 12;
const WORKERS: usize = 2;
/// Poll interval while jobs are open; afterwards no timer runs.
const POLL: Duration = Duration::from_millis(15);

struct Job {
    id: String,
    path: PathBuf,
}

type Done = (String, Option<SharedPixelBuffer<Rgba8Pixel>>);

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    in_flight: HashSet<String>,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

struct State {
    ui: slint::Weak<AppWindow>,
    shared: Arc<Shared>,
    results: mpsc::Receiver<Done>,
    /// Most recently used at the end
    cache: VecDeque<(String, Image)>,
    failed: HashSet<String>,
    /// Entry whose hero should be visible
    wanted: Option<String>,
    /// Entry whose hero is currently in front
    shown: Option<String>,
    polling: bool,
}

pub struct Heroes {
    state: Rc<RefCell<State>>,
}

impl Heroes {
    pub fn new(ui: &AppWindow) -> Self {
        let shared = Arc::new(Shared::default());
        let (sender, results) = mpsc::channel();
        for _ in 0..WORKERS {
            let shared = shared.clone();
            let sender = sender.clone();
            std::thread::spawn(move || worker(&shared, &sender));
        }
        let state = State {
            ui: ui.as_weak(),
            shared,
            results,
            cache: VecDeque::new(),
            failed: HashSet::new(),
            wanted: None,
            shown: None,
            polling: false,
        };
        Self { state: Rc::new(RefCell::new(state)) }
    }

    /// Forces a new display on the next [`Heroes::show`] (e.g. after a new library).
    pub fn invalidate(&self) {
        self.state.borrow_mut().wanted = None;
    }

    /// Shows the hero of `focus` (`(id, path)`; `None` = no entry or no hero) and prefetches
    /// the heroes in `prefetch`, in this order.
    pub fn show(&self, id: Option<&str>, path: Option<&Path>, prefetch: Vec<(String, PathBuf)>) {
        let mut state = self.state.borrow_mut();
        if state.wanted.as_deref() == id && id.is_some() {
            return;
        }
        state.wanted = id.map(str::to_string);
        let mut jobs = Vec::new();
        match (id, path) {
            (Some(id), Some(path)) => {
                if let Some(image) = state.take_cached(id) {
                    state.display(id, image);
                } else if state.failed.contains(id) {
                    state.display(id, Image::default());
                } else {
                    jobs.push(Job { id: id.to_string(), path: path.to_path_buf() });
                }
            }
            _ => state.display_none(),
        }
        jobs.extend(
            prefetch
                .into_iter()
                .filter(|(id, _)| !state.failed.contains(id) && !state.cache.iter().any(|(c, _)| c == id))
                .map(|(id, path)| Job { id, path }),
        );
        state.enqueue(jobs);
        drop(state);
        start_polling(&self.state);
    }

    /// Already decoded hero (for the launch screen), without waiting for the workers.
    pub fn cached(&self, id: &str) -> Option<Image> {
        self.state.borrow().cache.iter().find(|(c, _)| c == id).map(|(_, image)| image.clone())
    }
}

impl State {
    fn take_cached(&mut self, id: &str) -> Option<Image> {
        let pos = self.cache.iter().position(|(c, _)| c == id)?;
        let entry = self.cache.remove(pos)?;
        let image = entry.1.clone();
        self.cache.push_back(entry);
        Some(image)
    }

    fn insert(&mut self, id: String, image: Image) {
        self.cache.retain(|(c, _)| *c != id);
        self.cache.push_back((id, image));
        while self.cache.len() > CACHE_SIZE {
            self.cache.pop_front();
        }
    }

    /// Replaces the queue: jobs for tiles left long ago are dropped.
    fn enqueue(&self, jobs: Vec<Job>) {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        let in_flight = std::mem::take(&mut queue.in_flight);
        queue.jobs = jobs.into_iter().filter(|job| !in_flight.contains(&job.id)).collect();
        queue.in_flight = in_flight;
        if !queue.jobs.is_empty() {
            self.shared.wake.notify_all();
        }
    }

    fn busy(&self) -> bool {
        let queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        !queue.jobs.is_empty() || !queue.in_flight.is_empty()
    }

    /// Puts the image into the currently invisible layer and crossfades.
    fn display(&mut self, id: &str, image: Image) {
        if self.shown.as_deref() == Some(id) {
            return;
        }
        self.shown = Some(id.to_string());
        self.flip(image);
    }

    /// Fades over to the empty background (accent gradient only).
    fn display_none(&mut self) {
        // `shown == None` means: there is already no image in front.
        if self.shown.take().is_some() {
            self.flip(Image::default());
        }
    }

    fn flip(&self, image: Image) {
        let Some(ui) = self.ui.upgrade() else { return };
        let front_a = !ui.get_hero_front_a();
        if front_a {
            ui.set_hero_a(image);
        } else {
            ui.set_hero_b(image);
        }
        ui.set_hero_front_a(front_a);
    }

    /// Collects finished images; `true` while jobs are still open.
    fn poll(&mut self) -> bool {
        // Check first, then collect: a worker reports its result before it removes the job.
        let busy = self.busy();
        while let Ok((id, pixels)) = self.results.try_recv() {
            match pixels {
                Some(pixels) => {
                    let image = Image::from_rgba8(pixels);
                    self.insert(id.clone(), image.clone());
                    if self.wanted.as_deref() == Some(id.as_str()) {
                        self.display(&id, image);
                    }
                }
                None => {
                    if self.wanted.as_deref() == Some(id.as_str()) {
                        self.display(&id, Image::default());
                    }
                    self.failed.insert(id);
                }
            }
        }
        busy
    }
}

fn start_polling(state: &Rc<RefCell<State>>) {
    {
        let mut s = state.borrow_mut();
        if s.polling || !s.busy() {
            return;
        }
        s.polling = true;
    }
    schedule(Rc::downgrade(state));
}

fn schedule(state: Weak<RefCell<State>>) {
    slint::Timer::single_shot(POLL, move || {
        let Some(strong) = state.upgrade() else { return };
        let busy = strong.borrow_mut().poll();
        if busy {
            schedule(state);
        } else {
            strong.borrow_mut().polling = false;
        }
    });
}

fn worker(shared: &Shared, results: &mpsc::Sender<Done>) {
    loop {
        let job = {
            let mut queue = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = queue.jobs.pop_front() {
                    queue.in_flight.insert(job.id.clone());
                    break job;
                }
                queue = shared.wake.wait(queue).unwrap_or_else(|e| e.into_inner());
            }
        };
        let pixels = decode(&job.path);
        if results.send((job.id.clone(), pixels)).is_err() {
            return;
        }
        shared.queue.lock().unwrap_or_else(|e| e.into_inner()).in_flight.remove(&job.id);
    }
}

fn decode(path: &Path) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let image = image::open(path).map_err(|err| eprintln!("Cannot read hero {}: {err}", path.display())).ok()?;
    let (w, h) = MAX_SIZE;
    let image = if image.width() > w || image.height() > h { image.thumbnail(w, h) } else { image };
    let rgba = image.into_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
}
