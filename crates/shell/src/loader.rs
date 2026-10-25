//! Loads the library in the background while the boot animation runs: Steam and desktop
//! discovery, decoding covers and logos (in parallel), computing accent colors.
//! Slint `Image`s are only created on the UI thread; only `SharedPixelBuffer`s belong here.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use consoleui_core::game_options::GameOptionsStore;
use consoleui_core::{
    AppEntry, Collection, History, Launch, accent, apply_history, discover_library, dummy_library, sort_by_recent,
};
use slint::{Rgba8Pixel, SharedPixelBuffer};

/// Covers never get larger than this in the tiles (not even in 4K); smaller textures load faster.
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
    /// Pinned and hidden entries
    pub collection: Collection,
    pub collection_path: Option<PathBuf>,
    /// Nested gamescope options per entry (Story 7.10)
    pub game_options: GameOptionsStore,
    pub game_options_path: Option<PathBuf>,
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
        let collection_path = Collection::default_path();
        let collection = collection_path.as_deref().map(Collection::load).unwrap_or_default();
        let game_options_path = GameOptionsStore::default_path();
        let game_options = game_options_path.as_deref().map(GameOptionsStore::load).unwrap_or_default();
        let mut entries = discover_library();
        if !entries.is_empty() {
            apply_history(&mut entries, &history);
            sort_by_recent(&mut entries);
            return Library {
                entries: decode_all(entries),
                history,
                history_path,
                collection,
                collection_path,
                game_options,
                game_options_path,
            };
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
    // Dummy entries save neither history, collection nor game options (their IDs are not real).
    Library {
        entries: decode_all(entries),
        history: History::default(),
        history_path: None,
        collection: Collection::default(),
        collection_path: None,
        game_options: GameOptionsStore::default(),
        game_options_path: None,
    }
}

/// Decodes the images on several threads; the order is kept.
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
    let image = image::open(path).map_err(|err| eprintln!("Image {} not readable: {err}", path.display())).ok()?;
    let image = match max {
        Some((w, h)) if image.width() > w || image.height() > h => image.thumbnail(w, h),
        _ => image,
    };
    let rgba = image.into_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
}
