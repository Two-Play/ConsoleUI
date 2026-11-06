//! The store of ConsoleUI (Story 8.9): Flathub's apps, found and installed with a controller. An
//! app like any other, built on `consoleui-sdk`; what it installs shows up in the shell's library.
//! `consoleui-store [--fake] [--actions a,b,...] [--snapshot out.png [--snapshot-after MS]]`.

mod appstream;
mod backend;
mod flatpak;
mod page;
mod shots;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use backend::{FakeBackend, Request, Response};
use consoleui_core::accent;
use consoleui_core::i18n::{tr, trf};
use consoleui_sdk::{Action, Controls, Event, HintButton, Hints, Session, Sound, Sounds, snapshot_from_args};
use consoleui_sdk::{apply_language, launch, library_changed, script_from_args, toast, watch_session};
use page::{Area, COLUMNS, Finished, Outcome, Page, ROWS, Section};
use shots::{Loaded, Shots};
use slint::{Color, ComponentHandle, Image, ModelRc, SharedPixelBuffer, Timer, TimerMode, VecModel};

slint::include_modules!();
consoleui_sdk::bridge!();

/// Decoded icons by app, so that scrolling does not read the same files again.
#[derive(Default)]
struct Icons(HashMap<String, Option<(Image, Color)>>);

impl Icons {
    fn get(&mut self, listing: &appstream::Listing) -> Option<(Image, Color)> {
        let load = || listing.icon.as_deref().and_then(decode);
        self.0.entry(listing.id.clone()).or_insert_with(load).clone()
    }
}

fn decode(path: &Path) -> Option<(Image, Color)> {
    let rgba = image::open(path).ok()?.into_rgba8();
    let pixels = SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height());
    let [r, g, b] = accent::from_rgba(rgba.as_raw()).map_or([0x3b, 0x41, 0x52], |(primary, _)| primary);
    // Dark enough for a light icon on it
    let accent = Color::from_rgb_u8(r / 2, g / 2, b / 2);
    Some((Image::from_rgba8(pixels), accent))
}

/// A color of its own for apps without an icon.
fn color_of(id: &str) -> Color {
    let hash = id.bytes().fold(7u32, |hash, byte| hash.wrapping_mul(31).wrapping_add(u32::from(byte)));
    let channel = |shift: u32| 40 + ((hash >> shift) % 80) as u8;
    Color::from_rgb_u8(channel(0), channel(8), channel(16))
}

fn status_of(page: &Page, listing: &appstream::Listing) -> String {
    match page.job(&listing.id) {
        Some(job) if job.removing => tr("Removing…").to_string(),
        Some(page::Job { percent: Some(percent), .. }) => trf("Installing {} %", &[&percent]),
        Some(_) => tr("Installing…").to_string(),
        None => String::new(),
    }
}

fn tile(icons: &mut Icons, page: &Page, listing: &appstream::Listing) -> AppTile {
    let icon = icons.get(listing);
    let status = status_of(page, listing);
    let subtitle = match status.is_empty() {
        false => status,
        true if listing.installed => tr("Installed").to_string(),
        true => listing.developer.clone(),
    };
    AppTile {
        title: listing.name.as_str().into(),
        subtitle: subtitle.into(),
        monogram: listing.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into(),
        show_monogram: icon.is_none(),
        has_icon: icon.is_some(),
        accent: icon.as_ref().map_or_else(|| color_of(&listing.id), |(_, color)| *color),
        icon: icon.map(|(image, _)| image).unwrap_or_default(),
        ..AppTile::default()
    }
}

fn hints(page: &Page) -> Hints {
    if page.dialog.is_some() {
        Hints::new().with(HintButton::Dpad, tr("Select")).confirm(tr("OK")).back(tr("Cancel"))
    } else if page.typing {
        Hints::new().with(HintButton::Dpad, tr("Select")).confirm(tr("Type")).back(tr("Close"))
    } else if page.viewing() {
        Hints::new().with(HintButton::Dpad, tr("Select")).back(tr("Close"))
    } else if page.detail().is_some() {
        let buttons = page.buttons();
        let mut hints = Hints::new();
        match page.area() {
            Area::Buttons => {
                if let Some(button) = buttons.get(page.button()) {
                    hints = hints.confirm(button.label());
                }
            }
            Area::Shots => hints = hints.with(HintButton::Dpad, tr("Select")).confirm(tr("Enlarge")),
            Area::Text => {}
        }
        hints.updown(tr("Scroll")).back(tr("Back"))
    } else {
        let back = if page.query.is_empty() { tr("Quit") } else { tr("Back") };
        let hints = Hints::new().with(HintButton::Dpad, tr("Select"));
        hints.confirm(tr("Details")).with(HintButton::North, tr("Search")).back(back)
    }
}

/// Puts the state of the page into the window.
fn show(ui: &AppWindow, page: &Page, icons: &mut Icons, pictures: &HashMap<String, Image>) {
    let tiles: Vec<AppTile> = page.visible().map(|listing| tile(icons, page, listing)).collect();
    ui.set_tiles(ModelRc::new(VecModel::from(tiles)));
    ui.set_focused(page.visible_focus() as i32);
    ui.set_section(page.section().map_or(-1, |section| section as i32));
    ui.set_loading(page.loading);
    let total = page.total();
    let first = page.visible_focus() + 1 + (page.first_visible());
    ui.set_position(if total == 0 { String::new() } else { format!("{first} / {total}") }.into());
    let (title, text) = match (&page.error, page.query.is_empty(), page.section().map(|s| Section::ALL[s])) {
        (Some(error), ..) => (tr("The store is not available"), error.as_str()),
        (_, false, _) => (tr("Nothing found"), tr("Try another word.")),
        (_, _, Some(Section::Installed)) => {
            (tr("Nothing installed yet"), tr("What you install here shows up in your library."))
        }
        _ => (tr("Nothing here"), ""),
    };
    ui.set_empty_title(title.into());
    ui.set_empty_text(text.into());

    let detail = page.detail().map(|listing| {
        let percent = page.job(&listing.id).and_then(|job| job.percent);
        StoreDetail {
            open: true,
            tile: tile(icons, page, listing),
            developer: listing.developer.as_str().into(),
            summary: listing.summary.as_str().into(),
            description: listing.description.as_str().into(),
            gamepad: listing.gamepad,
            verified: listing.verified,
            installed: listing.installed,
            status: status_of(page, listing).into(),
            progress: percent.map_or(-1.0, |percent| f32::from(percent) / 100.0),
            buttons: ModelRc::new(VecModel::from(
                page.buttons().iter().map(|b| b.label().into()).collect::<Vec<slint::SharedString>>(),
            )),
            button: if page.area() == Area::Buttons { page.button() as i32 } else { -1 },
            scroll: page.scroll,
            shots: ModelRc::new(VecModel::from(
                page.shots()
                    .iter()
                    .map(|shot| {
                        let picture = pictures.get(&shot.url);
                        StoreShot {
                            ready: picture.is_some(),
                            image: picture.cloned().unwrap_or_default(),
                            caption: shot.caption.as_str().into(),
                        }
                    })
                    .collect::<Vec<_>>(),
            )),
            shot: page.shot() as i32,
            shots_focused: page.area() == Area::Shots,
            viewing: page.viewing(),
        }
    });
    ui.set_detail(detail.unwrap_or_default());

    ui.set_typing(page.typing);
    ui.set_query(page.query.as_str().into());
    ui.set_key_focused(page.keyboard.focused() as i32);
    let dialog = page.dialog.as_ref();
    ui.set_dialog_open(dialog.is_some());
    if let Some(dialog) = dialog {
        ui.set_dialog_title(dialog.title.as_str().into());
        ui.set_dialog_text(dialog.text.as_str().into());
        let labels: Vec<slint::SharedString> = dialog.buttons().iter().map(|b| b.label.as_str().into()).collect();
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

/// Starts an installed app: through the shell if there is one, so it counts as launched there.
fn open(id: String) {
    std::thread::spawn(move || {
        if !launch(&format!("desktop:{id}.desktop"))
            && let Err(err) = Command::new("flatpak").args(["run", &id]).spawn()
        {
            eprintln!("Cannot start {id}: {err}");
        }
    });
}

/// The state of the running store, shared by the input paths.
struct Store {
    ui: slint::Weak<AppWindow>,
    page: RefCell<Page>,
    icons: RefCell<Icons>,
    shots: Shots,
    /// Decoded screenshots of the open detail page
    pictures: RefCell<HashMap<String, Image>>,
    /// The app whose screenshots were asked for
    wanted: RefCell<Option<String>>,
    requests: Sender<Request>,
    sounds: Rc<Sounds>,
}

impl Store {
    fn refresh(&self) {
        self.want_shots();
        if let Some(ui) = self.ui.upgrade() {
            show(&ui, &self.page.borrow(), &mut self.icons.borrow_mut(), &self.pictures.borrow());
        }
    }

    /// A new detail page: its screenshots start loading, the ones before make room.
    fn want_shots(&self) {
        let page = self.page.borrow();
        let id = page.detail().map(|listing| listing.id.clone());
        if *self.wanted.borrow() != id {
            self.pictures.borrow_mut().clear();
            self.shots.want(page.shots().iter().map(|shot| shot.url.clone()));
            *self.wanted.borrow_mut() = id;
        }
    }

    fn poll_shots(&self) {
        let mut changed = false;
        for loaded in self.shots.loaded.try_iter() {
            changed = true;
            match loaded {
                Loaded::Ready(url, pixels) => {
                    // Still on this page?
                    if self.page.borrow().shots().iter().any(|shot| shot.url == url) {
                        self.pictures.borrow_mut().insert(url, Image::from_rgba8(pixels));
                    }
                }
                Loaded::Failed(url) => self.page.borrow_mut().drop_shot(&url),
            }
        }
        if changed {
            self.refresh();
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
            Outcome::Open(id) => {
                self.sounds.play(Sound::Confirm);
                open(id);
            }
            // Closing the window returns to the shell
            Outcome::Quit => {
                let _ = slint::quit_event_loop();
            }
        }
        self.refresh();
    }

    fn action(&self, action: Action) {
        if let Some(ui) = self.ui.upgrade() {
            self.page.borrow_mut().max_scroll = ui.get_max_scroll();
        }
        let outcome = self.page.borrow_mut().apply(action);
        self.outcome(outcome);
    }

    fn poll(&self, responses: &Receiver<Response>) {
        let mut changed = false;
        for response in responses.try_iter() {
            changed = true;
            let finished = self.page.borrow_mut().receive(response);
            match finished {
                Finished::Nothing => {}
                Finished::Installed(name) => {
                    toast(&trf("{} is installed", &[&name]), tr("You find it in your library."));
                    library_changed();
                }
                Finished::Removed(name) => {
                    toast(&trf("{} was removed", &[&name]), "");
                    library_changed();
                }
            }
        }
        if changed {
            self.refresh();
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
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

    let fake = args.iter().any(|arg| arg == "--fake");
    let (requests, responses) = if fake { backend::spawn(FakeBackend) } else { backend::spawn(flatpak::Flatpak) };
    let store = Rc::new(Store {
        ui: ui.as_weak(),
        page: RefCell::new(Page::new()),
        icons: RefCell::default(),
        shots: Shots::start(fake),
        pictures: RefCell::default(),
        wanted: RefCell::default(),
        requests,
        sounds,
    });
    show_keys(&ui, &store.page.borrow());
    store.refresh();

    let poll = Timer::default();
    poll.start(TimerMode::Repeated, Duration::from_millis(50), {
        let store = store.clone();
        move || {
            store.poll(&responses);
            store.poll_shots();
        }
    });

    let controls = Rc::new(Controls::start(
        {
            let ui = ui.as_weak();
            move || ui.upgrade().is_some_and(|ui| ui.get_active())
        },
        {
            let (ui, store) = (ui.as_weak(), store.clone());
            move |event| match event {
                Event::Action(action) => store.action(action),
                Event::Glyphs(glyphs) => {
                    if let Some(ui) = ui.upgrade() {
                        ui.set_style(controller_style(glyphs));
                    }
                }
                Event::Controller(_) => {}
            }
        },
    ));
    let keyboard = {
        let (store, controls) = (store.clone(), controls.clone());
        move |action| {
            controls.key(nav_action(action));
            store.refresh();
        }
    };
    ui.on_action(keyboard);
    ui.on_typed({
        let store = store.clone();
        move |text| {
            store.page.borrow_mut().typed(&text);
            store.refresh();
        }
    });
    ui.on_erase({
        let store = store.clone();
        move || {
            store.page.borrow_mut().erase();
            store.refresh();
        }
    });
    ui.on_tab_clicked({
        let store = store.clone();
        move |tab| {
            store.page.borrow_mut().go_to_section(tab as usize);
            store.refresh();
        }
    });
    ui.on_tile_clicked({
        let store = store.clone();
        move |index| {
            store.page.borrow_mut().focus_visible(index as usize);
            store.action(Action::Confirm);
        }
    });
    ui.on_button_clicked({
        let store = store.clone();
        move |index| {
            let mut page = store.page.borrow_mut();
            let button = page.buttons().get(index as usize).copied();
            let outcome = page.press(button);
            drop(page);
            store.outcome(outcome);
        }
    });
    ui.on_shot_clicked({
        let store = store.clone();
        move |index| {
            store.page.borrow_mut().view_shot(index as usize);
            store.refresh();
        }
    });
    ui.on_dialog_clicked({
        let store = store.clone();
        move |index| {
            if let Some(dialog) = store.page.borrow_mut().dialog.as_mut() {
                dialog.focus(index as usize);
            }
            store.action(Action::Confirm);
        }
    });
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    let _script = script_from_args(
        {
            let store = store.clone();
            move || !store.page.borrow().loading
        },
        {
            let store = store.clone();
            move |action| store.action(action)
        },
    );

    debug_assert_eq!(COLUMNS * ROWS, 21, "the grid in main.slint has seven columns");
    let _snapshot = snapshot_from_args(&ui);
    ui.run()
}
