//! Showcase of the ConsoleUI design system and the smallest example of an app built on it.
//! `consoleui-gallery [--snapshot out.png] [--dialog]`.

use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;
use std::time::Duration;

use consoleui_app::audio::Audio;
use consoleui_app::{Action, Event, Hints, Input, Session, Sound};
use slint::{Color, Image, Model, ModelRc, SharedPixelBuffer, Timer, TimerMode, VecModel};

slint::include_modules!();
consoleui_app::slint_bridge!();

fn tile(title: &str, monogram: &str, accent: Color) -> AppTile {
    let cover = Image::from_rgba8(SharedPixelBuffer::new(1, 1));
    AppTile {
        title: title.into(),
        subtitle: "".into(),
        kind: "APP".into(),
        action_label: "Open".into(),
        monogram: monogram.into(),
        show_monogram: true,
        cover: cover.clone(),
        icon: cover.clone(),
        has_icon: false,
        logo: cover,
        has_logo: false,
        accent,
        running: monogram == "M",
        pinned: monogram == "B",
    }
}

/// A small QWERTY layout in key units (the real one comes from `consoleui_core::osk`).
fn keyboard() -> Vec<KeyCap> {
    ["qwertyuiop", "asdfghjkl", "zxcvbnm"]
        .iter()
        .enumerate()
        .flat_map(|(row, letters)| {
            letters.chars().enumerate().map(move |(i, c)| KeyCap {
                label: c.to_string().into(),
                row: row as i32,
                x: i as f32 + row as f32 * 0.5,
                width: 1.0,
            })
        })
        .collect()
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let snapshot = args.iter().position(|a| a == "--snapshot").and_then(|i| args.get(i + 1)).cloned();

    let ui = GalleryWindow::new()?;
    let tiles = [
        ("Browser", "B", 0x2a4a8a),
        ("Music", "M", 0x7a2a5a),
        ("Photos", "P", 0x2a6a4a),
        ("Files", "F", 0x8a5a2a),
        ("Settings", "S", 0x4a4a5a),
    ];
    let model = Rc::new(VecModel::from(
        tiles
            .iter()
            .map(|(t, m, c)| tile(t, m, Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, *c as u8)))
            .collect::<Vec<_>>(),
    ));
    ui.set_tiles(ModelRc::from(model.clone()));
    let row = |label: &str, description: &str, value: &str, opens_page| SettingRow {
        label: label.into(),
        description: description.into(),
        value: value.into(),
        opens_page,
    };
    ui.set_rows(ModelRc::new(VecModel::from(vec![
        row("Animations", "Focus and page transitions", "On", false),
        row("Accent color", "Focus ring, buttons and keys", "White", false),
        row("Wi-Fi", "Home network", "Connected", true),
        row("Language", "Interface language", "English", false),
    ])));
    ui.set_keys(ModelRc::new(VecModel::from(keyboard())));
    ui.set_dialog_open(args.iter().any(|a| a == "--dialog"));

    let session = Session::detect();
    apply_theme(ui.global::<Theme>(), &session);
    if session.fullscreen() {
        ui.window().set_fullscreen(true);
    }

    let hints = |dialog: bool| {
        if dialog {
            Hints::new().confirm("Select").back("Cancel")
        } else {
            Hints::new().dpad("Select").confirm("OK").back("Quit")
        }
    };
    ui.set_hints(hint_model(hints(ui.get_dialog_open())));

    let audio = Rc::new(RefCell::new(Audio::start()));
    let handle = {
        let ui = ui.as_weak();
        let model = model.clone();
        let audio = audio.clone();
        move |action: Action| {
            let Some(ui) = ui.upgrade() else { return };
            let play = |sound| {
                if let Some(audio) = audio.borrow_mut().as_mut() {
                    audio.play(sound);
                }
            };
            let last = model.row_count() as i32 - 1;
            if ui.get_dialog_open() {
                match action {
                    Action::Confirm | Action::Back => {
                        ui.set_dialog_open(false);
                        ui.set_hints(hint_model(hints(false)));
                        play(Sound::Back);
                    }
                    _ => {}
                }
                return;
            }
            match action {
                Action::Left if ui.get_focused() > 0 => {
                    ui.set_focused(ui.get_focused() - 1);
                    play(Sound::Move);
                }
                Action::Right if ui.get_focused() < last => {
                    ui.set_focused(ui.get_focused() + 1);
                    play(Sound::Move);
                }
                Action::Up if ui.get_row_focus() > 0 => {
                    ui.set_row_focus(ui.get_row_focus() - 1);
                    play(Sound::Move);
                }
                Action::Down if ui.get_row_focus() < 3 => {
                    ui.set_row_focus(ui.get_row_focus() + 1);
                    play(Sound::Move);
                }
                Action::Confirm => {
                    ui.set_dialog_open(true);
                    ui.set_hints(hint_model(hints(true)));
                    play(Sound::Confirm);
                }
                // Closing the window returns to the shell
                Action::Back => {
                    let _ = slint::quit_event_loop();
                }
                _ => {}
            }
        }
    };
    let handle = Rc::new(RefCell::new(handle));

    // Keyboard: the InputScope reports NavActions
    let input: Rc<RefCell<Option<Input>>> = Rc::default();
    ui.on_action({
        let handle = handle.clone();
        let input = input.clone();
        move |action| {
            if let Some(input) = input.borrow().as_ref() {
                input.keyboard_used();
            }
            (handle.borrow_mut())(nav_action(action));
        }
    });
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    // Controllers: polled while the window has the focus
    *input.borrow_mut() = Some(Input::start(
        {
            let ui = ui.as_weak();
            move || ui.upgrade().is_some_and(|ui| ui.get_active())
        },
        {
            let ui = ui.as_weak();
            let handle = handle.clone();
            move |event| match event {
                Event::Action(action) => (handle.borrow_mut())(action),
                Event::Glyphs(glyphs) => {
                    if let Some(ui) = ui.upgrade() {
                        ui.set_style(controller_style(glyphs));
                    }
                }
                Event::Controller(connection) => println!("Controller: {connection:?}"),
            }
        },
    ));

    // `--down` / `--up` move the list focus right away, for snapshots in the middle of the animation
    for (flag, action) in [("--down", Action::Down), ("--up", Action::Up)] {
        if args.iter().any(|a| a == flag) {
            (handle.borrow_mut())(action);
        }
    }
    let delay = args.iter().position(|a| a == "--delay").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok());

    let timer = Timer::default();
    if let Some(path) = snapshot {
        let ui = ui.as_weak();
        timer.start(TimerMode::SingleShot, Duration::from_millis(delay.unwrap_or(1200)), move || {
            if let Some(ui) = ui.upgrade() {
                match ui.window().take_snapshot().map_err(Box::<dyn Error>::from).and_then(|px| save_png(&path, &px)) {
                    Ok(()) => println!("Snapshot saved: {path}"),
                    Err(err) => eprintln!("Snapshot failed: {err}"),
                }
            }
            let _ = slint::quit_event_loop();
        });
    }
    ui.run()?;
    Ok(())
}

fn save_png(path: &str, px: &SharedPixelBuffer<slint::Rgba8Pixel>) -> Result<(), Box<dyn Error>> {
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path)?), px.width(), px.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(px.as_bytes())?;
    Ok(())
}
