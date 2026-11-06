//! The media player of ConsoleUI (Story 8.8): an app on `consoleui-sdk` that plays with mpv.
//! `consoleui-media [FOLDER] [--actions a,b,...] [--snapshot out.png [--snapshot-after MS]]`.
//!
//! Music plays without a window, the app shows it and keeps it going in the background. A video
//! is mpv's own window over the app; the controller stays with the app, which steers mpv.

mod library;
mod mpris;
mod mpv;
mod page;
mod player;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use consoleui_core::i18n::{tr, trf};
use consoleui_sdk::{Action, Controls, Event, HintButton, Hints, Session, Sound, Sounds};
use consoleui_sdk::{apply_language, script_from_args, snapshot_from_args, watch_session};
use library::{Item, Medium};
use mpris::Mpris;
use mpv::{Control, Queue, Repeat};
use page::{Outcome, Page, Playback, Request, Response, clock};
use player::{Player, PlayerEvent};
use slint::{ComponentHandle, Image, ModelRc, SharedString, Timer, TimerMode, VecModel};

slint::include_modules!();
consoleui_sdk::bridge!();

fn icon(data: &[u8]) -> Image {
    Image::load_from_svg_data(data).unwrap_or_default()
}

fn row(item: &Item, medium: Medium) -> MediaRow {
    let data: &[u8] = match (item.folder, medium) {
        (true, _) => include_bytes!("../ui/icons/folder.svg"),
        (false, Medium::Music) => include_bytes!("../ui/icons/audio.svg"),
        (false, Medium::Videos) => include_bytes!("../ui/icons/video.svg"),
    };
    MediaRow { icon: icon(data), folder: item.folder, name: item.name.as_str().into() }
}

fn hints(page: &Page) -> Hints {
    let select = tr("Select");
    if page.dialog.is_some() {
        return Hints::new().with(HintButton::Dpad, select).confirm(tr("OK")).back(tr("Close"));
    }
    if page.now_playing {
        let paused = page.playback.as_ref().is_some_and(|playback| playback.paused);
        let hints = Hints::new().confirm(if paused { tr("Play") } else { tr("Pause") });
        let hints = hints.with(HintButton::Dpad, tr("Seek")).with(HintButton::West, tr("Shuffle"));
        return hints.with(HintButton::North, tr("Repeat")).with(HintButton::Menu, tr("Stop")).back(tr("Back"));
    }
    let tab = page.current();
    let mut hints = Hints::new().updown(select).confirm(tr("Open"));
    hints = hints.with(HintButton::West, tr("Play all"));
    if page.playback.is_some() {
        hints = hints.with(HintButton::North, tr("Now playing"));
    }
    hints.back(if tab.dir().is_some() { tr("Back") } else { tr("Quit") })
}

fn show_playback(ui: &AppWindow, playback: &Playback) {
    ui.set_track(playback.title.as_str().into());
    ui.set_artist(playback.artist.as_str().into());
    ui.set_album(playback.album.as_str().into());
    ui.set_paused(playback.paused);
    ui.set_progress(playback.progress());
    ui.set_time(clock(playback.position).into());
    ui.set_length(clock(playback.duration).into());
    let index = (playback.index + 1).to_string();
    let count = playback.count.to_string();
    ui.set_queue_place(trf("Track {} of {}", &[&index, &count]).to_string().into());
    ui.set_shuffle(playback.shuffle);
    ui.set_repeat(match playback.repeat {
        Repeat::Off => SharedString::new(),
        Repeat::All => tr("Repeat all").into(),
        Repeat::One => tr("Repeat track").into(),
    });
}

/// Puts the state of the page into the window.
fn show(ui: &AppWindow, page: &Page) {
    let tab = page.current();
    ui.set_tab(page.tab as i32);
    let path = tab.dir().map(|dir| dir.display().to_string());
    ui.set_path(path.unwrap_or_default().into());
    let rows: Vec<MediaRow> = tab.visible().iter().map(|item| row(item, tab.medium)).collect();
    ui.set_rows(ModelRc::new(VecModel::from(rows)));
    ui.set_focused(tab.visible_focus() as i32);
    let total = tab.items.len();
    ui.set_position(if total == 0 { String::new() } else { format!("{} / {total}", tab.focused + 1) }.into());
    ui.set_loading(tab.loading);
    ui.set_empty(!tab.loading && total == 0);
    ui.set_error(tab.error.as_deref().unwrap_or_default().into());
    let folder = tab.roots().first().map(|dir| dir.display().to_string()).unwrap_or_default();
    ui.set_empty_text(if tab.dir().is_some() {
        SharedString::new()
    } else {
        trf("Put your files into {}", &[&folder]).to_string().into()
    });

    ui.set_now_playing(page.now_playing);
    ui.set_playing(page.playback.is_some());
    if let Some(playback) = &page.playback {
        show_playback(ui, playback);
    }

    let dialog = page.dialog.as_ref();
    ui.set_dialog_open(dialog.is_some());
    if let Some(dialog) = dialog {
        ui.set_dialog_title(dialog.title.as_str().into());
        ui.set_dialog_text(dialog.text.as_str().into());
        let labels: Vec<SharedString> = dialog.buttons().iter().map(|b| b.label.as_str().into()).collect();
        ui.set_dialog_buttons(ModelRc::new(VecModel::from(labels)));
        ui.set_dialog_focused(dialog.focused() as i32);
    }
    ui.set_hints(hint_model(hints(page)));
}

fn worker(roots: [Vec<PathBuf>; 2]) -> (Sender<Request>, Receiver<Response>) {
    let (requests, incoming) = mpsc::channel();
    let (outgoing, responses) = mpsc::channel();
    let media = [Medium::Music, Medium::Videos];
    std::thread::spawn(move || {
        for request in incoming {
            let response = match request {
                Request::List { tab, dir } => {
                    let listed = match &dir {
                        Some(dir) => library::list(dir, media[tab]),
                        None => library::list_top(&roots[tab], media[tab]),
                    };
                    Response::Listed { tab, dir, items: listed.map_err(|err| err.to_string()) }
                }
                Request::Everything { medium, dirs } => {
                    Response::Everything { medium, files: library::everything(&dirs, medium) }
                }
            };
            if outgoing.send(response).is_err() {
                break;
            }
        }
    });
    (requests, responses)
}

struct Media {
    ui: slint::Weak<AppWindow>,
    page: RefCell<Page>,
    requests: Sender<Request>,
    sounds: Rc<Sounds>,
    player: RefCell<Option<Player>>,
    events: Sender<PlayerEvent>,
    next_id: Cell<u64>,
    mpris: Option<Mpris>,
    /// The file the cover in the window belongs to
    cover_of: RefCell<Option<PathBuf>>,
}

impl Media {
    fn refresh(&self) {
        let page = self.page.borrow();
        if let Some(ui) = self.ui.upgrade() {
            show(&ui, &page);
            self.show_cover(&ui, &page);
        }
        if let Some(mpris) = &self.mpris {
            mpris.show(page.shown());
        }
    }

    fn show_cover(&self, ui: &AppWindow, page: &Page) {
        let path = page.playback.as_ref().and_then(|playback| playback.path.clone());
        let mut shown = self.cover_of.borrow_mut();
        // Tracks of one album share the file; asking only on a change keeps the disk out of the timer
        if *shown != path {
            let cover = path.as_deref().and_then(library::cover);
            let image = cover.and_then(|cover| Image::load_from_path(&cover).ok());
            ui.set_has_cover(image.is_some());
            ui.set_album_art(image.unwrap_or_default());
            *shown = path;
        }
    }

    fn control(&self, control: &Control) {
        if let Some(player) = self.player.borrow().as_ref() {
            player.control(control);
        }
    }

    fn play(&self, queue: Queue) {
        if let Some(player) = self.player.borrow_mut().take() {
            player.quit();
        }
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        match Player::start(id, &queue, self.events.clone()) {
            Ok(player) => {
                self.sounds.play(Sound::Confirm);
                *self.player.borrow_mut() = Some(player);
                self.page.borrow_mut().started(&queue);
            }
            Err(err) => {
                eprintln!("Cannot start mpv: {err}");
                self.page.borrow_mut().no_player();
            }
        }
    }

    fn outcome(&self, outcome: Outcome) {
        match outcome {
            Outcome::Nothing => return,
            Outcome::Changed(sound) => self.sounds.play(sound),
            Outcome::Send(request) => {
                self.sounds.play(Sound::Confirm);
                let _ = self.requests.send(request);
            }
            Outcome::Play(queue) => self.play(queue),
            // Its state comes back from mpv
            Outcome::Control(control) => self.control(&control),
            Outcome::Stop => {
                self.sounds.play(Sound::Back);
                if let Some(player) = self.player.borrow().as_ref() {
                    player.quit();
                }
            }
            // Closing the window returns to the shell
            Outcome::Quit => {
                let _ = slint::quit_event_loop();
            }
        }
        self.refresh();
    }

    fn action(&self, action: Action) {
        let outcome = self.page.borrow_mut().apply(action);
        self.outcome(outcome);
    }

    fn with(&self, change: impl FnOnce(&mut Page) -> Outcome) {
        let outcome = change(&mut self.page.borrow_mut());
        self.outcome(outcome);
    }

    fn player_event(&self, event: PlayerEvent) -> bool {
        let current = self.player.borrow().as_ref().map(|player| player.id);
        match event {
            PlayerEvent::Update(id, update) if Some(id) == current => {
                let (changed, tell) = self.page.borrow_mut().update(update);
                if let Some(control) = tell {
                    self.control(&control);
                }
                changed
            }
            PlayerEvent::Gone(id, failed) if Some(id) == current => {
                *self.player.borrow_mut() = None;
                self.page.borrow_mut().stopped(failed);
                true
            }
            // From the mpv before this one
            _ => false,
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let session = Session::detect();
    apply_language(&session);
    apply_theme(ui.global::<Theme>(), &session);
    if session.fullscreen() {
        ui.window().set_fullscreen(true);
    }
    let sounds = Rc::new(Sounds::start(&session));
    let _watch = watch_session(&session, {
        let (ui, sounds) = (ui.as_weak(), sounds.clone());
        move |session| {
            sounds.follow(session);
            if let Some(ui) = ui.upgrade() {
                apply_theme(ui.global::<Theme>(), session);
            }
        }
    });

    // A folder on the command line takes the place of the user's folders
    let start = std::env::args().nth(1).filter(|arg| !arg.starts_with("--")).map(PathBuf::from);
    let roots = match start.filter(|path| path.is_dir()) {
        Some(path) => [vec![path.clone()], vec![path]],
        None => [library::roots(Medium::Music), library::roots(Medium::Videos)],
    };
    let [music, videos] = roots.clone();
    let mut page = Page::new(music, videos);
    let (requests, responses) = worker(roots);
    for tab in [0, 1] {
        let _ = requests.send(page.listing(tab));
    }
    let (events, player_events) = mpsc::channel();
    let media = Rc::new(Media {
        ui: ui.as_weak(),
        page: RefCell::new(page),
        requests,
        sounds,
        player: RefCell::new(None),
        events,
        next_id: Cell::new(1),
        mpris: Mpris::start(),
        cover_of: RefCell::new(None),
    });
    media.refresh();

    let poll = Timer::default();
    poll.start(TimerMode::Repeated, Duration::from_millis(30), {
        let media = media.clone();
        move || {
            for response in responses.try_iter() {
                let outcome = media.page.borrow_mut().receive(response);
                match outcome {
                    // A list that arrives is no reason for a sound
                    Outcome::Changed(_) => media.refresh(),
                    outcome => media.outcome(outcome),
                }
            }
            let mut changed = false;
            for event in player_events.try_iter() {
                changed |= media.player_event(event);
            }
            let commands: Vec<_> = media.mpris.iter().flat_map(|mpris| mpris.commands.try_iter()).collect();
            for command in commands {
                let control = media.page.borrow().media(command);
                if let Some(control) = control {
                    media.control(&control);
                }
            }
            if changed {
                media.refresh();
            }
        }
    });

    let controls = Rc::new(Controls::start(
        {
            let (ui, media) = (ui.as_weak(), media.clone());
            // While the quick menu is open the shell has the controller grabbed, so this cannot steal
            move || media.page.borrow().video_playing() || ui.upgrade().is_some_and(|ui| ui.get_active())
        },
        {
            let (ui, media) = (ui.as_weak(), media.clone());
            move |event| match event {
                Event::Action(action) => media.action(action),
                Event::Glyphs(glyphs) => {
                    media.page.borrow_mut().glyphs = glyphs;
                    if let Some(ui) = ui.upgrade() {
                        ui.set_style(controller_style(glyphs));
                    }
                }
                Event::Controller(_) => {}
            }
        },
    ));
    ui.on_action({
        let controls = controls.clone();
        move |action| controls.key(nav_action(action))
    });
    ui.on_tab_clicked({
        let media = media.clone();
        move |tab| media.with(|page| page.switch(tab as usize))
    });
    ui.on_row_clicked({
        let media = media.clone();
        move |index| media.with(|page| page.click_row(index as usize))
    });
    ui.on_bar_clicked({
        let media = media.clone();
        move || media.action(Action::Search)
    });
    ui.on_dialog_clicked({
        let media = media.clone();
        move |_| media.action(Action::Confirm)
    });
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    let _script = script_from_args(
        {
            let media = media.clone();
            move || !media.page.borrow().loading()
        },
        {
            let media = media.clone();
            move |action| media.action(action)
        },
    );
    let _snapshot = snapshot_from_args(&ui);
    let result = ui.run();
    if let Some(player) = media.player.borrow().as_ref() {
        player.finish();
    }
    result
}
