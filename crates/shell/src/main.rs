use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;
use std::time::{Duration, Instant};

use consoleui_core::input::GlyphStyle;
use consoleui_core::status::{self, Network};
use consoleui_core::{Action, AppEntry, History, apply_history, discover_library, dummy_library, sort_by_recent};
use slint::{RenderingState, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode};

mod cover;
mod gamepad;
mod options;
mod perf;
mod shell;
mod tiles;

use gamepad::Gamepads;
use options::{Options, StartScreen, USAGE};
use perf::FrameStats;
use shell::Shell;

slint::include_modules!();

const BENCH_STEP: Duration = Duration::from_millis(200);
/// Abfrage-Intervall für Gamepads (gilrs liefert Events nur auf Nachfrage).
const GAMEPAD_POLL: Duration = Duration::from_millis(8);
const SNAPSHOT_DELAY: Duration = Duration::from_millis(1500);
/// Wie oft geprüft wird, ob die gestartete App noch läuft.
const WATCH_INTERVAL: Duration = Duration::from_millis(500);

fn main() -> Result<(), Box<dyn Error>> {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    if options.help {
        println!("{USAGE}");
        return Ok(());
    }

    let ui = AppWindow::new()?;
    ui.set_show_fps(options.show_fps || options.bench);

    let (entries, history, history_path) = load_library(&options);
    let shell = Rc::new(RefCell::new(Shell::new(&ui, entries, history, history_path)));
    ui.on_action({
        let ui = ui.as_weak();
        let shell = shell.clone();
        move |nav| {
            if let Some(ui) = ui.upgrade() {
                shell.borrow_mut().apply(&ui, to_action(nav));
            }
        }
    });
    ui.on_typed({
        let ui = ui.as_weak();
        let shell = shell.clone();
        move |text| ui.upgrade().is_some_and(|ui| shell.borrow_mut().type_text(&ui, &text))
    });
    ui.on_erase({
        let ui = ui.as_weak();
        let shell = shell.clone();
        move || {
            if let Some(ui) = ui.upgrade() {
                shell.borrow_mut().erase_text(&ui);
            }
        }
    });
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    let stats = Rc::new(RefCell::new(FrameStats::default()));
    if ui.get_show_fps() {
        let stats = stats.clone();
        ui.window().set_rendering_notifier(move |state, _| match state {
            RenderingState::BeforeRendering => stats.borrow_mut().frame_started(),
            RenderingState::AfterRendering => stats.borrow_mut().frame_finished(),
            _ => {}
        })?;
    }

    let gamepads = Gamepads::new().map(|g| Rc::new(RefCell::new(g)));
    let gamepad_timer = Timer::default();
    if let Some(gamepads) = &gamepads {
        gamepad_timer.start(TimerMode::Repeated, GAMEPAD_POLL, {
            let ui = ui.as_weak();
            let shell = shell.clone();
            let gamepads = gamepads.clone();
            move || {
                let Some(ui) = ui.upgrade() else { return };
                let actions = gamepads.borrow_mut().poll(Instant::now());
                // gilrs liest die Geräte direkt; ohne Fensterfokus gehören die Eingaben dem Spiel.
                if !ui.get_window_active() {
                    return;
                }
                for action in actions {
                    shell.borrow_mut().apply(&ui, action);
                }
            }
        });
    }

    update_status(&ui, gamepads.as_deref(), &shell.borrow());
    let tick = Timer::default();
    tick.start(TimerMode::Repeated, Duration::from_secs(1), {
        let ui = ui.as_weak();
        let stats = stats.clone();
        let gamepads = gamepads.clone();
        let shell = shell.clone();
        move || {
            let Some(ui) = ui.upgrade() else { return };
            update_status(&ui, gamepads.as_deref(), &shell.borrow());
            if ui.get_show_fps() {
                ui.set_fps_text(perf::format_second(&stats.borrow_mut().take_current()).into());
            }
        }
    });

    let watch = Timer::default();
    watch.start(TimerMode::Repeated, WATCH_INTERVAL, {
        let ui = ui.as_weak();
        let shell = shell.clone();
        move || {
            if let Some(ui) = ui.upgrade() {
                shell.borrow_mut().watch_running(&ui);
            }
        }
    });

    let _bench = options.bench.then(|| start_bench(&ui, shell.clone(), stats.clone()));

    let screen = match options.start {
        StartScreen::Home => Screen::Home,
        StartScreen::Library => Screen::Library,
        StartScreen::Search => Screen::Search,
    };
    shell.borrow_mut().open(&ui, screen);
    if let Some(query) = &options.query {
        shell.borrow_mut().type_text(&ui, query);
    }
    let snapshot_timer = Timer::default();
    if let Some(path) = options.snapshot.clone() {
        // Etwas warten, bis Seitenwechsel und Hero-Überblendung fertig sind.
        snapshot_timer.start(TimerMode::SingleShot, SNAPSHOT_DELAY, {
            let ui = ui.as_weak();
            move || {
                if let Some(ui) = ui.upgrade() {
                    match ui.window().take_snapshot().map_err(Box::<dyn Error>::from).and_then(|px| save_png(&path, &px)) {
                        Ok(()) => println!("Snapshot gespeichert: {}", path.display()),
                        Err(err) => eprintln!("Snapshot fehlgeschlagen: {err}"),
                    }
                }
                let _ = slint::quit_event_loop();
            }
        });
    }

    if !options.windowed {
        ui.window().set_fullscreen(true);
    }
    ui.run()?;
    Ok(())
}

/// Echte Bibliothek, sortiert nach letzter Nutzung. Ist nichts installiert, gibt es Dummy-Einträge.
fn load_library(options: &Options) -> (Vec<AppEntry>, History, Option<std::path::PathBuf>) {
    if !options.dummy {
        let history_path = History::default_path();
        let history = history_path.as_deref().map(History::load).unwrap_or_default();
        let mut entries = discover_library();
        if !entries.is_empty() {
            apply_history(&mut entries, &history);
            sort_by_recent(&mut entries);
            return (entries, history, history_path);
        }
        eprintln!("Keine Spiele oder Apps gefunden, zeige Dummy-Einträge.");
    }
    (dummy_library(options.tiles), History::default(), None)
}

fn save_png(path: &std::path::Path, pixels: &SharedPixelBuffer<Rgba8Pixel>) -> Result<(), Box<dyn Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, pixels.width(), pixels.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    Ok(())
}

fn to_action(nav: NavAction) -> Action {
    match nav {
        NavAction::Up => Action::Up,
        NavAction::Down => Action::Down,
        NavAction::Left => Action::Left,
        NavAction::Right => Action::Right,
        NavAction::Confirm => Action::Confirm,
        NavAction::Back => Action::Back,
        NavAction::Menu => Action::Menu,
        NavAction::Search => Action::Search,
        NavAction::Secondary => Action::Secondary,
        NavAction::PrevTab => Action::PrevTab,
        NavAction::NextTab => Action::NextTab,
    }
}

/// Uhr, Netzwerk, Akku und Controller in der Top-Bar sowie die passenden Tastensymbole.
fn update_status(ui: &AppWindow, gamepads: Option<&RefCell<Gamepads>>, shell: &Shell) {
    ui.set_clock(chrono::Local::now().format("%H:%M").to_string().into());

    let system = status::read();
    ui.set_network(match system.network {
        Network::Offline => NetworkState::Offline,
        Network::Ethernet => NetworkState::Ethernet,
        Network::Wifi(_) => NetworkState::Wifi,
    });
    ui.set_battery_percent(system.battery.map_or(-1, |b| b.percent.into()));
    ui.set_battery_charging(system.battery.is_some_and(|b| b.charging));

    let controller = gamepads.and_then(|g| g.borrow().status());
    ui.set_controller_connected(controller.is_some());
    ui.set_controller_battery(controller.as_ref().and_then(|c| c.battery).map_or(-1, i32::from));
    let style = match controller.map(|c| c.style) {
        Some(GlyphStyle::PlayStation) => ControllerStyle::Playstation,
        Some(GlyphStyle::Xbox) => ControllerStyle::Xbox,
        None => ControllerStyle::Keyboard,
    };
    if ui.get_controller_style() != style {
        ui.set_controller_style(style);
        shell.update_hints(ui);
    }
}

/// Scrollt einmal ganz nach rechts und zurück, gibt danach die Messwerte aus und beendet sich.
fn start_bench(ui: &AppWindow, shell: Rc<RefCell<Shell>>, stats: Rc<RefCell<FrameStats>>) -> Timer {
    let len = shell.borrow().home_len();
    let steps = 2 * len.saturating_sub(1);
    let mut step = 0;
    let mut started: Option<Instant> = None;
    let ui = ui.as_weak();

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, BENCH_STEP, move || {
        let Some(ui) = ui.upgrade() else { return };
        let started = *started.get_or_insert_with(|| {
            stats.borrow_mut().reset_total();
            Instant::now()
        });
        if step >= steps {
            perf::print_bench_summary(len, steps, started.elapsed(), stats.borrow().total());
            let _ = slint::quit_event_loop();
            return;
        }
        let action = if step + 1 < len { Action::Right } else { Action::Left };
        shell.borrow_mut().apply(&ui, action);
        step += 1;
    });
    timer
}
