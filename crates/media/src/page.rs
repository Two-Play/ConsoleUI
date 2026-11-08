//! The state of the app without Slint and without mpv: two tabs of folders, and what plays.

use std::path::{Path, PathBuf};

use consoleui_core::Action;
use consoleui_core::dialog::{Dialog, DialogOutcome};
use consoleui_core::i18n::{tr, trf};
use consoleui_core::input::GlyphStyle;
use consoleui_core::media::MediaCommand;
use consoleui_sdk::Sound;

use crate::library::{Item, Medium};
use crate::mpris::Shown;
use crate::mpv::{Control, Queue, Repeat, Update};

/// Rows the list shows at once.
pub const VISIBLE: usize = 9;
/// Seconds per press.
const SEEK: i32 = 10;
const SEEK_FAR: i32 = 60;
/// After this many seconds "previous" goes to the start of the track, as on any music player.
const RESTART_AFTER: f64 = 3.0;

#[derive(Debug, PartialEq)]
pub enum Request {
    /// The folders of `tab`, or the folder `dir` in it
    List { tab: usize, dir: Option<PathBuf> },
    /// Everything below these folders, as a queue
    Everything { medium: Medium, dirs: Vec<PathBuf> },
}

#[derive(Debug)]
pub enum Response {
    Listed { tab: usize, dir: Option<PathBuf>, items: Result<Vec<Item>, String> },
    Everything { medium: Medium, files: Vec<PathBuf> },
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    Changed(Sound),
    Send(Request),
    Play(Queue),
    Control(Control),
    Stop,
    Quit,
}

/// A folder that is open, and where the focus was in the one before.
struct Level {
    dir: PathBuf,
    focus_before: usize,
}

pub struct Tab {
    pub medium: Medium,
    roots: Vec<PathBuf>,
    levels: Vec<Level>,
    pub items: Vec<Item>,
    pub focused: usize,
    top: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl Tab {
    fn new(medium: Medium, roots: Vec<PathBuf>) -> Self {
        Self { medium, roots, levels: Vec::new(), items: Vec::new(), focused: 0, top: 0, loading: true, error: None }
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn dir(&self) -> Option<&Path> {
        self.levels.last().map(|level| level.dir.as_path())
    }

    pub fn visible(&self) -> &[Item] {
        let end = (self.top + VISIBLE).min(self.items.len());
        &self.items[self.top.min(end)..end]
    }

    pub fn visible_focus(&self) -> usize {
        self.focused.saturating_sub(self.top)
    }

    fn focus(&mut self, index: usize) {
        self.focused = index.min(self.items.len().saturating_sub(1));
        if self.focused < self.top {
            self.top = self.focused;
        } else if self.focused >= self.top + VISIBLE {
            self.top = self.focused + 1 - VISIBLE;
        }
    }

    /// The files of the open folder, and which of them is the focused one.
    fn queue(&self) -> Queue {
        let files = self.items.iter().filter(|item| !item.folder);
        let start = self.items[..self.focused].iter().filter(|item| !item.folder).count();
        Queue { medium: self.medium, files: files.map(|item| item.path.clone()).collect(), start }
    }
}

/// What mpv plays, as far as it told.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Playback {
    pub video: bool,
    pub paused: bool,
    pub position: f64,
    pub duration: f64,
    /// From the file's tags, or else what mpv makes of the file name
    pub title: String,
    tag_title: Option<String>,
    file_title: String,
    pub artist: String,
    pub album: String,
    pub index: usize,
    pub count: usize,
    pub path: Option<PathBuf>,
    pub shuffle: bool,
    pub repeat: Repeat,
    hinted: bool,
}

impl Playback {
    /// Whether something on the screen changes; the time only counts once per second.
    fn update(&mut self, update: Update) -> bool {
        match update {
            Update::Paused(paused) => self.paused = paused,
            Update::Time(time) => {
                let tick = time.floor() != self.position.floor();
                self.position = time;
                return tick;
            }
            Update::Duration(duration) => self.duration = duration,
            Update::Title(title) => self.file_title = title,
            Update::Tags { title, artist, album } => {
                self.tag_title = title;
                self.artist = artist;
                self.album = album;
            }
            Update::Index(index) => self.index = index,
            Update::Count(count) => self.count = count,
            Update::Path(path) => self.path = Some(path),
        }
        self.title = self.tag_title.clone().unwrap_or_else(|| self.file_title.clone());
        true
    }

    pub fn progress(&self) -> f32 {
        if self.duration > 0.0 { (self.position / self.duration) as f32 } else { 0.0 }
    }

    fn can_next(&self) -> bool {
        self.index + 1 < self.count || self.repeat == Repeat::All
    }

    fn previous(&self) -> Control {
        if self.position > RESTART_AFTER || self.index == 0 { Control::Restart } else { Control::Previous }
    }
}

/// "1:05", or "1:02:05" from one hour on.
pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

pub struct Page {
    pub tabs: [Tab; 2],
    pub tab: usize,
    pub playback: Option<Playback>,
    /// The screen of the track instead of the lists
    pub now_playing: bool,
    pub dialog: Option<Dialog<()>>,
    /// The controller's buttons, for the words on the video; `None` with a keyboard
    pub glyphs: Option<GlyphStyle>,
}

impl Page {
    pub fn new(music: Vec<PathBuf>, videos: Vec<PathBuf>) -> Self {
        Self {
            tabs: [Tab::new(Medium::Music, music), Tab::new(Medium::Videos, videos)],
            tab: 0,
            playback: None,
            now_playing: false,
            dialog: None,
            glyphs: None,
        }
    }

    pub fn current(&self) -> &Tab {
        &self.tabs[self.tab]
    }

    fn current_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.tab]
    }

    pub fn loading(&self) -> bool {
        self.tabs.iter().any(|tab| tab.loading)
    }

    /// A video's window lies over the app, which then has no focus but still gets the controller.
    pub fn video_playing(&self) -> bool {
        self.playback.as_ref().is_some_and(|playback| playback.video)
    }

    pub fn shown(&self) -> Shown {
        let Some(playback) = &self.playback else { return Shown::default() };
        Shown {
            loaded: true,
            playing: !playback.paused,
            title: playback.title.clone(),
            artist: playback.artist.clone(),
            album: playback.album.clone(),
            can_next: playback.can_next(),
            can_previous: true,
        }
    }

    fn list(&mut self) -> Request {
        let tab = self.tab;
        let current = self.current_mut();
        current.loading = true;
        current.error = None;
        Request::List { tab, dir: current.dir().map(Path::to_path_buf) }
    }

    pub fn listing(&mut self, tab: usize) -> Request {
        let dir = self.tabs[tab].dir().map(Path::to_path_buf);
        Request::List { tab, dir }
    }

    fn fail(&mut self, text: String) {
        self.dialog = Some(Dialog::alert(tr("That did not work"), text, tr("OK"), ()));
    }

    /// `mpv` could not be started.
    pub fn no_player(&mut self) {
        self.playback = None;
        self.now_playing = false;
        self.fail(tr("Playing needs the program mpv. Install it and try again.").to_string());
    }

    pub fn started(&mut self, queue: &Queue) {
        let video = queue.medium == Medium::Videos;
        self.playback = Some(Playback { video, count: queue.files.len(), index: queue.start, ..Playback::default() });
        self.now_playing = !video;
    }

    /// mpv ended, by itself or because it was told to.
    pub fn stopped(&mut self, failed: bool) {
        self.playback = None;
        self.now_playing = false;
        if failed {
            self.fail(tr("This file cannot be played.").to_string());
        }
    }

    /// An update from mpv: whether the screen changes, and what to tell mpv in return.
    pub fn update(&mut self, update: Update) -> (bool, Option<Control>) {
        let Some(playback) = self.playback.as_mut() else { return (false, None) };
        let first_video_frame = playback.video && !playback.hinted && matches!(update, Update::Duration(_));
        let changed = playback.update(update);
        let words = first_video_frame.then(|| self.glyphs.map(video_hint)).flatten();
        if let Some(playback) = self.playback.as_mut().filter(|_| first_video_frame) {
            playback.hinted = true;
        }
        (changed, words.map(Control::Message))
    }

    pub fn receive(&mut self, response: Response) -> Outcome {
        match response {
            Response::Listed { tab, dir, items } => {
                let tab = &mut self.tabs[tab];
                // An answer for a folder that was left in between
                if tab.dir() != dir.as_deref() {
                    return Outcome::Nothing;
                }
                tab.loading = false;
                match items {
                    Ok(items) => tab.items = items,
                    Err(err) => {
                        tab.items.clear();
                        tab.error = Some(err);
                    }
                }
                let focused = tab.focused;
                tab.focus(focused);
                Outcome::Changed(Sound::Move)
            }
            Response::Everything { medium, files } if files.is_empty() => {
                let _ = medium;
                self.fail(tr("There is nothing to play in here.").to_string());
                Outcome::Changed(Sound::Back)
            }
            Response::Everything { medium, files } => Outcome::Play(Queue { medium, files, start: 0 }),
        }
    }

    pub fn apply(&mut self, action: Action) -> Outcome {
        if self.dialog.is_some() {
            return self.apply_dialog(action);
        }
        if self.video_playing() {
            return Self::apply_video(action);
        }
        if self.now_playing {
            return self.apply_now_playing(action);
        }
        self.apply_list(action)
    }

    fn apply_dialog(&mut self, action: Action) -> Outcome {
        let Some(dialog) = self.dialog.as_mut() else { return Outcome::Nothing };
        match dialog.apply(action) {
            DialogOutcome::None => Outcome::Nothing,
            _ => {
                self.dialog = None;
                Outcome::Changed(Sound::Back)
            }
        }
    }

    fn apply_video(action: Action) -> Outcome {
        Outcome::Control(match action {
            Action::Confirm => Control::PlayPause,
            Action::Left => Control::Seek(-SEEK),
            Action::Right => Control::Seek(SEEK),
            Action::Down => Control::Seek(-SEEK_FAR),
            Action::Up => Control::Seek(SEEK_FAR),
            Action::PrevTab => Control::Previous,
            Action::NextTab => Control::Next,
            Action::Secondary => Control::CycleSubtitles,
            Action::Search => Control::CycleAudio,
            Action::Menu => Control::ShowProgress,
            Action::Back => return Outcome::Stop,
            _ => return Outcome::Nothing,
        })
    }

    fn apply_now_playing(&mut self, action: Action) -> Outcome {
        let Some(playback) = self.playback.as_mut() else { return Outcome::Nothing };
        Outcome::Control(match action {
            Action::Confirm => Control::PlayPause,
            Action::Left => Control::Seek(-SEEK),
            Action::Right => Control::Seek(SEEK),
            Action::PrevTab => playback.previous(),
            Action::NextTab => Control::Next,
            Action::Secondary => {
                playback.shuffle = !playback.shuffle;
                Control::Shuffle(playback.shuffle)
            }
            Action::Search => {
                playback.repeat = playback.repeat.next();
                Control::Repeat(playback.repeat)
            }
            Action::Menu => return Outcome::Stop,
            Action::Back => {
                self.now_playing = false;
                return Outcome::Changed(Sound::Back);
            }
            _ => return Outcome::Nothing,
        })
    }

    fn apply_list(&mut self, action: Action) -> Outcome {
        match action {
            Action::Up | Action::Down => {
                let tab = self.current_mut();
                let before = tab.focused;
                let wanted = if action == Action::Up { before.saturating_sub(1) } else { before + 1 };
                tab.focus(wanted);
                if tab.focused == before { Outcome::Nothing } else { Outcome::Changed(Sound::Move) }
            }
            Action::PrevTab => self.switch(0),
            Action::NextTab => self.switch(1),
            Action::Confirm => self.open_focused(),
            Action::Secondary => self.play_everything(),
            Action::Search if self.playback.is_some() => {
                self.now_playing = true;
                Outcome::Changed(Sound::Open)
            }
            Action::Back => self.back(),
            _ => Outcome::Nothing,
        }
    }

    pub fn switch(&mut self, tab: usize) -> Outcome {
        if tab == self.tab || tab >= self.tabs.len() {
            return Outcome::Nothing;
        }
        self.tab = tab;
        Outcome::Changed(Sound::Move)
    }

    fn open_focused(&mut self) -> Outcome {
        let current = self.current_mut();
        let Some(item) = current.items.get(current.focused).cloned() else { return Outcome::Nothing };
        if !item.folder {
            return Outcome::Play(current.queue());
        }
        current.levels.push(Level { dir: item.path, focus_before: current.focused });
        current.items.clear();
        current.focused = 0;
        current.top = 0;
        Outcome::Send(self.list())
    }

    fn play_everything(&mut self) -> Outcome {
        let current = self.current();
        let dirs = current.dir().map_or_else(|| current.roots.clone(), |dir| vec![dir.to_path_buf()]);
        Outcome::Send(Request::Everything { medium: current.medium, dirs })
    }

    fn back(&mut self) -> Outcome {
        let current = self.current_mut();
        let Some(level) = current.levels.pop() else { return Outcome::Quit };
        current.items.clear();
        current.top = 0;
        current.focused = 0;
        // The list arrives later; `focus` keeps the row in view then
        current.top = level.focus_before.saturating_sub(VISIBLE - 1);
        current.focused = level.focus_before;
        Outcome::Send(self.list())
    }

    pub fn click_row(&mut self, visible: usize) -> Outcome {
        if self.dialog.is_some() {
            return Outcome::Nothing;
        }
        let tab = self.current_mut();
        tab.focus(tab.top + visible);
        self.open_focused()
    }

    /// From the media card in the quick menu.
    pub fn media(&self, command: MediaCommand) -> Option<Control> {
        let playback = self.playback.as_ref()?;
        Some(match command {
            MediaCommand::PlayPause => Control::PlayPause,
            MediaCommand::Next => Control::Next,
            MediaCommand::Previous => playback.previous(),
            // The app tells the bus it cannot seek and has no shuffle, so these do not come
            MediaCommand::Seek(_) | MediaCommand::Shuffle(_) => return None,
        })
    }
}

/// The words on a starting video: it covers the app, so there is no hint bar.
fn video_hint(glyphs: GlyphStyle) -> String {
    let (confirm, back) = match glyphs {
        GlyphStyle::PlayStation => ("✕", "○"),
        GlyphStyle::Xbox => ("A", "B"),
    };
    trf("{}: pause, left and right: seek, {}: stop", &[&confirm, &back]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, folder: bool) -> Item {
        Item { name: name.into(), path: PathBuf::from("/m").join(name), folder }
    }

    fn page(items: Vec<Item>) -> Page {
        let mut page = Page::new(vec!["/m".into()], vec!["/v".into()]);
        for tab in [0, 1] {
            page.receive(Response::Listed { tab, dir: None, items: Ok(items.clone()) });
        }
        page
    }

    fn songs(count: usize) -> Vec<Item> {
        (0..count).map(|n| item(&format!("{n}.mp3"), false)).collect()
    }

    #[test]
    fn a_file_plays_with_the_rest_of_its_folder() {
        let mut page = page(vec![item("Album", true), item("a.mp3", false), item("b.mp3", false)]);
        page.apply(Action::Down);
        page.apply(Action::Down);
        let Outcome::Play(queue) = page.apply(Action::Confirm) else { panic!("nothing plays") };
        assert_eq!(queue.files, [PathBuf::from("/m/a.mp3"), PathBuf::from("/m/b.mp3")]);
        assert_eq!(queue.start, 1);
    }

    #[test]
    fn folders_open_and_back_returns_to_the_row() {
        let mut page = page(vec![item("a", true), item("b", true)]);
        page.apply(Action::Down);
        let dir = Some(PathBuf::from("/m/b"));
        assert_eq!(page.apply(Action::Confirm), Outcome::Send(Request::List { tab: 0, dir: dir.clone() }));
        page.receive(Response::Listed { tab: 0, dir, items: Ok(songs(1)) });
        assert_eq!(page.apply(Action::Back), Outcome::Send(Request::List { tab: 0, dir: None }));
        page.receive(Response::Listed { tab: 0, dir: None, items: Ok(vec![item("a", true), item("b", true)]) });
        assert_eq!(page.current().focused, 1);
        assert_eq!(page.apply(Action::Back), Outcome::Quit);
    }

    #[test]
    fn an_answer_for_a_folder_left_in_between_is_dropped() {
        let mut page = page(vec![item("a", true)]);
        page.apply(Action::Confirm);
        page.receive(Response::Listed { tab: 0, dir: None, items: Ok(songs(3)) });
        assert!(page.current().items.is_empty());
        assert!(page.current().loading);
    }

    #[test]
    fn the_list_scrolls_with_the_focus() {
        let mut page = page(songs(20));
        for _ in 0..12 {
            page.apply(Action::Down);
        }
        assert_eq!(page.current().visible().len(), VISIBLE);
        assert_eq!(page.current().visible_focus(), VISIBLE - 1);
    }

    fn music() -> Page {
        let mut page = page(songs(3));
        page.started(&Queue { medium: Medium::Music, files: vec!["/m/0.mp3".into(), "/m/1.mp3".into()], start: 1 });
        page
    }

    fn video() -> Page {
        let mut page = page(songs(3));
        page.started(&Queue { medium: Medium::Videos, files: vec!["/v/f.mkv".into()], start: 0 });
        page
    }

    #[test]
    fn music_keeps_playing_behind_the_lists() {
        let mut page = music();
        assert!(page.now_playing);
        assert_eq!(page.apply(Action::Confirm), Outcome::Control(Control::PlayPause));
        assert_eq!(page.apply(Action::Back), Outcome::Changed(Sound::Back));
        assert!(!page.now_playing && page.playback.is_some());
        assert_eq!(page.apply(Action::Search), Outcome::Changed(Sound::Open));
        assert!(page.now_playing);
        assert_eq!(page.apply(Action::Menu), Outcome::Stop);
    }

    #[test]
    fn previous_restarts_a_track_that_has_run_for_a_while() {
        let mut page = music();
        page.update(Update::Index(1));
        assert_eq!(page.apply(Action::PrevTab), Outcome::Control(Control::Previous));
        page.update(Update::Time(30.0));
        assert_eq!(page.apply(Action::PrevTab), Outcome::Control(Control::Restart));
    }

    #[test]
    fn shuffle_and_repeat_switch() {
        let mut page = music();
        assert_eq!(page.apply(Action::Secondary), Outcome::Control(Control::Shuffle(true)));
        assert_eq!(page.apply(Action::Secondary), Outcome::Control(Control::Shuffle(false)));
        assert_eq!(page.apply(Action::Search), Outcome::Control(Control::Repeat(Repeat::All)));
        assert_eq!(page.apply(Action::Search), Outcome::Control(Control::Repeat(Repeat::One)));
        assert_eq!(page.apply(Action::Search), Outcome::Control(Control::Repeat(Repeat::Off)));
    }

    #[test]
    fn a_video_gets_the_whole_controller() {
        let mut page = video();
        assert!(page.video_playing());
        assert_eq!(page.apply(Action::Right), Outcome::Control(Control::Seek(SEEK)));
        assert_eq!(page.apply(Action::Up), Outcome::Control(Control::Seek(SEEK_FAR)));
        assert_eq!(page.apply(Action::Secondary), Outcome::Control(Control::CycleSubtitles));
        assert_eq!(page.apply(Action::Back), Outcome::Stop);
        page.stopped(false);
        assert!(!page.video_playing());
        assert!(page.dialog.is_none());
    }

    #[test]
    fn a_video_says_its_buttons_once() {
        let mut page = video();
        page.glyphs = Some(GlyphStyle::Xbox);
        let (_, first) = page.update(Update::Duration(100.0));
        assert!(matches!(first, Some(Control::Message(text)) if text.contains("A") && text.contains("B")));
        assert_eq!(page.update(Update::Duration(100.0)).1, None);
    }

    #[test]
    fn the_time_counts_once_a_second() {
        let mut page = music();
        assert!(page.update(Update::Time(1.2)).0);
        assert!(!page.update(Update::Time(1.7)).0);
        assert!(page.update(Update::Time(2.0)).0);
    }

    #[test]
    fn tags_win_over_the_file_name() {
        let mut page = music();
        page.update(Update::Title("01-song.flac".into()));
        assert_eq!(page.shown().title, "01-song.flac");
        page.update(Update::Tags { title: Some("Song".into()), artist: "Band".into(), album: String::new() });
        assert_eq!(page.shown().title, "Song");
        assert_eq!(page.shown().artist, "Band");
    }

    #[test]
    fn a_failed_file_says_so() {
        let mut page = music();
        page.stopped(true);
        assert!(page.dialog.is_some() && page.playback.is_none() && !page.now_playing);
        assert_eq!(page.apply(Action::Confirm), Outcome::Changed(Sound::Back));
        assert!(page.dialog.is_none());
    }

    #[test]
    fn the_card_controls_what_plays() {
        assert_eq!(page(songs(1)).media(MediaCommand::Next), None);
        assert_eq!(music().media(MediaCommand::PlayPause), Some(Control::PlayPause));
    }

    #[test]
    fn clock_grows_with_the_length() {
        assert_eq!(clock(65.4), "1:05");
        assert_eq!(clock(3725.0), "1:02:05");
    }
}
