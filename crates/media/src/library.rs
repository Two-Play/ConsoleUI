//! What can be played: the user's music and video folders and the drives mounted for the user.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use consoleui_core::search::normalize;
use consoleui_core::xdg;

/// A "play everything" stops here, so a whole drive cannot make a queue without end.
const QUEUE_LIMIT: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Medium {
    Music,
    Videos,
}

impl Medium {
    pub fn plays(self, name: &str) -> bool {
        let extension = name.rsplit_once('.').map(|(_, extension)| extension.to_lowercase()).unwrap_or_default();
        let known: &[&str] = match self {
            Self::Music => &["mp3", "flac", "ogg", "oga", "opus", "wav", "m4a", "aac", "wma", "ape"],
            Self::Videos => &["mp4", "mkv", "webm", "avi", "mov", "m4v", "mpg", "mpeg", "ts", "wmv", "ogv"],
        };
        known.contains(&extension.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub path: PathBuf,
    pub folder: bool,
}

fn item(path: PathBuf, folder: bool) -> Item {
    let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned();
    Item { name, path, folder }
}

fn sorted(mut items: Vec<Item>) -> Vec<Item> {
    items.sort_by_cached_key(|item| (!item.folder, normalize(&item.name), item.name.clone()));
    items
}

/// The folders and playable files in `dir`: folders first, then by name, dot files left out.
pub fn list(dir: &Path, medium: Medium) -> io::Result<Vec<Item>> {
    let mut items = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            items.push(item(path, true));
        } else if medium.plays(&name) {
            items.push(item(path, false));
        }
    }
    Ok(sorted(items))
}

/// What the top of a tab shows: the content of its first folder, the others as folders in it.
pub fn list_top(roots: &[PathBuf], medium: Medium) -> io::Result<Vec<Item>> {
    let Some((first, others)) = roots.split_first() else { return Ok(Vec::new()) };
    // The user may not have that folder, the drives still count
    let mut items = match list(first, medium) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
        listed => listed.unwrap_or_default(),
    };
    items.extend(others.iter().map(|path| item(path.clone(), true)));
    Ok(sorted(items))
}

/// Every playable file in the folders and below, in the order of the lists.
pub fn everything(dirs: &[PathBuf], medium: Medium) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut open: Vec<PathBuf> = dirs.iter().rev().cloned().collect();
    while let Some(dir) = open.pop() {
        if files.len() >= QUEUE_LIMIT {
            break;
        }
        let items = list(&dir, medium).unwrap_or_default();
        // Files before folders, as listed; `open` is a stack
        let (folders, tracks): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|item| item.folder);
        files.extend(tracks.into_iter().map(|item| item.path));
        open.extend(folders.into_iter().rev().map(|item| item.path));
    }
    files.truncate(QUEUE_LIMIT);
    files
}

/// The folders at the top of a tab: the user's folder, then the drives.
pub fn roots(medium: Medium) -> Vec<PathBuf> {
    let (key, fallback) = match medium {
        Medium::Music => ("MUSIC", "Music"),
        Medium::Videos => ("VIDEOS", "Videos"),
    };
    let own = xdg::user_dir(key).or_else(|| xdg::home().map(|home| home.join(fallback)));
    let user = std::env::var("USER").unwrap_or_default();
    let drives = fs::read_dir(Path::new("/run/media").join(user)).into_iter().flatten().flatten();
    own.into_iter().chain(drives.map(|entry| entry.path())).collect()
}

/// The picture of an album: `cover.jpg`, `folder.png` and the like next to the track.
pub fn cover(track: &Path) -> Option<PathBuf> {
    let mut found: Vec<(usize, PathBuf)> = fs::read_dir(track.parent()?)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let (stem, extension) = (path.file_stem()?.to_str()?, path.extension()?.to_str()?);
            let (stem, extension) = (stem.to_lowercase(), extension.to_lowercase());
            let rank = ["cover", "folder", "front", "album"].iter().position(|name| *name == stem)?;
            ["jpg", "jpeg", "png"].contains(&extension.as_str()).then_some((rank, path))
        })
        .collect();
    found.sort();
    found.into_iter().next().map(|(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("consoleui-media-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Album")).unwrap();
        for file in ["b.mp3", "a.FLAC", "film.mkv", "notes.txt", ".hidden.mp3", "Album/1.ogg", "Album/Cover.JPG"] {
            fs::write(dir.join(file), "").unwrap();
        }
        dir
    }

    fn names(items: &[Item]) -> Vec<&str> {
        items.iter().map(|item| item.name.as_str()).collect()
    }

    #[test]
    fn lists_only_what_the_tab_plays() {
        let dir = scratch("list");
        assert_eq!(names(&list(&dir, Medium::Music).unwrap()), ["Album", "a.FLAC", "b.mp3"]);
        assert_eq!(names(&list(&dir, Medium::Videos).unwrap()), ["Album", "film.mkv"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn everything_goes_down_in_list_order() {
        let dir = scratch("all");
        let files = everything(std::slice::from_ref(&dir), Medium::Music);
        let expected = [dir.join("a.FLAC"), dir.join("b.mp3"), dir.join("Album/1.ogg")];
        assert_eq!(files, expected);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_top_shows_drives_as_folders_even_without_the_own_folder() {
        let dir = scratch("top");
        let roots = [dir.join("gone"), dir.join("Album")];
        let top = list_top(&roots, Medium::Music).unwrap();
        assert_eq!(names(&top), ["Album"]);
        assert!(top[0].folder);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_the_cover_next_to_the_track() {
        let dir = scratch("cover");
        assert_eq!(cover(&dir.join("Album/1.ogg")), Some(dir.join("Album/Cover.JPG")));
        assert_eq!(cover(&dir.join("b.mp3")), None);
        fs::remove_dir_all(dir).unwrap();
    }
}
