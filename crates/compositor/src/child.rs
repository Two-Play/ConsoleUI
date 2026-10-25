//! Starting the shell inside the compositor. When the shell exits, the compositor exits with its
//! exit code (like gamescope does with its main program).

use std::io;
use std::process::Command;

use consoleui_core::{ownership, protocol};
use smithay::reexports::calloop::channel::{self, Event};
use tracing::{info, warn};

use crate::state::State;

pub struct ShellProcess {
    pub pid: u32,
}

impl ShellProcess {
    /// Asks the shell to quit (compositor shutdown).
    pub fn terminate(&self) {
        if let Ok(pid) = i32::try_from(self.pid) {
            // Safety: plain kill(2) on our own child.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}

/// Starts `command` with this compositor's Wayland socket, the XWayland display (if any) and the
/// control socket in its environment.
pub fn spawn_shell(state: &mut State, command: &[String], x_display: Option<u32>) -> io::Result<()> {
    let (program, args) = command.split_first().ok_or_else(|| io::Error::other("empty shell command"))?;
    let mut process = Command::new(program);
    process
        .args(args)
        .env("WAYLAND_DISPLAY", &state.socket_name)
        .env(protocol::SOCKET_ENV, state.control.path())
        .env("XDG_SESSION_TYPE", "wayland")
        .env_remove("WAYLAND_SOCKET")
        // The shell must not think it runs in the gamescope session.
        .env_remove("CONSOLEUI_GAMESCOPE")
        .env_remove("GAMESCOPE_WAYLAND_DISPLAY")
        .env_remove("ENABLE_GAMESCOPE_WSI");
    match x_display {
        Some(display) => process.env("DISPLAY", format!(":{display}")),
        // Without XWayland, X11 apps must not open on the desktop around a nested compositor.
        None => process.env_remove("DISPLAY"),
    };
    let mut child = process.spawn()?;
    let pid = child.id();
    info!("started the shell: {} (pid {pid})", command.join(" "));

    let (sender, receiver) = channel::channel::<Option<i32>>();
    std::thread::spawn(move || {
        let code = child.wait().ok().and_then(|status| status.code());
        let _ = sender.send(code);
    });
    state
        .loop_handle
        .insert_source(receiver, |event, _, state| {
            if let Event::Msg(code) = event {
                info!("the shell exited ({code:?}), stopping");
                state.shell = None;
                state.exit_code = code.unwrap_or(1);
                state.running = false;
                state.loop_signal.stop();
            }
        })
        .map_err(|err| io::Error::other(err.to_string()))?;
    state.shell = Some(ShellProcess { pid });
    // Until the shell says `Hello` (it may run behind a wrapper), its windows are the child's.
    let events = state.policy.set_shell_pid(pid, &ownership::read_process);
    state.after_policy_change(events);
    Ok(())
}

/// Starts the shell or reports why not; without a shell there is nothing to show.
pub fn start_shell_or_stop(state: &mut State, command: Option<&[String]>, x_display: Option<u32>) {
    let Some(command) = command else {
        info!("no shell: connect clients to WAYLAND_DISPLAY={}", state.socket_name);
        return;
    };
    if let Err(err) = spawn_shell(state, command, x_display) {
        warn!("cannot start the shell {:?}: {err}", command.join(" "));
        state.exit_code = 1;
        state.running = false;
        state.loop_signal.stop();
    }
}
