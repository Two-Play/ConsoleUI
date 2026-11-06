//! The real backend: the `flatpak` program and the catalog it keeps on disk. Apps go into the
//! user's installation, which needs no administrator rights on a console without a password prompt.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, SystemTime};

use consoleui_core::i18n::{self, Language};
use flate2::read::GzDecoder;

use crate::appstream::{self, Listing};
use crate::backend::Backend;

const REMOTE: &str = "flathub";
const REMOTE_FILE: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";
/// The catalog is fetched again after this time, for the next start.
const MAX_AGE: Duration = Duration::from_secs(24 * 3600);

pub struct Flatpak;

fn flatpak(args: &[&str]) -> Command {
    let mut command = Command::new("flatpak");
    // The output is parsed
    command.args(args).env("LC_ALL", "C").stdin(Stdio::null());
    command
}

fn run(args: &[&str]) -> Result<Output, String> {
    let output = flatpak(args).output().map_err(|err| format!("flatpak: {err}"))?;
    if output.status.success() {
        return Ok(output);
    }
    Err(last_line(&String::from_utf8_lossy(&output.stderr)))
}

fn last_line(text: &str) -> String {
    let line = text.lines().rfind(|line| !line.trim().is_empty()).unwrap_or("flatpak failed");
    line.trim().trim_start_matches("error: ").to_string()
}

fn ensure_remote() -> Result<(), String> {
    run(&["remote-add", "--user", "--if-not-exists", REMOTE, REMOTE_FILE]).map(|_| ())
}

fn fetch_catalog() -> Result<(), String> {
    ensure_remote()?;
    run(&["update", "--user", "--appstream", REMOTE]).map(|_| ())
}

/// The directories that can hold the catalog, the user's first.
fn catalog_dirs() -> Vec<PathBuf> {
    let user = std::env::var_os("FLATPAK_USER_DIR")
        .map(PathBuf::from)
        .or_else(|| consoleui_core::xdg::data_home().map(|home| home.join("flatpak")));
    let tail = format!("appstream/{REMOTE}/{}/active", std::env::consts::ARCH);
    user.into_iter().chain([PathBuf::from("/var/lib/flatpak")]).map(|dir| dir.join(&tail)).collect()
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|data| data.modified()).ok()
}

/// The newest catalog on disk with its age: flatpak keeps one per installation.
fn newest_catalog() -> Option<(PathBuf, SystemTime)> {
    catalog_dirs()
        .into_iter()
        .flat_map(|dir| [dir.join("appstream.xml"), dir.join("appstream.xml.gz")])
        .filter_map(|path| modified(&path).map(|time| (path, time)))
        .max_by_key(|(_, time)| *time)
}

fn read_catalog(path: &Path) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut xml = Vec::new();
    let read = if path.extension().is_some_and(|e| e == "gz") {
        GzDecoder::new(BufReader::new(file)).read_to_end(&mut xml)
    } else {
        BufReader::new(file).read_to_end(&mut xml)
    };
    read.map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(xml)
}

fn installed_apps() -> HashSet<String> {
    let output = run(&["list", "--app", "--columns=application"]);
    let text = output.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    text.lines().map(|line| line.trim().to_string()).filter(|line| !line.is_empty()).collect()
}

/// The share of a whole install in a line of flatpak's output: `Installing 2/3… ███   40%`.
fn parse_progress(line: &str) -> Option<u8> {
    let percent = line.rsplit_once('%')?.0;
    let digits = percent.rfind(|c: char| !c.is_ascii_digit()).map_or(0, |i| i + 1);
    let percent: u32 = percent[digits..].parse().ok()?;
    let step = line.split_whitespace().find_map(|word| {
        let (done, total) = word.trim_end_matches('…').split_once('/')?;
        Some((done.parse::<u32>().ok()?, total.parse::<u32>().ok()?))
    });
    let (step, total) = step.filter(|&(step, total)| step >= 1 && step <= total).unwrap_or((1, 1));
    Some((((step - 1) * 100 + percent.min(100)) / total) as u8)
}

impl Backend for Flatpak {
    fn load(&mut self) -> Result<Vec<Listing>, String> {
        if newest_catalog().is_none() {
            fetch_catalog()?;
        }
        let (path, time) = newest_catalog().ok_or("Flatpak has no catalog of Flathub.")?;
        if time.elapsed().is_ok_and(|age| age > MAX_AGE) {
            std::thread::spawn(|| {
                if let Err(err) = fetch_catalog() {
                    eprintln!("Cannot update the catalog: {err}");
                }
            });
        }
        let xml = read_catalog(&path)?;
        // Next to `appstream.xml`
        let icons = path.with_file_name("icons");
        let language = if i18n::language() == Language::German { "de" } else { "" };
        let mut listings = appstream::parse(&xml, &icons, language);
        let installed = installed_apps();
        for listing in &mut listings {
            listing.installed = installed.contains(&listing.id);
            listing.icon = listing.icon.take().filter(|icon| icon.exists());
        }
        Ok(listings)
    }

    fn install(&mut self, id: &str, progress: &mut dyn FnMut(Option<u8>)) -> Result<(), String> {
        ensure_remote()?;
        let mut child = flatpak(&["install", "--user", "-y", "--or-update", REMOTE, id])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("flatpak: {err}"))?;
        // stderr is small; it is read after the end
        let mut stdout = child.stdout.take().ok_or("flatpak has no output")?;
        let mut stderr = child.stderr.take().ok_or("flatpak has no output")?;
        let errors = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        let (mut buffer, mut line) = ([0u8; 4096], Vec::new());
        while let Ok(read) = stdout.read(&mut buffer) {
            if read == 0 {
                break;
            }
            for &byte in &buffer[..read] {
                if byte == b'\n' || byte == b'\r' {
                    if let Some(percent) = parse_progress(&String::from_utf8_lossy(&line)) {
                        progress(Some(percent));
                    }
                    line.clear();
                } else {
                    line.push(byte);
                }
            }
        }
        let status = child.wait().map_err(|err| format!("flatpak: {err}"))?;
        let errors = errors.join().unwrap_or_default();
        if status.success() { Ok(()) } else { Err(last_line(&errors)) }
    }

    fn remove(&mut self, id: &str) -> Result<(), String> {
        run(&["uninstall", "-y", "--noninteractive", id]).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_counts_all_steps() {
        assert_eq!(parse_progress("Installing… ███         37%"), Some(37));
        assert_eq!(parse_progress("Installing 2/4… ██████  50%  1,2 MB/s"), Some(37));
        assert_eq!(parse_progress("Installing 4/4… ██████ 100%"), Some(100));
        assert_eq!(parse_progress("Installation complete."), None);
    }

    /// Downloads about 1 MB. Run it with a scratch installation:
    /// `FLATPAK_USER_DIR=/tmp/x cargo test -p consoleui-store -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the network and FLATPAK_USER_DIR"]
    fn installs_and_removes_for_real() {
        assert!(std::env::var_os("FLATPAK_USER_DIR").is_some(), "this must not touch the real installation");
        let id = "org.freedesktop.Platform.openh264//2.5.1";
        let mut reports = Vec::new();
        Flatpak.install(id, &mut |percent| reports.push(percent)).expect("install");
        println!("progress: {reports:?}");
        assert!(!reports.is_empty(), "flatpak reported no progress");
        Flatpak.remove(id).expect("remove");
        assert!(Flatpak.install("org.example.DoesNotExist", &mut |_| {}).is_err());
    }

    #[test]
    fn errors_are_the_last_line() {
        assert_eq!(last_line("Note: x\nerror: No remote refs found\n\n"), "No remote refs found");
    }
}
