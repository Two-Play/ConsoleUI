//! `consoleui-compositor`: the ConsoleUI shell's own Wayland compositor (Epic 7, built on
//! Smithay). It runs nested (`--winit`) or on a TTY (`--drm`), starts the shell as a fullscreen
//! client and talks to it over the control socket in `CONSOLEUI_COMPOSITOR`.

use std::sync::atomic::{AtomicBool, Ordering};

use tracing::info;

mod backdrop;
mod backend;
mod blur;
mod child;
mod control;
mod drm;
mod focus;
mod input;
mod options;
mod policy;
mod render;
mod rounded;
mod state;
mod transition;
mod winit;
mod xwayland;

use options::{BackendChoice, Options, USAGE};
use state::State;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::Relaxed);
}

/// SIGTERM/SIGINT arrived: the main loops stop and clean up (sockets, XWayland, the shell).
pub fn shutdown_requested() -> bool {
    SHUTDOWN.load(Ordering::Relaxed)
}

fn install_signal_handlers() {
    // A plain flag instead of a signalfd: a signalfd needs the signals blocked, and that mask
    // would be inherited by the shell and every game it starts.
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // Safety: the handler only stores to an atomic.
        unsafe {
            libc::signal(signal, on_signal as *const () as libc::sighandler_t);
        }
    }
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,smithay=warn"));
    // The log goes to stderr: display managers drop a session's stdout (SDDM keeps stderr in
    // ~/.cache/wayland-errors, the session script sends both to the journal). Colors only on a
    // terminal, so log files stay readable.
    let stderr = std::io::stderr;
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&stderr()))
        .init();

    let nested = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    let mut options = match Options::parse(std::env::args().skip(1), nested) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    if options.help {
        println!("{USAGE}");
        return;
    }
    // Lets the session scripts turn the blur off without passing compositor options.
    if std::env::var_os("CONSOLEUI_NO_BLUR").is_some_and(|value| value != "0") {
        options.blur = false;
    }
    install_signal_handlers();
    let result = match options.backend {
        BackendChoice::Winit => winit::run(&options),
        BackendChoice::Drm => drm::run(&options),
    };
    match result {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("consoleui-compositor: {err}");
            std::process::exit(1);
        }
    }
}

impl State {
    /// After the backend is up: XWayland, then the shell (with `DISPLAY`), scheduled screenshots.
    pub fn start(&mut self, options: &Options) {
        self.blur = options.blur;
        if !self.blur {
            info!("backdrop blur off");
        }
        for (delay, path) in &options.screenshots {
            self.schedule_screenshot(*delay, path.clone());
        }
        info!("WAYLAND_DISPLAY={} CONSOLEUI_COMPOSITOR={}", self.socket_name, self.control.path().display());
        if options.xwayland {
            xwayland::start(self, options.shell.clone());
        } else {
            child::start_shell_or_stop(self, options.shell.as_deref(), None);
        }
    }

    /// Stops the shell and returns the exit code; the rest is cleaned up when `State` drops.
    pub fn shutdown(mut self) -> i32 {
        if let Some(shell) = self.shell.take() {
            info!("stopping the shell");
            shell.terminate();
        }
        self.exit_code
    }
}
