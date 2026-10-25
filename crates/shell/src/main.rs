use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;
use std::time::{Duration, Instant};

use consoleui_core::{Action, AppEntry, History, apply_history, discover_library, dummy_library, sort_by_recent};
use slint::{RenderingState, Timer, TimerMode};

mod cover;
mod options;
mod perf;
mod shell;
mod tiles;

use options::{Options, USAGE};
use perf::FrameStats;
use shell::Shell;

slint::include_modules!();

const BENCH_STEP: Duration = Duration::from_millis(200);

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

    ui.set_clock(current_time().into());
    let tick = Timer::default();
    tick.start(TimerMode::Repeated, Duration::from_secs(1), {
        let ui = ui.as_weak();
        let stats = stats.clone();
        move || {
            let Some(ui) = ui.upgrade() else { return };
            ui.set_clock(current_time().into());
            if ui.get_show_fps() {
                ui.set_fps_text(perf::format_second(&stats.borrow_mut().take_current()).into());
            }
        }
    });

    let _bench = options.bench.then(|| start_bench(&ui, shell.clone(), stats.clone()));

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

fn to_action(nav: NavAction) -> Action {
    match nav {
        NavAction::Up => Action::Up,
        NavAction::Down => Action::Down,
        NavAction::Left => Action::Left,
        NavAction::Right => Action::Right,
        NavAction::Confirm => Action::Confirm,
        NavAction::Back => Action::Back,
        NavAction::Menu => Action::Menu,
    }
}

fn current_time() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

/// Scrollt einmal ganz nach rechts und zurück, gibt danach die Messwerte aus und beendet sich.
fn start_bench(ui: &AppWindow, shell: Rc<RefCell<Shell>>, stats: Rc<RefCell<FrameStats>>) -> Timer {
    let len = shell.borrow().len();
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
