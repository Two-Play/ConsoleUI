//! The file manager of ConsoleUI (Story 8.8): an app like any other, built on `consoleui-sdk`.
//! `consoleui-files [PATH] [--actions a,b,...] [--snapshot out.png [--snapshot-after MS]]`.

mod entries;
mod ops;
mod page;
mod worker;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use chrono::{DateTime, Local};
use consoleui_core::i18n::tr;
use consoleui_sdk::{Action, Controls, Event, HintButton, Hints, Session, Sound, Sounds};
use consoleui_sdk::{apply_language, script_from_args, snapshot_from_args, watch_session};
use entries::{Entry, Kind, human_size};
use ops::Place;
use page::{Area, Outcome, Page};
use slint::{ComponentHandle, Image, ModelRc, SharedString, Timer, TimerMode, VecModel};
use worker::Request;

slint::include_modules!();
consoleui_sdk::bridge!();

fn icon(kind: Kind) -> Image {
    let data: &[u8] = match kind {
        Kind::Folder => include_bytes!("../ui/icons/folder.svg"),
        Kind::Image => include_bytes!("../ui/icons/image.svg"),
        Kind::Audio => include_bytes!("../ui/icons/audio.svg"),
        Kind::Video => include_bytes!("../ui/icons/video.svg"),
        Kind::Text => include_bytes!("../ui/icons/text.svg"),
        Kind::Archive => include_bytes!("../ui/icons/archive.svg"),
        Kind::Other => include_bytes!("../ui/icons/file.svg"),
    };
    Image::load_from_svg_data(data).unwrap_or_default()
}

fn row(entry: &Entry) -> FileRow {
    let date = entry.modified.map(|time| DateTime::<Local>::from(time).format("%Y-%m-%d").to_string());
    let size = if entry.kind == Kind::Folder { String::new() } else { human_size(entry.size) };
    FileRow {
        icon: icon(entry.kind),
        folder: entry.kind == Kind::Folder,
        name: entry.name.as_str().into(),
        size: size.into(),
        date: date.unwrap_or_default().into(),
    }
}

fn hints(page: &Page) -> Hints {
    let select = tr("Select");
    if page.dialog.is_some() {
        Hints::new().with(HintButton::Dpad, select).confirm(tr("OK")).back(tr("Cancel"))
    } else if page.typing() {
        Hints::new().with(HintButton::Dpad, select).confirm(tr("Type")).back(tr("Cancel"))
    } else if page.menu().is_some() {
        Hints::new().updown(select).confirm(tr("OK")).back(tr("Close"))
    } else if page.area == Area::Places {
        Hints::new().updown(select).confirm(tr("Open")).back(tr("Quit"))
    } else {
        let hints = Hints::new().updown(select).confirm(tr("Open"));
        hints.with(HintButton::Menu, tr("Options")).back(tr("Back"))
    }
}

/// Puts the state of the page into the window.
fn show(ui: &AppWindow, page: &Page) {
    let places: Vec<SharedString> = page.places().iter().map(|place: &Place| place.name.as_str().into()).collect();
    ui.set_places(ModelRc::new(VecModel::from(places)));
    ui.set_place(page.place() as i32);
    ui.set_places_focused(page.area == Area::Places);
    ui.set_path(page.dir().display().to_string().into());
    ui.set_rows(ModelRc::new(VecModel::from(page.visible().iter().map(row).collect::<Vec<_>>())));
    ui.set_focused(if page.area == Area::Files { page.visible_focus() as i32 } else { -1 });
    let total = page.total();
    let position = if total == 0 { String::new() } else { format!("{} / {total}", page.focused() + 1) };
    ui.set_position(position.into());
    ui.set_loading(page.loading || page.busy);
    ui.set_empty(!page.loading && total == 0);
    ui.set_error(page.error.as_deref().unwrap_or_default().into());

    let menu = page.menu();
    ui.set_menu_open(menu.is_some());
    if let Some((items, focused)) = menu {
        let rows = items.iter().map(|item| ContextItem { label: item.label(page.hidden).into(), symbol: "".into() });
        ui.set_menu_items(ModelRc::new(VecModel::from(rows.collect::<Vec<_>>())));
        ui.set_menu_focused(focused as i32);
    }

    ui.set_typing(page.typing());
    ui.set_naming_title(page.naming_title().into());
    ui.set_text(page.text.as_str().into());
    ui.set_key_focused(page.keyboard.focused() as i32);

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

fn show_keys(ui: &AppWindow, page: &Page) {
    let keys = page.keyboard.keys().iter().map(|key| KeyCap {
        label: key.key.label().into(),
        row: key.row as i32,
        x: key.x,
        width: key.width,
    });
    ui.set_keys(ModelRc::new(VecModel::from(keys.collect::<Vec<_>>())));
}

/// With the app the system has for this kind of file. As a child of this app it counts as part of
/// it, so under our compositor its window comes up in front.
fn open(path: &Path) {
    let opened = Command::new("xdg-open").arg(path).stdin(Stdio::null()).spawn();
    match opened {
        // Waits for it on a thread, so that no zombie stays behind
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => eprintln!("Cannot open {}: {err}", path.display()),
    }
}

struct Files {
    ui: slint::Weak<AppWindow>,
    page: RefCell<Page>,
    requests: Sender<Request>,
    sounds: Rc<Sounds>,
}

impl Files {
    fn refresh(&self) {
        if let Some(ui) = self.ui.upgrade() {
            show(&ui, &self.page.borrow());
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
            Outcome::Open(path) => {
                self.sounds.play(Sound::Confirm);
                open(&path);
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

    /// Runs something on the page that has an outcome, from a click.
    fn with(&self, change: impl FnOnce(&mut Page) -> Outcome) {
        let outcome = change(&mut self.page.borrow_mut());
        self.outcome(outcome);
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

    // A folder on the command line takes the place of the home folder
    let mut places = ops::places();
    let start = std::env::args().nth(1).filter(|arg| !arg.starts_with("--")).map(PathBuf::from);
    if let Some(path) = start.filter(|path| path.is_dir()) {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        places.insert(0, Place { name, path });
    }
    let page = Page::new(places);
    let (requests, responses) = worker::spawn();
    let _ = requests.send(page.list());
    let files = Rc::new(Files { ui: ui.as_weak(), page: RefCell::new(page), requests, sounds });
    show_keys(&ui, &files.page.borrow());
    files.refresh();

    let poll = Timer::default();
    poll.start(TimerMode::Repeated, Duration::from_millis(30), {
        let files = files.clone();
        move || {
            let mut changed = false;
            for response in responses.try_iter() {
                changed = true;
                if let Some(request) = files.page.borrow_mut().receive(response) {
                    let _ = files.requests.send(request);
                }
            }
            if changed {
                files.refresh();
            }
        }
    });

    let controls = Rc::new(Controls::start(
        {
            let ui = ui.as_weak();
            move || ui.upgrade().is_some_and(|ui| ui.get_active())
        },
        {
            let (ui, files) = (ui.as_weak(), files.clone());
            move |event| match event {
                Event::Action(action) => files.action(action),
                Event::Glyphs(glyphs) => {
                    if let Some(ui) = ui.upgrade() {
                        ui.set_style(controller_style(glyphs));
                    }
                }
                Event::Controller(_) => {}
            }
        },
    ));
    ui.on_action({
        let (files, controls) = (files.clone(), controls.clone());
        move |action| {
            // Enter ends the name when a real keyboard types it
            if files.page.borrow().typing() && action == NavAction::Confirm {
                files.with(Page::finish_naming);
            } else {
                controls.key(nav_action(action));
            }
        }
    });
    ui.on_typed({
        let files = files.clone();
        move |text| {
            files.page.borrow_mut().typed(&text);
            files.refresh();
        }
    });
    ui.on_erase({
        let files = files.clone();
        move || {
            files.page.borrow_mut().erase();
            files.refresh();
        }
    });
    ui.on_place_clicked({
        let files = files.clone();
        move |index| files.with(|page| page.open_place(index as usize))
    });
    ui.on_row_clicked({
        let files = files.clone();
        move |index| files.with(|page| page.click_row(index as usize))
    });
    ui.on_menu_clicked({
        let files = files.clone();
        move |index| files.with(|page| page.click_menu(index as usize))
    });
    ui.on_dialog_clicked({
        let files = files.clone();
        move |index| {
            if let Some(dialog) = files.page.borrow_mut().dialog.as_mut() {
                dialog.focus(index as usize);
            }
            files.action(Action::Confirm);
        }
    });
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    let _script = script_from_args(
        {
            let files = files.clone();
            move || !files.page.borrow().loading
        },
        {
            let files = files.clone();
            move |action| files.action(action)
        },
    );
    let _snapshot = snapshot_from_args(&ui);
    ui.run()
}
