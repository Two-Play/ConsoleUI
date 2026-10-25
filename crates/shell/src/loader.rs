//! Lädt die Bibliothek im Hintergrund, während die Boot-Animation läuft: Steam- und
//! Desktop-Erkennung, Cover und Logos dekodieren (parallel), Akzentfarben berechnen.
//! Slint-`Image`s entstehen erst im UI-Thread; hierher gehören nur `SharedPixelBuffer`.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use consoleui_core::{
    AppEntry, History, Launch, accent, apply_history, discover_library, dummy_library, sort_by_recent,
};
use slint::{Rgba8Pixel, SharedPixelBuffer};

/// Größer werden Cover in den Kacheln nie (auch nicht in 4K); kleinere Texturen laden schneller.
const COVER_MAX: (u32, u32) = (480, 720);

pub struct LoadOptions {
    pub dummy: bool,
    pub tiles: usize,
    pub dummy_launch: Option<String>,
}

pub struct LoadedEntry {
    pub entry: AppEntry,
    pub cover: Option<SharedPixelBuffer<Rgba8Pixel>>,
    pub logo: Option<SharedPixelBuffer<Rgba8Pixel>>,
}

pub struct Library {
    pub entries: Vec<LoadedEntry>,
    pub history: History,
    pub history_path: Option<PathBuf>,
}

pub fn spawn(options: LoadOptions) -> mpsc::Receiver<Library> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(load(&options));
    });
    receiver
}

fn load(options: &LoadOptions) -> Library {
    if !options.dummy {
        let history_path = History::default_path();
        let history = history_path.as_deref().map(History::load).unwrap_or_default();
        let mut entries = discover_library();
        if !entries.is_empty() {
            apply_history(&mut entries, &history);
            sort_by_recent(&mut entries);
            return Library { entries: decode_all(entries), history, history_path };
        }
        eprintln!("Keine Spiele oder Apps gefunden, zeige Dummy-Einträge.");
    }
    let mut entries = dummy_library(options.tiles);
    if let Some(command) = &options.dummy_launch {
        let argv: Vec<String> = command.split_whitespace().map(str::to_string).collect();
        for entry in &mut entries {
            entry.launch = Launch::Command(argv.clone());
        }
    }
    Library { entries: decode_all(entries), history: History::default(), history_path: None }
}

/// Dekodiert die Bilder auf mehreren Threads; die Reihenfolge bleibt erhalten.
fn decode_all(entries: Vec<AppEntry>) -> Vec<LoadedEntry> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let chunk = entries.len().div_ceil(threads).max(1);
    let mut chunks: Vec<Vec<AppEntry>> = Vec::new();
    let mut rest = entries;
    while !rest.is_empty() {
        let tail = rest.split_off(chunk.min(rest.len()));
        chunks.push(std::mem::replace(&mut rest, tail));
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> =
            chunks.into_iter().map(|chunk| scope.spawn(|| chunk.into_iter().map(decode).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    })
}

fn decode(mut entry: AppEntry) -> LoadedEntry {
    let cover = entry.artwork.cover.as_deref().and_then(|p| read(p, Some(COVER_MAX)));
    if let Some((primary, secondary)) = cover.as_ref().and_then(|px| accent::from_rgba(px.as_bytes())) {
        entry.accent = primary;
        entry.accent2 = secondary;
    }
    let logo = entry.artwork.logo.as_deref().and_then(|p| read(p, None));
    LoadedEntry { entry, cover, logo }
}

fn read(path: &Path, max: Option<(u32, u32)>) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let image = image::open(path).map_err(|err| eprintln!("Bild {} nicht lesbar: {err}", path.display())).ok()?;
    let image = match max {
        Some((w, h)) if image.width() > w || image.height() > h => image.thumbnail(w, h),
        _ => image,
    };
    let rgba = image.into_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
}
