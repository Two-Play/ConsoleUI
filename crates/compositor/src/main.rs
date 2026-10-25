//! `consoleui-compositor`: the ConsoleUI shell's own Wayland compositor (Epic 7, built on
//! Smithay). It runs nested (`--winit`) or on a TTY (`--drm`), starts the shell as a fullscreen
//! client and talks to it over the control socket in `CONSOLEUI_COMPOSITOR`.

use std::sync::atomic::{AtomicBool, Ordering};

use tracing::info;

mod backend;
mod child;
mod control;
mod drm;
mod focus;
mod input;
mod options;
mod policy;
mod render;
mod state;
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
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let nested = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    let options = match Options::parse(std::env::args().skip(1), nested) {
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
    /// After the backend is up: XWayland (7.3), the shell, scheduled screenshots.
    pub fn start(&mut self, options: &Options) {
        for (delay, path) in &options.screenshots {
            self.schedule_screenshot(*delay, path.clone());
        }
        info!("WAYLAND_DISPLAY={} CONSOLEUI_COMPOSITOR={}", self.socket_name, self.control.path().display());
        child::start_shell_or_stop(self, options.shell.as_deref(), None);
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
