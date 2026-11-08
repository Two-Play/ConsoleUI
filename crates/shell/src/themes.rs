//! Fetches theme music with `yt-dlp` on a thread of its own (see `consoleui_core::themes`).
//!
//! Only the newest request waits: scrolling past games does not queue a search for each of them.
//! `yt-dlp` (and the `ffmpeg` it runs) work at the lowest CPU priority, so the UI stays smooth.

use std::collections::HashSet;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};

use consoleui_core::themes;

struct Job {
    query: String,
    result: u32,
    file: PathBuf,
}

#[derive(Default)]
struct Waiting {
    job: Option<Job>,
    closed: bool,
}

pub struct Fetcher {
    waiting: Arc<(Mutex<Waiting>, Condvar)>,
    done: Receiver<PathBuf>,
    /// Asked for and not finished (or replaced) yet
    busy: HashSet<PathBuf>,
}

impl Fetcher {
    pub fn start() -> Self {
        let waiting: Arc<(Mutex<Waiting>, Condvar)> = Arc::default();
        let (finished, done) = mpsc::channel();
        let shared = waiting.clone();
        std::thread::spawn(move || {
            loop {
                let job = {
                    let (lock, ready) = &*shared;
                    let Ok(mut waiting) = lock.lock() else { return };
                    loop {
                        if waiting.closed {
                            return;
                        }
                        if let Some(job) = waiting.job.take() {
                            break job;
                        }
                        let Ok(next) = ready.wait(waiting) else { return };
                        waiting = next;
                    }
                };
                fetch(&job);
                if finished.send(job.file).is_err() {
                    return;
                }
            }
        });
        Self { waiting, done, busy: HashSet::new() }
    }

    /// Searches for the file next; a request still waiting is dropped. [`Self::poll`] says when
    /// a search is over (found or not).
    pub fn request(&mut self, title: &str, result: u32, file: &Path) {
        if self.busy.contains(file) {
            return;
        }
        let job = Job { query: themes::search_query(title), result, file: file.to_path_buf() };
        let (lock, ready) = &*self.waiting;
        if let Ok(mut waiting) = lock.lock() {
            if let Some(old) = waiting.job.replace(job) {
                self.busy.remove(&old.file);
            }
            self.busy.insert(file.to_path_buf());
            ready.notify_one();
        }
    }

    /// Whether a search finished since the last call.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        for file in self.done.try_iter() {
            self.busy.remove(&file);
            any = true;
        }
        any
    }
}

impl Drop for Fetcher {
    fn drop(&mut self) {
        let (lock, ready) = &*self.waiting;
        if let Ok(mut waiting) = lock.lock() {
            waiting.closed = true;
            ready.notify_one();
        }
    }
}

/// Runs a helper at the lowest CPU priority (`nice 19`); its children inherit it.
pub fn low_priority(command: &mut Command) -> &mut Command {
    // Safety: setpriority(2) only, which is async-signal-safe, between fork and exec.
    unsafe {
        command.pre_exec(|| {
            libc::setpriority(libc::PRIO_PROCESS, 0, 19);
            Ok(())
        })
    }
}

fn fetch(job: &Job) {
    if let Some(dir) = job.file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut command = Command::new("yt-dlp");
    command.args(themes::ytdlp_args(&job.query, job.result, &job.file));
    let status = low_priority(&mut command).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    match status {
        Ok(status) if status.success() && job.file.is_file() => {}
        Ok(_) => {
            eprintln!("No theme music found for \"{}\" (result {})", job.query, job.result + 1);
            let _ = std::fs::write(themes::failed_marker(&job.file), "");
        }
        // Not installed: nothing to mark, it may be installed later
        Err(err) => eprintln!("Theme music needs yt-dlp: {err}"),
    }
}
