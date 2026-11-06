//! What is in a folder.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use consoleui_core::search::normalize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Folder,
    Image,
    Audio,
    Video,
    Text,
    Archive,
    Other,
}

impl Kind {
    /// By the file name, which is all a list of thousands of files can afford.
    pub fn of(name: &str) -> Self {
        let extension = name.rsplit_once('.').map(|(_, extension)| extension.to_lowercase()).unwrap_or_default();
        match extension.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif" | "heic" => Self::Image,
            "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" | "aac" => Self::Audio,
            "mp4" | "mkv" | "webm" | "avi" | "mov" | "m4v" => Self::Video,
            "txt" | "md" | "pdf" | "log" | "odt" | "doc" | "docx" | "rtf" | "csv" => Self::Text,
            "zip" | "gz" | "xz" | "bz2" | "zst" | "tar" | "7z" | "rar" | "iso" => Self::Archive,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// Folders first, then by name; dot files only with `hidden`.
pub fn list(dir: &Path, hidden: bool) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(dir)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && !hidden {
            continue;
        }
        let path = item.path();
        // Follows links, so a link to a folder opens like a folder; a broken one stays a file
        let data = fs::metadata(&path).or_else(|_| fs::symlink_metadata(&path))?;
        let kind = if data.is_dir() { Kind::Folder } else { Kind::of(&name) };
        entries.push(Entry { name, path, kind, size: data.len(), modified: data.modified().ok() });
    }
    entries.sort_by_cached_key(|entry| (entry.kind != Kind::Folder, normalize(&entry.name), entry.name.clone()));
    Ok(entries)
}

/// "1.4 MB"
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1000.0;
    let mut unit = 0;
    while size >= 1000.0 && unit + 1 < UNITS.len() {
        size /= 1000.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::tests::Scratch;

    #[test]
    fn folders_come_first_and_dot_files_stay_hidden() {
        let scratch = Scratch::new("list");
        fs::write(scratch.0.join("alpha.png"), "x").unwrap();
        fs::write(scratch.0.join(".secret"), "x").unwrap();
        fs::create_dir(scratch.0.join("zeta")).unwrap();
        let names =
            |hidden| -> Vec<String> { list(&scratch.0, hidden).unwrap().into_iter().map(|entry| entry.name).collect() };
        assert_eq!(names(false), ["zeta", "alpha.png"]);
        assert_eq!(names(true), ["zeta", ".secret", "alpha.png"]);
        assert_eq!(list(&scratch.0, false).unwrap()[1].kind, Kind::Image);
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1_400_000), "1.4 MB");
        assert_eq!(human_size(3_000_000_000), "3.0 GB");
    }

    #[test]
    fn kinds_ignore_case() {
        assert_eq!(Kind::of("Song.FLAC"), Kind::Audio);
        assert_eq!(Kind::of("noextension"), Kind::Other);
    }
}
