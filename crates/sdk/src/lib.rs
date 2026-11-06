//! The ConsoleUI SDK (Epic 8): what an app for the console builds on, and the only crate it should
//! depend on besides `slint`. The same app runs in a window on any Linux desktop and fullscreen
//! in the console session. Start from the template (`dev/new-app.sh`), the rules are in `docs/sdk.md`.
//!
//! ```ignore
//! slint::include_modules!();
//! consoleui_sdk::bridge!();
//!
//! fn main() -> Result<(), slint::PlatformError> {
//!     let ui = AppWindow::new()?;
//!     let session = Session::detect();
//!     apply_theme(ui.global::<Theme>(), &session);
//!     // ...
//!     ui.run()
//! }
//! ```
//!
//! `bridge!` generates `nav_action`, `hint_model`, `controller_style` and `apply_theme` against the
//! types Slint generated for the app. Everything outside this crate may change without notice.

use std::cell::{Cell, RefCell};
use std::error::Error;
use std::rc::Rc;
use std::time::Duration;

use consoleui_app::Input;
use consoleui_app::audio::Audio;
use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode};

pub use consoleui_app::gamepad::Connection;
pub use consoleui_app::session::{launch, library_changed, toast};
pub use consoleui_app::slint_bridge as bridge;
pub use consoleui_app::{Action, Event, GlyphStyle, HintButton, Hints, Session, Sound};

/// `build.rs` of an app.
#[cfg(feature = "build")]
pub mod build {
    /// Compiles the app's UI, with `@consoleui/...` imports resolved to the design system.
    pub fn compile(path: &str) {
        slint_build::compile_with_config(path, consoleui_ui::compiler_config()).expect("Slint build failed");
    }
}

/// Both input paths of an app as one stream of [`Event`]s: controllers are polled while the window
/// has the focus, keys arrive through [`Controls::key`]. Keep it alive as long as the app runs.
pub struct Controls {
    input: Input,
    handler: Rc<RefCell<dyn FnMut(Event)>>,
}

impl Controls {
    /// `active` is the window's focus (`InputScope.active`): the shell's quick menu may be open
    /// above the app, and then a controller press is not for the app.
    pub fn start(active: impl Fn() -> bool + 'static, handler: impl FnMut(Event) + 'static) -> Self {
        let handler: Rc<RefCell<dyn FnMut(Event)>> = Rc::new(RefCell::new(handler));
        let forward = handler.clone();
        let input = Input::start(active, move |event| (forward.borrow_mut())(event));
        Self { input, handler }
    }

    /// A key press from `InputScope`: `controls.key(nav_action(action))`. The hints switch to
    /// keyboard keys until a controller is used again.
    pub fn key(&self, action: Action) {
        self.input.keyboard_used();
        (self.handler.borrow_mut())(Event::Action(action));
    }
}

/// The UI sounds of the shell; silent if the user turned them off or there is no audio.
pub struct Sounds {
    audio: RefCell<Option<Audio>>,
    enabled: Cell<bool>,
}

impl Sounds {
    pub fn start(session: &Session) -> Self {
        let sounds = Self { audio: RefCell::new(None), enabled: Cell::new(false) };
        sounds.follow(session);
        sounds
    }

    /// Takes over the user's setting, for [`watch_session`].
    pub fn follow(&self, session: &Session) {
        let enabled = session.settings.sounds;
        self.enabled.set(enabled);
        let mut audio = self.audio.borrow_mut();
        if enabled && audio.is_none() {
            *audio = Audio::start();
        }
    }

    pub fn play(&self, sound: Sound) {
        if let Some(audio) = self.audio.borrow_mut().as_mut().filter(|_| self.enabled.get()) {
            audio.play(sound);
        }
    }
}

/// Calls `changed` when the user changes a setting in the running shell (accent color, animations,
/// sounds), so the app can follow at once. Keep the returned timer alive.
pub fn watch_session(session: &Session, mut changed: impl FnMut(&Session) + 'static) -> Timer {
    let sessions = session.watch();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
        if let Some(session) = sessions.try_iter().last() {
            changed(&session);
        }
    });
    timer
}

/// With `--snapshot PATH` on the command line: saves the window as PNG after a moment and quits,
/// for checking a UI without a screen capture; `--snapshot-after MS` changes the moment. Keep the
/// returned timer alive.
pub fn snapshot_from_args<C: ComponentHandle + 'static>(ui: &C) -> Timer {
    let timer = Timer::default();
    let mut args = std::env::args().skip_while(|arg| arg != "--snapshot").skip(1);
    let delay = std::env::args().skip_while(|arg| arg != "--snapshot-after").nth(1);
    let delay = delay.and_then(|ms| ms.parse().ok()).unwrap_or(1200);
    if let Some(path) = args.next() {
        let ui = ui.as_weak();
        timer.start(TimerMode::SingleShot, Duration::from_millis(delay), move || {
            if let Some(ui) = ui.upgrade() {
                let saved = ui.window().take_snapshot().map_err(Box::<dyn Error>::from);
                if let Err(err) = saved.and_then(|pixels| save_png(&path, &pixels)) {
                    eprintln!("Snapshot failed: {err}");
                }
            }
            let _ = slint::quit_event_loop();
        });
    }
    timer
}

fn save_png(path: &str, pixels: &SharedPixelBuffer<Rgba8Pixel>) -> Result<(), Box<dyn Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, pixels.width(), pixels.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    Ok(())
}
