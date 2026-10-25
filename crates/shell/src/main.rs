use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;
use std::time::{Duration, Instant};

use consoleui_core::input::GlyphStyle;
use consoleui_core::settings::{GlyphChoice, Settings};
use consoleui_core::status::{self, Network};
use consoleui_core::Action;
use slint::{RenderingState, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode};

mod audio;
mod cover;
mod gamepad;
mod gamescope;
mod grab;
mod loader;
mod options;
mod perf;
mod quick;
mod shell;
mod tiles;

use gamepad::{Connection, Gamepads};
use loader::LoadOptions;
use options::{Options, ScriptStep, StartScreen, USAGE};
use perf::FrameStats;
use shell::{GamescopeUi, Shell};

slint::include_modules!();

const BENCH_STEP: Duration = Duration::from_millis(200);
/// Abfrage-Intervall für Gamepads (gilrs liefert Events nur auf Nachfrage).
const GAMEPAD_POLL: Duration = Duration::from_millis(8);
const SNAPSHOT_DELAY: Duration = Duration::from_millis(1500);
const SCRIPT_START_DELAY: Duration = Duration::from_millis(800);
/// Die Boot-Animation steht mindestens so lange, auch wenn die Bibliothek schneller geladen ist.
const MIN_BOOT: Duration = Duration::from_millis(1200);
const LOADER_POLL: Duration = Duration::from_millis(30);
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

    // Die Bibliothek lädt im Hintergrund, während die Boot-Animation läuft.
    let library = loader::spawn(LoadOptions {
        dummy: options.dummy,
        tiles: options.tiles,
        dummy_launch: options.dummy_launch.clone(),
    });
    let boot_started = Instant::now();

    // In der gamescope-Session (Steam-Modus) steuert die Shell die Fenster selbst und zeigt das
    // Quick-Menü als eigenes Overlay-Fenster.
    let gamescope = match gamescope::Gamescope::connect() {
        Some(control) => Some(GamescopeUi::new(control, OverlayWindow::new()?)),
        None => None,
    };
    let overlay = gamescope.as_ref().map(|g| g.overlay.as_weak());
    let settings_path = Settings::default_path();
    let settings = settings_path.as_deref().map(Settings::load).unwrap_or_default();
    let shell = Rc::new(RefCell::new(Shell::new(&ui, gamescope, settings, settings_path)));
    if let Some(overlay) = overlay.and_then(|o| o.upgrade()) {
        overlay.on_action({
            let ui = ui.as_weak();
            let shell = shell.clone();
            move |nav| {
                if let Some(ui) = ui.upgrade() {
                    shell.borrow_mut().apply(&ui, to_action(nav));
                }
            }
        });
    }
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
            let mut grab: Option<grab::Grab> = None;
            move || {
                let Some(ui) = ui.upgrade() else { return };
                // Steuerzentrale offen: Controller exklusiv lesen, damit das Spiel nichts mitbekommt.
                let exclusive = shell.borrow().wants_exclusive_input();
                if exclusive && grab.is_none() {
                    grab = Some(grab::Grab::acquire(&gamepads.borrow().names()));
                } else if !exclusive && grab.take().is_some() {
                    gamepads.borrow_mut().reset();
                }
                let now = Instant::now();
                let actions = match grab.as_mut().filter(|g| !g.is_empty()) {
                    Some(grab) => grab.poll(now),
                    None => gamepads.borrow_mut().poll(now),
                };
                for connection in gamepads.borrow_mut().take_connections() {
                    let (title, name) = match connection {
                        Connection::Connected(name) => ("Controller verbunden", name),
                        Connection::Disconnected(name) => ("Controller getrennt", name),
                    };
                    shell.borrow_mut().toast(&ui, "◉", title, &name);
                }
                for action in actions {
                    let accepted = shell.borrow().accepts_gamepad(&ui, action);
                    if accepted {
                        shell.borrow_mut().apply(&ui, action);
                    }
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
            let battery = gamepads.as_ref().and_then(|g| g.borrow().status()).and_then(|c| c.battery);
            shell.borrow_mut().check_controller_battery(&ui, battery);
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

    // Sobald die Bibliothek da ist: übernehmen, Boot-Animation ausblenden und erst dann
    // Benchmark, Skript und Snapshot starten (sie sollen die fertige Oberfläche sehen).
    let after_boot: Rc<RefCell<Vec<Timer>>> = Rc::default();
    let loader_timer = Timer::default();
    loader_timer.start(TimerMode::Repeated, LOADER_POLL, {
        let ui = ui.as_weak();
        let shell = shell.clone();
        let stats = stats.clone();
        let after_boot = after_boot.clone();
        let mut tasks = Some(ReadyTasks::from(&options));
        move || {
            // Nur einmal: danach ist `tasks` leer und der Timer tut nichts mehr.
            if tasks.is_none() {
                return;
            }
            let Ok(library) = library.try_recv() else { return };
            let (Some(ready), Some(ui)) = (tasks.take(), ui.upgrade()) else { return };
            shell.borrow_mut().set_library(&ui, library);
            shell.borrow_mut().open(&ui, ready.screen);
            if let Some(query) = &ready.query {
                shell.borrow_mut().type_text(&ui, query);
            }
            let ui = ui.as_weak();
            let shell = shell.clone();
            let stats = stats.clone();
            let after_boot = after_boot.clone();
            Timer::single_shot(MIN_BOOT.saturating_sub(boot_started.elapsed()), move || {
                let Some(ui) = ui.upgrade() else { return };
                ui.set_booting(false);
                after_boot.borrow_mut().extend(ready.start(&ui, shell, stats));
            });
        }
    });

    if !options.windowed {
        ui.window().set_fullscreen(true);
    }
    ui.run()?;
    Ok(())
}

/// Was nach dem Booten passieren soll (aus den Kommandozeilen-Optionen)
struct ReadyTasks {
    screen: Screen,
    query: Option<String>,
    bench: bool,
    actions: Vec<ScriptStep>,
    snapshot: Option<std::path::PathBuf>,
}

impl From<&Options> for ReadyTasks {
    fn from(options: &Options) -> Self {
        let screen = match options.start {
            StartScreen::Home => Screen::Home,
            StartScreen::Library => Screen::Library,
            StartScreen::Settings => Screen::Settings,
            StartScreen::Search => Screen::Search,
        };
        Self {
            screen,
            query: options.query.clone(),
            bench: options.bench,
            actions: options.actions.clone(),
            snapshot: options.snapshot.clone(),
        }
    }
}

impl ReadyTasks {
    /// Startet Benchmark, Skript und Snapshot; die Timer müssen am Leben bleiben.
    fn start(self, ui: &AppWindow, shell: Rc<RefCell<Shell>>, stats: Rc<RefCell<FrameStats>>) -> Vec<Timer> {
        let mut timers = Vec::new();
        if self.bench {
            timers.push(start_bench(ui, shell.clone(), stats));
        }
        if !self.actions.is_empty() {
            timers.push(run_script(ui, shell, self.actions));
        }
        if let Some(path) = self.snapshot {
            let timer = Timer::default();
            let ui = ui.as_weak();
            // Etwas warten, bis Seitenwechsel und Hero-Überblendung fertig sind.
            timer.start(TimerMode::SingleShot, SNAPSHOT_DELAY, move || {
                if let Some(ui) = ui.upgrade() {
                    match ui.window().take_snapshot().map_err(Box::<dyn Error>::from).and_then(|px| save_png(&path, &px)) {
                        Ok(()) => println!("Snapshot gespeichert: {}", path.display()),
                        Err(err) => eprintln!("Snapshot fehlgeschlagen: {err}"),
                    }
                }
                let _ = slint::quit_event_loop();
            });
            timers.push(timer);
        }
        timers
    }
}

/// Spielt `--actions` ab (für Tests ohne echte Eingabe). Die Aktionen gehen direkt an die Shell,
/// unabhängig vom Fensterfokus.
fn run_script(ui: &AppWindow, shell: Rc<RefCell<Shell>>, steps: Vec<ScriptStep>) -> Timer {
    let ui = ui.as_weak();
    let mut next = 0;
    // Kurz warten, bis das Fenster steht
    let mut due = Instant::now() + SCRIPT_START_DELAY;
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(20), move || {
        let Some(ui) = ui.upgrade() else { return };
        if Instant::now() < due {
            return;
        }
        match steps.get(next) {
            Some(ScriptStep::Wait(ms)) => due = Instant::now() + Duration::from_millis(*ms),
            Some(ScriptStep::Action(action)) => shell.borrow_mut().apply(&ui, *action),
            None => return,
        }
        next += 1;
    });
    timer
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
        NavAction::Guide => Action::Guide,
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
    let style = match (shell.glyph_choice(), controller.map(|c| c.style)) {
        (GlyphChoice::PlayStation, _) | (GlyphChoice::Auto, Some(GlyphStyle::PlayStation)) => ControllerStyle::Playstation,
        (GlyphChoice::Xbox, _) | (GlyphChoice::Auto, Some(GlyphStyle::Xbox)) => ControllerStyle::Xbox,
        (GlyphChoice::Auto, None) => ControllerStyle::Keyboard,
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
