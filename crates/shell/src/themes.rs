//! Fetches theme music with `yt-dlp` on a thread of its own (see `consoleui_core::themes`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};

use consoleui_core::themes;

struct Job {
    query: String,
    result: u32,
    file: PathBuf,
}

pub struct Fetcher {
    jobs: Sender<Job>,
    done: Receiver<PathBuf>,
    busy: HashSet<PathBuf>,
}

impl Fetcher {
    pub fn start() -> Self {
        let (jobs, incoming) = mpsc::channel::<Job>();
        let (finished, done) = mpsc::channel();
        std::thread::spawn(move || {
            for job in incoming {
                fetch(&job);
                if finished.send(job.file).is_err() {
                    return;
                }
            }
        });
        Self { jobs, done, busy: HashSet::new() }
    }

    /// Asks for the file once; [`Self::poll`] says when that is over (found or not).
    pub fn request(&mut self, title: &str, result: u32, file: &Path) {
        if self.busy.insert(file.to_path_buf()) {
            let job = Job { query: themes::search_query(title), result, file: file.to_path_buf() };
            let _ = self.jobs.send(job);
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

fn fetch(job: &Job) {
    if let Some(dir) = job.file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let status = Command::new("yt-dlp")
        .args(themes::ytdlp_args(&job.query, job.result, &job.file))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
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
