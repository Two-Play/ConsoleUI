//! Hero-Bilder (großes Artwork im Hintergrund) abseits des UI-Threads laden (Story 6.5).
//!
//! Steam-Heroes sind JPEGs mit bis zu 3840 px Breite; sie im UI-Thread zu dekodieren kostete beim
//! Scrollen sichtbare Ruckler. Zwei Worker dekodieren und verkleinern sie in `SharedPixelBuffer`s,
//! der UI-Thread holt die Ergebnisse ab, solange etwas aussteht, und blendet das Bild ein, sobald es
//! fertig ist. Bis dahin bleibt das bisherige Bild stehen (kein Umweg über einen leeren
//! Hintergrund). Nachbarn der fokussierten Kachel werden vorab geladen; der Cache ist begrenzt.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::AppWindow;

/// Breiter wird kein Hero dekodiert (der Hintergrund ist ohnehin abgedunkelt).
const MAX_SIZE: (u32, u32) = (1920, 1080);
/// So viele Heroes bleiben dekodiert (je ca. 5 MB bei 1920×620).
const CACHE_SIZE: usize = 12;
const WORKERS: usize = 2;
/// Abholintervall, solange Aufträge offen sind; danach läuft kein Timer.
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
    /// Zuletzt benutzte am Ende
    cache: VecDeque<(String, Image)>,
    failed: HashSet<String>,
    /// Eintrag, dessen Hero zu sehen sein soll
    wanted: Option<String>,
    /// Eintrag, dessen Hero gerade vorne liegt
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

    /// Erzwingt beim nächsten [`Heroes::show`] eine neue Anzeige (z. B. nach neuer Bibliothek).
    pub fn invalidate(&self) {
        self.state.borrow_mut().wanted = None;
    }

    /// Zeigt den Hero von `focus` (`(id, Pfad)`; `None` = kein Eintrag oder kein Hero) und lädt
    /// die Heroes in `prefetch` vorab, in dieser Reihenfolge.
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

    /// Schon dekodierter Hero (für den Startbildschirm), ohne auf die Worker zu warten.
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

    /// Ersetzt die Warteschlange: Aufträge für längst verlassene Kacheln verfallen.
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

    /// Legt das Bild in die gerade unsichtbare Ebene und blendet um.
    fn display(&mut self, id: &str, image: Image) {
        if self.shown.as_deref() == Some(id) {
            return;
        }
        self.shown = Some(id.to_string());
        self.flip(image);
    }

    /// Blendet zum leeren Hintergrund (nur Akzentverlauf) über.
    fn display_none(&mut self) {
        // `shown == None` heißt: vorne liegt schon kein Bild.
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

    /// Holt fertige Bilder ab; `true`, solange noch Aufträge offen sind.
    fn poll(&mut self) -> bool {
        // Erst prüfen, dann abholen: ein Worker meldet sein Ergebnis, bevor er den Auftrag austrägt.
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
    let image = image::open(path).map_err(|err| eprintln!("Hero {} nicht lesbar: {err}", path.display())).ok()?;
    let (w, h) = MAX_SIZE;
    let image = if image.width() > w || image.height() > h { image.thumbnail(w, h) } else { image };
    let rgba = image.into_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
}
