//! Turns loaded entries into the `AppTile` struct of the UI.

use std::path::Path;

use chrono::{DateTime, Local, NaiveDate};
use consoleui_core::i18n::{tr, trn};
use consoleui_core::image_format::{ImageFormat, sniff};
use consoleui_core::{AppEntry, AppKind, last_used_label};
use slint::{Color, Image, SharedPixelBuffer};

use crate::loader::LoadedEntry;
use crate::{AppTile, cover};

const COVER_SIZE: u32 = 384;

/// Loads an icon. Slint detects the format only by the file extension; without one (e.g.
/// `~/AppImages/.icons/name`) or when Slint fails, it is detected from the file content.
pub fn load_icon(path: &Path) -> Option<Image> {
    if path.extension().is_some()
        && let Ok(image) = Image::load_from_path(path)
    {
        return Some(image);
    }
    let data = std::fs::read(path)
        .map_err(|err| eprintln!("Cannot read image {}: {err}", path.display()))
        .ok()?;
    let image = match sniff(&data) {
        Some(ImageFormat::Svg) => Image::load_from_svg_data(&data).ok(),
        Some(ImageFormat::Png | ImageFormat::Jpeg) => image::load_from_memory(&data).ok().map(|img| {
            let rgba = img.into_rgba8();
            Image::from_rgba8(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
        }),
        None => None,
    };
    if image.is_none() {
        eprintln!("Image {}: unknown format or broken file", path.display());
    }
    image
}

/// Cover and logo are already decoded in the background; Slint loads icons (mostly SVG) here itself.
pub fn to_tile(loaded: &LoadedEntry, index: usize, today: NaiveDate) -> AppTile {
    let entry = &loaded.entry;
    let real_cover = loaded.cover.clone().map(Image::from_rgba8);
    let icon = if real_cover.is_none() { entry.artwork.icon.as_deref().and_then(load_icon) } else { None };
    let logo = loaded.logo.clone().map(Image::from_rgba8);
    AppTile {
        title: entry.title.as_str().into(),
        subtitle: subtitle(entry, today).into(),
        kind: kind_label(entry.kind).into(),
        action_label: action_label(entry.kind, false).into(),
        monogram: entry.monogram().into(),
        show_monogram: real_cover.is_none() && icon.is_none(),
        has_icon: icon.is_some(),
        icon: icon.unwrap_or_default(),
        has_logo: logo.is_some(),
        logo: logo.unwrap_or_default(),
        cover: real_cover
            .unwrap_or_else(|| cover::generate(entry.accent, entry.accent2, index as u32, COVER_SIZE)),
        accent: Color::from_rgb_u8(entry.accent[0], entry.accent[1], entry.accent[2]),
        running: false,
        pinned: false,
    }
}

/// Badge above the title
pub fn kind_label(kind: AppKind) -> &'static str {
    match kind {
        AppKind::Game => tr("GAME"),
        AppKind::App => tr("APP"),
    }
}

/// Label of the launch button
pub fn action_label(kind: AppKind, running: bool) -> &'static str {
    match (kind, running) {
        (_, true) => tr("Resume"),
        (AppKind::Game, false) => tr("Play"),
        (AppKind::App, false) => tr("Open"),
    }
}

/// Last tile of the home row, opens the library.
pub fn library_tile(count: usize) -> AppTile {
    let (accent, accent2) = ([0x3b, 0x41, 0x52], [0x8a, 0x93, 0xa8]);
    let icon = Image::load_from_svg_data(include_bytes!("../ui/icons/library.svg")).unwrap_or_default();
    AppTile {
        title: tr("Library").into(),
        subtitle: trn("{n} game or app", "{n} games and apps", count, &[]).into(),
        kind: tr("ALL APPS").into(),
        action_label: tr("Open").into(),
        monogram: "".into(),
        show_monogram: false,
        cover: cover::generate(accent, accent2, 7, COVER_SIZE),
        has_icon: true,
        icon,
        logo: Image::default(),
        has_logo: false,
        accent: Color::from_rgb_u8(accent[0], accent[1], accent[2]),
        running: false,
        pinned: false,
    }
}

/// "Last played: …", otherwise the description of the entry.
pub fn subtitle(entry: &AppEntry, today: NaiveDate) -> String {
    entry
        .last_used
        .and_then(|t| DateTime::from_timestamp(t as i64, 0))
        .map(|utc| last_used_label(entry.kind, utc.with_timezone(&Local).date_naive(), today))
        .unwrap_or_else(|| entry.description.clone())
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}
