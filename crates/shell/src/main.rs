use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;
use std::time::{Duration, Instant};

use consoleui_core::{Action, AppEntry, AppKind, Carousel, dummy_library};
use slint::{Color, ModelRc, RenderingState, Timer, TimerMode, VecModel};

mod cover;
mod options;
mod perf;

use options::{Options, USAGE};
use perf::FrameStats;

slint::include_modules!();

const COVER_SIZE: u32 = 384;
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

    let library = dummy_library(options.tiles);
    let ui = AppWindow::new()?;
    let tiles: Vec<AppTile> = library.iter().enumerate().map(|(i, entry)| to_tile(entry, i)).collect();
    ui.set_tiles(ModelRc::new(VecModel::from(tiles)));
    ui.set_show_fps(options.show_fps || options.bench);

    let carousel = Rc::new(RefCell::new(Carousel::new(library.len())));
    ui.on_action({
        let ui = ui.as_weak();
        let carousel = carousel.clone();
        move |nav| {
            if let Some(ui) = ui.upgrade() {
                apply_action(&ui, &mut carousel.borrow_mut(), to_action(nav));
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

    let _bench = options.bench.then(|| start_bench(&ui, carousel.clone(), stats.clone()));

    if !options.windowed {
        ui.window().set_fullscreen(true);
    }
    ui.run()?;
    Ok(())
}

fn to_tile(entry: &AppEntry, index: usize) -> AppTile {
    let (kind, action_label) = match entry.kind {
        AppKind::Game => ("SPIEL", "Spielen"),
        AppKind::App => ("APP", "Öffnen"),
    };
    AppTile {
        title: entry.title.as_str().into(),
        subtitle: entry.subtitle.as_str().into(),
        kind: kind.into(),
        action_label: action_label.into(),
        monogram: entry.monogram().into(),
        cover: cover::generate(entry.accent, entry.accent2, index as u32, COVER_SIZE),
        accent: Color::from_rgb_u8(entry.accent[0], entry.accent[1], entry.accent[2]),
    }
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

fn apply_action(ui: &AppWindow, carousel: &mut Carousel, action: Action) {
    match action {
        // Das eigentliche Starten der App folgt in Sprint 2 (Story 1.5).
        Action::Confirm => ui.set_pulse(true),
        _ => {
            if carousel.apply(action) {
                ui.set_focused(carousel.focused() as i32);
            }
        }
    }
}

fn current_time() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

/// Scrollt einmal ganz nach rechts und zurück, gibt danach die Messwerte aus und beendet sich.
fn start_bench(ui: &AppWindow, carousel: Rc<RefCell<Carousel>>, stats: Rc<RefCell<FrameStats>>) -> Timer {
    let len = carousel.borrow().len();
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
        apply_action(&ui, &mut carousel.borrow_mut(), action);
        step += 1;
    });
    timer
}
