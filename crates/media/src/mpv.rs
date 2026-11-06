//! mpv's side of the player without a process: its command line, the lines for its JSON IPC and
//! what its answers mean. `man mpv`, "JSON IPC".

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::library::Medium;

/// What to play: `files`, beginning with the one at `start`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    pub medium: Medium,
    pub files: Vec<PathBuf>,
    pub start: usize,
}

/// The properties mpv reports on every change.
const OBSERVED: [&str; 7] =
    ["pause", "time-pos", "duration", "media-title", "metadata", "playlist-pos", "playlist-count"];

/// The path is reported on its own: the cover depends on it.
pub fn observe_lines() -> Vec<String> {
    let names = OBSERVED.iter().chain(&["path"]);
    names.enumerate().map(|(id, name)| format!(r#"{{"command":["observe_property",{id},"{name}"]}}"#)).collect()
}

/// `positions` is where videos go on from next time.
pub fn arguments(queue: &Queue, socket: &Path, positions: Option<&Path>) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["--no-terminal", "--idle=no", "--keep-open=no"].map(OsString::from).to_vec();
    args.push(format!("--playlist-start={}", queue.start).into());
    let mut socket_arg = OsString::from("--input-ipc-server=");
    socket_arg.push(socket);
    args.push(socket_arg);
    match queue.medium {
        Medium::Music => {
            args.extend(["--no-video", "--no-resume-playback", "--no-save-position-on-quit"].map(OsString::from))
        }
        Medium::Videos => {
            args.extend(["--fullscreen", "--force-window=yes", "--save-position-on-quit"].map(OsString::from));
            if let Some(dir) = positions {
                let mut arg = OsString::from("--watch-later-dir=");
                arg.push(dir);
                args.push(arg);
            }
        }
    }
    // A file name that starts with a dash is no option
    args.push("--".into());
    args.extend(queue.files.iter().map(|file| file.as_os_str().to_owned()));
    args
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    PlayPause,
    Seek(i32),
    Restart,
    Previous,
    Next,
    Shuffle(bool),
    Repeat(Repeat),
    CycleSubtitles,
    CycleAudio,
    ShowProgress,
    Message(String),
}

/// mpv takes commands in the syntax of `input.conf` as plain lines; only those show on its OSD.
pub fn line(control: &Control) -> String {
    match control {
        Control::PlayPause => "osd-msg-bar cycle pause".into(),
        Control::Seek(seconds) => format!("osd-msg-bar seek {seconds}"),
        Control::Restart => "seek 0 absolute".into(),
        Control::Previous => "playlist-prev".into(),
        Control::Next => "playlist-next".into(),
        Control::Shuffle(true) => "playlist-shuffle".into(),
        Control::Shuffle(false) => "playlist-unshuffle".into(),
        Control::Repeat(Repeat::Off) => "set loop-playlist no; set loop-file no".into(),
        Control::Repeat(Repeat::All) => "set loop-playlist inf; set loop-file no".into(),
        Control::Repeat(Repeat::One) => "set loop-playlist no; set loop-file inf".into(),
        Control::CycleSubtitles => "osd-msg cycle sub".into(),
        Control::CycleAudio => "osd-msg cycle audio".into(),
        Control::ShowProgress => "show-progress".into(),
        Control::Message(text) => {
            let text = text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " ");
            format!("show-text \"{text}\" 6000")
        }
    }
}

/// Ends mpv; with `--save-position-on-quit` a video remembers where it stopped.
pub const QUIT: &str = "quit";

#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Paused(bool),
    Time(f64),
    Duration(f64),
    Title(String),
    Tags { title: Option<String>, artist: String, album: String },
    Index(usize),
    Count(usize),
    Path(PathBuf),
}

/// mpv keeps the spelling of the file: `ARTIST` in a FLAC, `artist` in an MP3.
fn tag(tags: &serde_json::Map<String, Value>, name: &str) -> Option<String> {
    let found = tags.iter().find(|(key, _)| key.eq_ignore_ascii_case(name))?.1;
    found.as_str().map(str::to_string).filter(|text| !text.is_empty())
}

/// One line from mpv; `None` for answers and events the player does not need.
pub fn parse(line: &str) -> Option<Update> {
    let message: Value = serde_json::from_str(line).ok()?;
    if message.get("event")?.as_str()? != "property-change" {
        return None;
    }
    let data = message.get("data")?;
    Some(match message.get("name")?.as_str()? {
        "pause" => Update::Paused(data.as_bool()?),
        "time-pos" => Update::Time(data.as_f64()?),
        "duration" => Update::Duration(data.as_f64()?),
        "media-title" => Update::Title(data.as_str()?.to_string()),
        "metadata" => {
            let tags = data.as_object()?;
            Update::Tags {
                title: tag(tags, "title"),
                artist: tag(tags, "artist").unwrap_or_default(),
                album: tag(tags, "album").unwrap_or_default(),
            }
        }
        "playlist-pos" => Update::Index(usize::try_from(data.as_i64()?).ok()?),
        "playlist-count" => Update::Count(usize::try_from(data.as_i64()?).ok()?),
        "path" => Update::Path(PathBuf::from(data.as_str()?)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(name: &str, data: &str) -> Option<Update> {
        parse(&format!(r#"{{"event":"property-change","id":1,"name":"{name}","data":{data}}}"#))
    }

    #[test]
    fn reads_the_properties() {
        assert_eq!(change("pause", "true"), Some(Update::Paused(true)));
        assert_eq!(change("time-pos", "12.5"), Some(Update::Time(12.5)));
        assert_eq!(change("playlist-pos", "2"), Some(Update::Index(2)));
        assert_eq!(change("path", r#""/m/a.flac""#), Some(Update::Path("/m/a.flac".into())));
    }

    #[test]
    fn tags_do_not_care_about_case() {
        let update = change("metadata", r#"{"ARTIST":"Band","Album":"Record","title":"Song"}"#);
        let expected = Update::Tags { title: Some("Song".into()), artist: "Band".into(), album: "Record".into() };
        assert_eq!(update, Some(expected));
    }

    #[test]
    fn skips_what_is_no_change() {
        assert_eq!(parse(r#"{"data":null,"request_id":0,"error":"success"}"#), None);
        assert_eq!(parse(r#"{"event":"file-loaded"}"#), None);
        assert_eq!(parse("not json"), None);
        // No value yet, between two files
        assert_eq!(parse(r#"{"event":"property-change","id":1,"name":"time-pos"}"#), None);
        assert_eq!(change("playlist-pos", "-1"), None);
    }

    #[test]
    fn a_file_with_a_dash_stays_a_file() {
        let queue = Queue { medium: Medium::Music, files: vec!["-x.mp3".into()], start: 0 };
        let args = arguments(&queue, Path::new("/s"), None);
        let dashes = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(args[dashes + 1], "-x.mp3");
        assert!(args.iter().any(|arg| arg == "--no-video"));
    }

    #[test]
    fn videos_remember_where_they_stopped() {
        let queue = Queue { medium: Medium::Videos, files: vec!["/f.mkv".into()], start: 0 };
        let args = arguments(&queue, Path::new("/s"), Some(Path::new("/p")));
        assert!(args.iter().any(|arg| arg == "--watch-later-dir=/p"));
        assert!(!args.iter().any(|arg| arg == "--no-video"));
    }

    #[test]
    fn a_message_cannot_break_out_of_its_quotes() {
        assert_eq!(line(&Control::Message("a \"b\"".into())), r#"show-text "a \"b\"" 6000"#);
    }
}
