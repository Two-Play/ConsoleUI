//! What can happen to files, and where the sidebar leads.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use chrono::Local;
use consoleui_core::i18n::tr;
use consoleui_core::xdg;

fn message(err: io::Error) -> String {
    match err.kind() {
        io::ErrorKind::PermissionDenied => tr("You may not change this.").to_string(),
        io::ErrorKind::StorageFull => tr("There is no space left.").to_string(),
        _ => err.to_string(),
    }
}

/// A valid name of a single file.
fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(tr("A name cannot contain a slash.").to_string());
    }
    Ok(())
}

/// A path in `dir` that is free: "a.txt", then "a (2).txt", ...
fn free_path(dir: &Path, name: &str) -> PathBuf {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    let mut path = dir.join(name);
    let mut number = 2;
    while path.symlink_metadata().is_ok() {
        path = dir.join(format!("{stem} ({number}){extension}"));
        number += 1;
    }
    path
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let data = fs::symlink_metadata(from)?;
    if data.is_symlink() {
        symlink(fs::read_link(from)?, to)
    } else if data.is_dir() {
        fs::create_dir(to)?;
        for item in fs::read_dir(from)? {
            let item = item?;
            copy_tree(&item.path(), &to.join(item.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(|_| ())
    }
}

fn file_name(path: &Path) -> Result<String, String> {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).ok_or_else(|| tr("Not possible here.").to_string())
}

fn check_target(from: &Path, into: &Path) -> Result<(), String> {
    if into.starts_with(from) {
        return Err(tr("A folder cannot go into itself.").to_string());
    }
    Ok(())
}

/// Copies into a folder; a name that is taken gets a number.
pub fn copy(from: &Path, into: &Path) -> Result<(), String> {
    check_target(from, into)?;
    let target = free_path(into, &file_name(from)?);
    copy_tree(from, &target).map_err(|err| {
        // No half copy stays behind
        let _ = fs::remove_dir_all(&target).or_else(|_| fs::remove_file(&target));
        message(err)
    })
}

/// Moves into a folder, across drives too.
pub fn move_to(from: &Path, into: &Path) -> Result<(), String> {
    check_target(from, into)?;
    if from.parent() == Some(into) {
        return Ok(());
    }
    let target = free_path(into, &file_name(from)?);
    if fs::rename(from, &target).is_ok() {
        return Ok(());
    }
    copy(from, into)?;
    let data = fs::symlink_metadata(from).map_err(message)?;
    let removed = if data.is_dir() { fs::remove_dir_all(from) } else { fs::remove_file(from) };
    removed.map_err(message)
}

pub fn rename(path: &Path, name: &str) -> Result<(), String> {
    check_name(name)?;
    let target = path.with_file_name(name);
    if target == path {
        return Ok(());
    }
    if target.symlink_metadata().is_ok() {
        return Err(tr("That name is already taken.").to_string());
    }
    fs::rename(path, target).map_err(message)
}

pub fn new_folder(dir: &Path, name: &str) -> Result<(), String> {
    check_name(name)?;
    fs::create_dir(dir.join(name)).map_err(|err| match err.kind() {
        io::ErrorKind::AlreadyExists => tr("That name is already taken.").to_string(),
        _ => message(err),
    })
}

fn encode(path: &Path) -> String {
    let mut out = String::new();
    for &byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Moves to the user's trash as the Freedesktop specification says, so a desktop can restore it.
pub fn trash(path: &Path, trash_dir: &Path) -> Result<(), String> {
    let (files, info) = (trash_dir.join("files"), trash_dir.join("info"));
    fs::create_dir_all(&files).map_err(message)?;
    fs::create_dir_all(&info).map_err(message)?;
    let target = free_path(&files, &file_name(path)?);
    let name = file_name(&target)?;
    let info_path = info.join(format!("{name}.trashinfo"));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&info_path).map_err(message)?;
    let date = Local::now().format("%Y-%m-%dT%H:%M:%S");
    let text = format!("[Trash Info]\nPath={}\nDeletionDate={date}\n", encode(path));
    file.write_all(text.as_bytes()).map_err(message)?;
    fs::rename(path, &target).map_err(|err| {
        let _ = fs::remove_file(&info_path);
        match err.kind() {
            io::ErrorKind::CrossesDevices => tr("This drive has no trash.").to_string(),
            _ => message(err),
        }
    })
}

pub fn home_trash() -> Option<PathBuf> {
    xdg::data_home().map(|dir| dir.join("Trash"))
}

/// An entry of the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
}

/// Reads `XDG_DOCUMENTS_DIR="$HOME/Dokumente"` and its kin.
fn user_dirs(text: &str, home: &Path) -> Vec<PathBuf> {
    let wanted = ["DOWNLOAD", "DOCUMENTS", "PICTURES", "MUSIC", "VIDEOS"];
    wanted.iter().filter_map(|key| xdg::parse_user_dir(text, home, key)).filter(|path| path != home).collect()
}

/// Home, the user's folders that exist, then the drives mounted for the user.
pub fn places() -> Vec<Place> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    let mut places = vec![Place { name: tr("Home").to_string(), path: home.clone() }];
    let config = xdg::config_home().map(|dir| dir.join("user-dirs.dirs"));
    let text = config.and_then(|path| fs::read_to_string(path).ok()).unwrap_or_default();
    let dirs = user_dirs(&text, &home).into_iter().filter(|path| path.is_dir());
    let named = |path: PathBuf| file_name(&path).ok().map(|name| Place { name, path });
    places.extend(dirs.filter_map(named));
    let user = std::env::var("USER").unwrap_or_default();
    let drives = fs::read_dir(Path::new("/run/media").join(user)).into_iter().flatten().flatten();
    places.extend(drives.map(|item| item.path()).filter_map(named));
    places
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A directory that goes away with the test.
    pub struct Scratch(pub PathBuf);

    impl Scratch {
        pub fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("consoleui-files-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn copy_keeps_both_and_numbers_the_new_one() {
        let scratch = Scratch::new("copy");
        let file = scratch.0.join("a.txt");
        fs::write(&file, "x").unwrap();
        copy(&file, &scratch.0).unwrap();
        assert_eq!(fs::read_to_string(scratch.0.join("a (2).txt")).unwrap(), "x");
        assert!(file.exists());
    }

    #[test]
    fn folders_copy_with_their_content_and_links_stay_links() {
        let scratch = Scratch::new("tree");
        let folder = scratch.0.join("folder");
        fs::create_dir_all(folder.join("inner")).unwrap();
        fs::write(folder.join("inner/a"), "x").unwrap();
        symlink("inner/a", folder.join("link")).unwrap();
        let target = scratch.0.join("target");
        fs::create_dir(&target).unwrap();
        copy(&folder, &target).unwrap();
        assert!(target.join("folder/inner/a").exists());
        assert!(fs::symlink_metadata(target.join("folder/link")).unwrap().is_symlink());
    }

    #[test]
    fn a_folder_cannot_go_into_itself() {
        let scratch = Scratch::new("self");
        let folder = scratch.0.join("folder");
        fs::create_dir_all(folder.join("inner")).unwrap();
        assert!(copy(&folder, &folder.join("inner")).is_err());
        assert!(move_to(&folder, &folder).is_err());
    }

    #[test]
    fn moving_does_not_overwrite() {
        let scratch = Scratch::new("move");
        let (a, b) = (scratch.0.join("a"), scratch.0.join("b"));
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        fs::write(a.join("f"), "new").unwrap();
        fs::write(b.join("f"), "old").unwrap();
        move_to(&a.join("f"), &b).unwrap();
        assert!(!a.join("f").exists());
        assert_eq!(fs::read_to_string(b.join("f")).unwrap(), "old");
        assert_eq!(fs::read_to_string(b.join("f (2)")).unwrap(), "new");
    }

    #[test]
    fn rename_refuses_a_taken_name_and_slashes() {
        let scratch = Scratch::new("rename");
        let (a, b) = (scratch.0.join("a"), scratch.0.join("b"));
        fs::write(&a, "1").unwrap();
        fs::write(&b, "2").unwrap();
        assert!(rename(&a, "b").is_err());
        assert!(rename(&a, "x/y").is_err());
        assert_eq!(fs::read_to_string(&b).unwrap(), "2");
        rename(&a, "c").unwrap();
        assert!(scratch.0.join("c").exists());
    }

    #[test]
    fn trash_follows_the_specification() {
        let scratch = Scratch::new("trash");
        let file = scratch.0.join("a b.txt");
        fs::write(&file, "x").unwrap();
        let bin = scratch.0.join("Trash");
        trash(&file, &bin).unwrap();
        assert!(!file.exists());
        assert!(bin.join("files/a b.txt").exists());
        let info = fs::read_to_string(bin.join("info/a b.txt.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath=/"));
        assert!(info.contains("a%20b.txt\nDeletionDate=20"));
        fs::write(&file, "y").unwrap();
        trash(&file, &bin).unwrap();
        assert!(bin.join("files/a b (2).txt").exists(), "a second one with the same name");
    }

    #[test]
    fn user_dirs_keep_their_localized_paths() {
        let text = "# comment\nXDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"\nXDG_MUSIC_DIR=\"$HOME/\"\nXDG_VIDEOS_DIR=\"/data/Filme\"\n";
        let home = Path::new("/home/me");
        assert_eq!(user_dirs(text, home), [PathBuf::from("/home/me/Dokumente"), PathBuf::from("/data/Filme")]);
    }
}
