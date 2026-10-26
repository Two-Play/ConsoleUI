use std::cell::{Cell, RefCell};
use std::error::Error;
use std::rc::Rc;
use std::time::{Duration, Instant};

use consoleui_core::Action;
use consoleui_core::i18n::{self, tr};
use consoleui_core::input::{self, GlyphStyle, InputDevice};
use consoleui_core::status::{self, Network};
use slint::{RenderingState, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode};

mod audio;
mod backdrop;
mod compositor;
mod context;
mod cover;
mod dbus;
mod gamepad;
mod gamescope;
mod grab;
mod hero;
mod loader;
mod music;
mod network;
mod options;
mod perf;
mod profiles;
mod quick;
mod shell;
mod tiles;

use gamepad::{Connection, Gamepads};
use loader::LoadOptions;
use options::{Options, ScriptStep, StartScreen, USAGE};
use perf::FrameStats;
use profiles::ProfileStart;
use shell::{CompositorUi, GamescopeUi, Shell};

slint::include_modules!();

const BENCH_STEP: Duration = Duration::from_millis(200);
/// Poll interval for gamepads (gilrs delivers events only on request).
const GAMEPAD_POLL: Duration = Duration::from_millis(8);
const SNAPSHOT_DELAY: Duration = Duration::from_millis(1500);
const SCRIPT_START_DELAY: Duration = Duration::from_millis(800);
const SCRIPT_TICK: Duration = Duration::from_millis(20);
const SNAPSHOT_AFTER_SCRIPT: Duration = Duration::from_millis(700);
/// The boot animation stays at least this long, even if the library loads faster.
const MIN_BOOT: Duration = Duration::from_millis(1200);
const LOADER_POLL: Duration = Duration::from_millis(30);
/// How often to check whether the launched app is still running.
const WATCH_INTERVAL: Duration = Duration::from_millis(500);
/// How often responses of the network thread (Wi-Fi/Bluetooth) are collected
const NETWORK_POLL: Duration = Duration::from_millis(100);
/// How often messages of our own compositor are collected
const COMPOSITOR_POLL: Duration = Duration::from_millis(50);

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

    // The language decides the desktop entry names the loader reads, so it is set first. It comes
    // from the settings of the profile chosen last (or given with `--profile`, Story 5.7).
    let start = ProfileStart::load(options.profile.as_deref());
    let language = start.settings.language.resolve();
    i18n::set_language(language);

    let ui = AppWindow::new()?;
    ui.set_show_fps(options.show_fps || options.bench);
    // Slint's translations exist only once a component has been created.
    shell::select_language(language);

    // The library loads in the background while the boot animation runs.
    let library = loader::spawn(LoadOptions {
        dummy: options.dummy,
        tiles: options.tiles,
        dummy_launch: options.dummy_launch.clone(),
        artwork: start.settings.artwork,
    });
    let boot_started = Instant::now();

    // Inside our own compositor (Epic 7) the control socket decides what is in front; the quick
    // menu and toasts get their own windows, which the compositor stacks above the app (Story
    // 7.6). In the gamescope session (Steam mode) the shell controls the windows itself and shows
    // the quick menu as its own overlay window.
    let compositor = compositor::Compositor::connect();
    let compositor_ui = match compositor {
        Some(_) => Some(CompositorUi::new(OverlayWindow::new()?, NotificationWindow::new()?)),
        None => None,
    };
    let gamescope = match compositor.is_none().then(gamescope::Gamescope::connect).flatten() {
        Some(control) => Some(GamescopeUi::new(control, OverlayWindow::new()?, NotificationWindow::new()?)),
        None => None,
    };
    let overlay =
        gamescope.as_ref().map(|g| g.overlay.as_weak()).or_else(|| compositor_ui.as_ref().map(|c| c.overlay.as_weak()));
    let shell = Rc::new(RefCell::new(Shell::new(&ui, gamescope, compositor, compositor_ui, start)));
    let gamepads = Gamepads::new().map(|g| Rc::new(RefCell::new(g)));
    // The hints show the buttons of the device used last (keyboard keys after keyboard input).
    let last_input = Rc::new(Cell::new(InputDevice::Gamepad));
    let keyboard_used = {
        let gamepads = gamepads.clone();
        let shell = shell.clone();
        let last_input = last_input.clone();
        move |ui: &AppWindow| input_used(ui, gamepads.as_deref(), &shell.borrow(), &last_input, InputDevice::Keyboard)
    };
    if let Some(overlay) = overlay.and_then(|o| o.upgrade()) {
        overlay.on_action({
            let ui = ui.as_weak();
            let shell = shell.clone();
            let keyboard_used = keyboard_used.clone();
            move |nav| {
                if let Some(ui) = ui.upgrade() {
                    keyboard_used(&ui);
                    shell.borrow_mut().apply(&ui, to_action(nav));
                }
            }
        });
    }
    ui.on_action({
        let ui = ui.as_weak();
        let shell = shell.clone();
        let keyboard_used = keyboard_used.clone();
        move |nav| {
            if let Some(ui) = ui.upgrade() {
                keyboard_used(&ui);
                shell.borrow_mut().apply(&ui, to_action(nav));
            }
        }
    });
    ui.on_typed({
        let ui = ui.as_weak();
        let shell = shell.clone();
        let keyboard_used = keyboard_used.clone();
        move |text| {
            ui.upgrade().is_some_and(|ui| {
                keyboard_used(&ui);
                shell.borrow_mut().type_text(&ui, &text)
            })
        }
    });
    ui.on_erase({
        let ui = ui.as_weak();
        let shell = shell.clone();
        move || {
            if let Some(ui) = ui.upgrade() {
                keyboard_used(&ui);
                shell.borrow_mut().erase_text(&ui);
            }
        }
    });
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    let stats = Rc::new(RefCell::new(FrameStats::default()));
    let notifier = ui.window().set_rendering_notifier({
        let stats = stats.clone();
        let shell = shell.clone();
        let show_fps = ui.get_show_fps();
        move |state, _| match state {
            RenderingState::BeforeRendering if show_fps => stats.borrow_mut().frame_started(),
            RenderingState::AfterRendering => {
                if show_fps {
                    stats.borrow_mut().frame_finished();
                }
                // Not while the shell is busy (a snapshot renders from within a timer)
                if let Ok(mut shell) = shell.try_borrow_mut() {
                    shell.frame_rendered();
                }
            }
            _ => {}
        }
    });
    if ui.get_show_fps() {
        notifier?;
    }

    // Story 7.13: after each frame of the quick menu or a toast the compositor learns where the
    // panel is now, so its blur moves with the sliding panel.
    if let Some((overlay, notification)) = shell.borrow().blur_windows() {
        let notifier = |shell: Rc<RefCell<Shell>>| {
            move |state: RenderingState, _: &slint::GraphicsAPI| {
                if let RenderingState::AfterRendering = state
                    && let Ok(mut shell) = shell.try_borrow_mut()
                {
                    shell.sync_blur();
                }
            }
        };
        if let Some(overlay) = overlay.upgrade()
            && let Err(err) = overlay.window().set_rendering_notifier(notifier(shell.clone()))
        {
            eprintln!("Cannot follow the quick menu for the blur: {err}");
        }
        if let Some(notification) = notification.upgrade()
            && let Err(err) = notification.window().set_rendering_notifier(notifier(shell.clone()))
        {
            eprintln!("Cannot follow toasts for the blur: {err}");
        }
    }

    let gamepad_timer = Timer::default();
    if let Some(gamepads) = &gamepads {
        gamepad_timer.start(TimerMode::Repeated, GAMEPAD_POLL, {
            let ui = ui.as_weak();
            let shell = shell.clone();
            let gamepads = gamepads.clone();
            let last_input = last_input.clone();
            let mut grab: Option<grab::Grab> = None;
            move || {
                let Some(ui) = ui.upgrade() else { return };
                // Control center open: read controllers exclusively so the game notices nothing.
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
                        Connection::Connected(name) => (tr("Controller connected"), name),
                        Connection::Disconnected(name) => (tr("Controller disconnected"), name),
                    };
                    shell.borrow_mut().toast(&ui, "◉", title, &name);
                }
                for action in actions {
                    let accepted = shell.borrow().accepts_gamepad(&ui, action);
                    if accepted {
                        input_used(&ui, Some(&gamepads), &shell.borrow(), &last_input, InputDevice::Gamepad);
                        shell.borrow_mut().apply(&ui, action);
                    }
                }
            }
        });
    }

    update_status(&ui, gamepads.as_deref(), &shell.borrow(), last_input.get());
    let tick = Timer::default();
    tick.start(TimerMode::Repeated, Duration::from_secs(1), {
        let ui = ui.as_weak();
        let stats = stats.clone();
        let gamepads = gamepads.clone();
        let shell = shell.clone();
        move || {
            let Some(ui) = ui.upgrade() else { return };
            update_status(&ui, gamepads.as_deref(), &shell.borrow(), last_input.get());
            let battery = gamepads.as_ref().and_then(|g| g.borrow().status()).and_then(|c| c.battery);
            shell.borrow_mut().check_controller_battery(&ui, battery);
            if ui.get_show_fps() {
                ui.set_fps_text(perf::format_second(&stats.borrow_mut().take_current()).into());
            }
        }
    });

    shell.borrow_mut().start_network(options.fake_network);
    let network_timer = Timer::default();
    network_timer.start(TimerMode::Repeated, NETWORK_POLL, {
        let ui = ui.as_weak();
        let shell = shell.clone();
        move || {
            if let Some(ui) = ui.upgrade() {
                shell.borrow_mut().poll_network(&ui);
            }
        }
    });

    let compositor_timer = Timer::default();
    compositor_timer.start(TimerMode::Repeated, COMPOSITOR_POLL, {
        let ui = ui.as_weak();
        let shell = shell.clone();
        move || {
            if let Some(ui) = ui.upgrade() {
                shell.borrow_mut().poll_compositor(&ui);
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

    // Once the library is there: take it over, hide the boot animation and only then start the
    // benchmark, script and snapshot (they should see the finished UI).
    let after_boot: Rc<RefCell<Vec<Timer>>> = Rc::default();
    let loader_timer = Timer::default();
    loader_timer.start(TimerMode::Repeated, LOADER_POLL, {
        let ui = ui.as_weak();
        let shell = shell.clone();
        let stats = stats.clone();
        let after_boot = after_boot.clone();
        let mut tasks = Some(ReadyTasks::from(&options));
        move || {
            // Only once: afterwards `tasks` is empty and the timer does nothing.
            if tasks.is_none() {
                return;
            }
            let Ok(library) = library.try_recv() else { return };
            let (Some(ready), Some(ui)) = (tasks.take(), ui.upgrade()) else { return };
            shell.borrow_mut().set_library(&ui, library);
            shell.borrow_mut().open(&ui, ready.screen);
            // "Who's playing?" when several profiles exist (or with `--start profiles`)
            let several = shell.borrow().profile_count() > 1;
            if ready.profiles == ProfilePrompt::Always || (ready.profiles == ProfilePrompt::IfSeveral && several) {
                shell.borrow_mut().open_profiles(&ui, true);
            }
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

/// Whether the profile screen opens after booting
#[derive(PartialEq)]
enum ProfilePrompt {
    Always,
    IfSeveral,
    Never,
}

/// What should happen after booting (from the command-line options)
struct ReadyTasks {
    screen: Screen,
    profiles: ProfilePrompt,
    query: Option<String>,
    bench: bool,
    actions: Vec<ScriptStep>,
    snapshot: Option<std::path::PathBuf>,
}

impl From<&Options> for ReadyTasks {
    fn from(options: &Options) -> Self {
        let screen = match options.start {
            StartScreen::Home | StartScreen::Profiles => Screen::Home,
            StartScreen::Library => Screen::Library,
            StartScreen::Settings => Screen::Settings,
            StartScreen::Search => Screen::Search,
        };
        // A profile given on the command line, another start page or the benchmark skip the question.
        let profiles = match options.start {
            StartScreen::Profiles => ProfilePrompt::Always,
            StartScreen::Home if options.profile.is_none() && !options.bench => ProfilePrompt::IfSeveral,
            _ => ProfilePrompt::Never,
        };
        Self {
            screen,
            profiles,
            query: options.query.clone(),
            bench: options.bench,
            actions: options.actions.clone(),
            snapshot: options.snapshot.clone(),
        }
    }
}

impl ReadyTasks {
    /// Starts benchmark, script and snapshot; the timers must stay alive.
    fn start(self, ui: &AppWindow, shell: Rc<RefCell<Shell>>, stats: Rc<RefCell<FrameStats>>) -> Vec<Timer> {
        let mut timers = Vec::new();
        // With a script, take the picture only after its last step (plus time for the animations)
        let snapshot_delay = SNAPSHOT_DELAY.max(script_duration(&self.actions) + SNAPSHOT_AFTER_SCRIPT);
        if self.bench {
            timers.push(start_bench(ui, shell.clone(), stats));
        }
        if !self.actions.is_empty() {
            timers.push(run_script(ui, shell, self.actions));
        }
        if let Some(path) = self.snapshot {
            let timer = Timer::default();
            let ui = ui.as_weak();
            // Wait a bit until the page change and hero crossfade are done.
            timer.start(TimerMode::SingleShot, snapshot_delay, move || {
                if let Some(ui) = ui.upgrade() {
                    match ui
                        .window()
                        .take_snapshot()
                        .map_err(Box::<dyn Error>::from)
                        .and_then(|px| save_png(&path, &px))
                    {
                        Ok(()) => println!("Snapshot saved: {}", path.display()),
                        Err(err) => eprintln!("Snapshot failed: {err}"),
                    }
                }
                let _ = slint::quit_event_loop();
            });
            timers.push(timer);
        }
        timers
    }
}

/// Approximate duration of an `--actions` script (start delay, waits, one tick per step)
fn script_duration(steps: &[ScriptStep]) -> Duration {
    if steps.is_empty() {
        return Duration::ZERO;
    }
    let waits: u64 = steps.iter().map(|s| if let ScriptStep::Wait(ms) = s { *ms } else { 0 }).sum();
    SCRIPT_START_DELAY + SCRIPT_TICK * steps.len() as u32 + Duration::from_millis(waits)
}

/// Plays back `--actions` (for tests without real input). The actions go straight to the shell,
/// regardless of the window focus.
fn run_script(ui: &AppWindow, shell: Rc<RefCell<Shell>>, steps: Vec<ScriptStep>) -> Timer {
    let ui = ui.as_weak();
    let mut next = 0;
    // Wait briefly until the window is up
    let mut due = Instant::now() + SCRIPT_START_DELAY;
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, SCRIPT_TICK, move || {
        let Some(ui) = ui.upgrade() else { return };
        if Instant::now() < due {
            return;
        }
        match steps.get(next) {
            Some(ScriptStep::Wait(ms)) => due = Instant::now() + Duration::from_millis(*ms),
            Some(ScriptStep::Action(action)) => shell.borrow_mut().apply(&ui, *action),
            // Development aid for testing toasts (also over running apps)
            Some(ScriptStep::Toast) => {
                shell.borrow_mut().toast(&ui, "◆", "Test notice", "From --actions (development)")
            }
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

/// Clock, network, battery and controller in the top bar, plus the matching button glyphs.
fn update_status(ui: &AppWindow, gamepads: Option<&RefCell<Gamepads>>, shell: &Shell, last_input: InputDevice) {
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
    update_glyphs(ui, controller.map(|c| c.style), shell, last_input);
}

/// Notes the device of an input and switches the hint glyphs when it changed.
fn input_used(
    ui: &AppWindow,
    gamepads: Option<&RefCell<Gamepads>>,
    shell: &Shell,
    last_input: &Cell<InputDevice>,
    device: InputDevice,
) {
    if last_input.replace(device) != device {
        let controller = gamepads.and_then(|g| g.borrow().status()).map(|c| c.style);
        update_glyphs(ui, controller, shell, device);
    }
}

fn update_glyphs(ui: &AppWindow, controller: Option<GlyphStyle>, shell: &Shell, last_input: InputDevice) {
    let style = match input::hint_glyphs(shell.glyph_choice(), controller, last_input) {
        Some(GlyphStyle::PlayStation) => ControllerStyle::Playstation,
        Some(GlyphStyle::Xbox) => ControllerStyle::Xbox,
        None => ControllerStyle::Keyboard,
    };
    if ui.get_controller_style() != style {
        ui.set_controller_style(style);
        shell.update_hints(ui);
    }
}

/// Scrolls once all the way right and back, then prints the measurements and exits.
fn start_bench(ui: &AppWindow, shell: Rc<RefCell<Shell>>, stats: Rc<RefCell<FrameStats>>) -> Timer {
    let len = shell.borrow().home_len();
    let steps = 2 * len.saturating_sub(1);
    let mut step = 0;
    let mut started: Option<Instant> = None;
    // Longest processing of an input on the UI thread (nothing expensive may happen there)
    let mut worst_step = Duration::ZERO;
    let ui = ui.as_weak();

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, BENCH_STEP, move || {
        let Some(ui) = ui.upgrade() else { return };
        let started = *started.get_or_insert_with(|| {
            stats.borrow_mut().reset_total();
            Instant::now()
        });
        if step >= steps {
            perf::print_bench_summary(len, steps, started.elapsed(), stats.borrow().total(), worst_step);
            let _ = slint::quit_event_loop();
            return;
        }
        let action = if step + 1 < len { Action::Right } else { Action::Left };
        let begin = Instant::now();
        shell.borrow_mut().apply(&ui, action);
        worst_step = worst_step.max(begin.elapsed());
        step += 1;
    });
    timer
}
