//! A ConsoleUI app: a window on the desktop, fullscreen in the console session.

mod page;

use std::cell::RefCell;
use std::rc::Rc;

use consoleui_sdk::{Action, Controls, Event, Hints, Session, Sounds, snapshot_from_args};
use page::{Outcome, Page};
use slint::{ComponentHandle, ModelRc, VecModel};

slint::include_modules!();
consoleui_sdk::bridge!();

/// Puts the state of the page into the window.
fn show(ui: &AppWindow, page: &Page) {
    let row = |(label, on): &(&str, bool)| SettingRow {
        label: (*label).into(),
        description: "Left and right change it".into(),
        value: if *on { "On" } else { "Off" }.into(),
        opens_page: false,
    };
    ui.set_rows(ModelRc::new(VecModel::from(page.rows.iter().map(row).collect::<Vec<_>>())));
    ui.set_focused(page.focused as i32);
    ui.set_quitting(page.quitting.is_some());
    ui.set_dialog_focused(page.quitting.unwrap_or(0) as i32);
    let hints = if page.quitting.is_some() {
        Hints::new().confirm("Select").back("Cancel")
    } else {
        Hints::new().updown("Select").confirm("Change").back("Quit")
    };
    ui.set_hints(hint_model(hints));
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let session = Session::detect();
    apply_theme(ui.global::<Theme>(), &session);
    if session.fullscreen() {
        ui.window().set_fullscreen(true);
    }
    let sounds = Sounds::start(&session);
    let page = Rc::new(RefCell::new(Page::new()));
    show(&ui, &page.borrow());

    // Every input ends up here: controller, keyboard and, through the callbacks below, the mouse
    let handle = {
        let (ui, page) = (ui.as_weak(), page.clone());
        move |action: Action| {
            let Some(ui) = ui.upgrade() else { return };
            match page.borrow_mut().apply(action) {
                Outcome::Nothing => return,
                Outcome::Changed(sound) => sounds.play(sound),
                // Closing the window returns to the shell
                Outcome::Quit => {
                    let _ = slint::quit_event_loop();
                }
            }
            show(&ui, &page.borrow());
        }
    };
    let handle = Rc::new(handle);

    let controls = Rc::new(Controls::start(
        {
            let ui = ui.as_weak();
            move || ui.upgrade().is_some_and(|ui| ui.get_active())
        },
        {
            let (ui, handle) = (ui.as_weak(), handle.clone());
            move |event| match event {
                Event::Action(action) => handle(action),
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
        let controls = controls.clone();
        move |action| controls.key(nav_action(action))
    });
    ui.on_row_clicked({
        let (page, handle) = (page.clone(), handle.clone());
        move |row| {
            page.borrow_mut().focused = row as usize;
            handle(Action::Confirm);
        }
    });
    ui.on_dialog_clicked({
        let (page, handle) = (page.clone(), handle.clone());
        move |button| {
            page.borrow_mut().quitting = Some(button as usize);
            handle(Action::Confirm);
        }
    });
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    let _snapshot = snapshot_from_args(&ui);
    ui.run()
}
