//! Wandelt geladene Einträge in die `AppTile`-Struktur der UI um.

use std::path::Path;

use chrono::{DateTime, Local, NaiveDate};
use consoleui_core::image_format::{ImageFormat, sniff};
use consoleui_core::{AppEntry, AppKind, last_used_label};
use slint::{Color, Image, SharedPixelBuffer};

use crate::loader::LoadedEntry;
use crate::{AppTile, cover};

const COVER_SIZE: u32 = 384;

/// Lädt ein Icon. Slint erkennt das Format nur an der Endung; ohne Endung (z. B.
/// `~/AppImages/.icons/name`) oder wenn Slint scheitert, wird es am Dateiinhalt bestimmt.
pub fn load_icon(path: &Path) -> Option<Image> {
    if path.extension().is_some()
        && let Ok(image) = Image::load_from_path(path)
    {
        return Some(image);
    }
    let data = std::fs::read(path)
        .map_err(|err| eprintln!("Bild {} konnte nicht gelesen werden: {err}", path.display()))
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
        eprintln!("Bild {}: Format unbekannt oder Datei beschädigt", path.display());
    }
    image
}

/// Cover und Logo sind schon im Hintergrund dekodiert; Icons (meist SVG) lädt Slint hier selbst.
pub fn to_tile(loaded: &LoadedEntry, index: usize, today: NaiveDate) -> AppTile {
    let entry = &loaded.entry;
    let real_cover = loaded.cover.clone().map(Image::from_rgba8);
    let icon = if real_cover.is_none() { entry.artwork.icon.as_deref().and_then(load_icon) } else { None };
    let logo = loaded.logo.clone().map(Image::from_rgba8);
    let kind = match entry.kind {
        AppKind::Game => "SPIEL",
        AppKind::App => "APP",
    };

    AppTile {
        title: entry.title.as_str().into(),
        subtitle: subtitle(entry, today).into(),
        kind: kind.into(),
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

/// Beschriftung des Start-Buttons
pub fn action_label(kind: AppKind, running: bool) -> &'static str {
    match (kind, running) {
        (_, true) => "Fortsetzen",
        (AppKind::Game, false) => "Spielen",
        (AppKind::App, false) => "Öffnen",
    }
}

/// Letzte Kachel der Home-Reihe, öffnet die Bibliothek.
pub fn library_tile(count: usize) -> AppTile {
    let (accent, accent2) = ([0x3b, 0x41, 0x52], [0x8a, 0x93, 0xa8]);
    let icon = Image::load_from_svg_data(include_bytes!("../ui/icons/library.svg")).unwrap_or_default();
    AppTile {
        title: "Bibliothek".into(),
        subtitle: format!("{count} Spiele und Apps").into(),
        kind: "ALLE APPS".into(),
        action_label: "Öffnen".into(),
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

/// „Zuletzt gespielt: …“, sonst die Beschreibung des Eintrags.
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
