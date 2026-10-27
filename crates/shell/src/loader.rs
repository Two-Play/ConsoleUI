//! Loads the library in the background while the boot animation runs: Steam and desktop
//! discovery, decoding covers and logos (in parallel), computing accent colors.
//! Slint `Image`s are only created on the UI thread; only `SharedPixelBuffer`s belong here.

use std::path::Path;
use std::sync::mpsc;

use consoleui_core::{AppEntry, Launch, accent, discover_library, dummy_library};
use slint::{Rgba8Pixel, SharedPixelBuffer};

/// Covers never get larger than this in the tiles (not even in 4K); smaller textures load faster.
const COVER_MAX: (u32, u32) = (480, 720);

pub struct LoadOptions {
    pub dummy: bool,
    pub tiles: usize,
    pub dummy_launch: Option<String>,
    /// Decode covers and logos (settings row "Game artwork")
    pub artwork: bool,
}

pub struct LoadedEntry {
    pub entry: AppEntry,
    pub cover: Option<SharedPixelBuffer<Rgba8Pixel>>,
    pub logo: Option<SharedPixelBuffer<Rgba8Pixel>>,
}

/// The entries in discovery order; the shell sorts them by the active profile's history.
pub struct Library {
    pub entries: Vec<LoadedEntry>,
    /// Made-up entries: the profile saves neither history, collection nor game options for them
    /// (their IDs are not real).
    pub dummy: bool,
}

/// Decodes the covers and logos of `entries` later, after "Game artwork" was switched on.
pub fn spawn_artwork(entries: Vec<AppEntry>) -> mpsc::Receiver<Vec<LoadedEntry>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(decode_all(entries, true));
    });
    receiver
}

/// Looks for Steam games that are installed now but not in `known` (their downloads finished).
pub fn spawn_new_games(known: Vec<String>, artwork: bool) -> mpsc::Receiver<Vec<LoadedEntry>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let found = consoleui_core::steam::find_root().map(|root| consoleui_core::steam::discover(&root));
        let new: Vec<AppEntry> = found.unwrap_or_default().into_iter().filter(|e| !known.contains(&e.id)).collect();
        let _ = sender.send(decode_all(new, artwork));
    });
    receiver
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
        let entries = discover_library();
        if !entries.is_empty() {
            return Library { entries: decode_all(entries, options.artwork), dummy: false };
        }
        eprintln!("No games or apps found, showing dummy entries.");
    }
    let mut entries = dummy_library(options.tiles);
    if let Some(command) = &options.dummy_launch {
        let argv: Vec<String> = command.split_whitespace().map(str::to_string).collect();
        for entry in &mut entries {
            entry.launch = Launch::Command(argv.clone());
        }
    }
    Library { entries: decode_all(entries, options.artwork), dummy: true }
}

/// Decodes the images on several threads; the order is kept.
fn decode_all(entries: Vec<AppEntry>, artwork: bool) -> Vec<LoadedEntry> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let chunk = entries.len().div_ceil(threads).max(1);
    let mut chunks: Vec<Vec<AppEntry>> = Vec::new();
    let mut rest = entries;
    while !rest.is_empty() {
        let tail = rest.split_off(chunk.min(rest.len()));
        chunks.push(std::mem::replace(&mut rest, tail));
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| scope.spawn(move || chunk.into_iter().map(|entry| decode(entry, artwork)).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    })
}

fn decode(mut entry: AppEntry, artwork: bool) -> LoadedEntry {
    if !artwork {
        return LoadedEntry { entry, cover: None, logo: None };
    }
    let cover = entry.artwork.cover.as_deref().and_then(|p| read(p, Some(COVER_MAX)));
    if let Some((primary, secondary)) = cover.as_ref().and_then(|px| accent::from_rgba(px.as_bytes())) {
        entry.accent = primary;
        entry.accent2 = secondary;
    }
    let logo = entry.artwork.logo.as_deref().and_then(|p| read(p, None));
    LoadedEntry { entry, cover, logo }
}

fn read(path: &Path, max: Option<(u32, u32)>) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let image = image::open(path).map_err(|err| eprintln!("Image {} not readable: {err}", path.display())).ok()?;
    let image = match max {
        Some((w, h)) if image.width() > w || image.height() > h => image.thumbnail(w, h),
        _ => image,
    };
    let rgba = image.into_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
}
