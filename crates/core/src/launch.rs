use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::library::Launch;
use crate::ownership::Owner;

/// How a launched process is stopped again.
#[derive(Debug, Clone, PartialEq)]
enum Stopper {
    None,
    /// Transient systemd scope unit: also catches child processes that detach from the launcher.
    /// The PID serves as fallback while systemd has not registered the unit yet.
    Scope {
        unit: String,
        pid: u32,
    },
    /// Fallback without systemd: the own process group.
    ProcessGroup(u32),
}

/// Reports whether a launched process has exited, and can stop it.
#[derive(Debug, Clone)]
pub struct ProcessHandle {
    exited: Arc<AtomicBool>,
    stopper: Stopper,
}

impl ProcessHandle {
    /// For launches without a process (dummy entries).
    pub fn finished() -> Self {
        Self { exited: Arc::new(AtomicBool::new(true)), stopper: Stopper::None }
    }

    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::Relaxed)
    }

    /// How the processes of this app are recognized (for window assignment in gamescope).
    pub fn owner(&self) -> Option<Owner> {
        match &self.stopper {
            Stopper::None => None,
            Stopper::Scope { unit, .. } => Some(Owner::Scope(unit.clone())),
            Stopper::ProcessGroup(pid) => Some(Owner::Process(*pid)),
        }
    }

    /// PID of the launched process (for the compositor's window mapping).
    pub fn pid(&self) -> Option<u32> {
        match &self.stopper {
            Stopper::None => None,
            Stopper::Scope { pid, .. } => Some(*pid),
            Stopper::ProcessGroup(pid) => Some(*pid),
        }
    }

    /// Transient systemd scope unit the app runs in, if any.
    pub fn scope(&self) -> Option<&str> {
        match &self.stopper {
            Stopper::Scope { unit, .. } => Some(unit),
            _ => None,
        }
    }

    /// Asks the app via SIGTERM to quit.
    pub fn stop(&self) -> io::Result<()> {
        if self.has_exited() {
            return Ok(());
        }
        match &self.stopper {
            Stopper::None => Ok(()),
            Stopper::Scope { unit, pid } => {
                let stopped = Command::new("systemctl").args(["--user", "stop", unit]).status()?.success();
                if stopped { Ok(()) } else { kill_group(*pid) }
            }
            Stopper::ProcessGroup(pgid) => kill_group(*pgid),
        }
    }
}

fn kill_group(pgid: u32) -> io::Result<()> {
    let status = Command::new("kill").args(["-TERM", "--", &format!("-{pgid}")]).status()?;
    if status.success() { Ok(()) } else { Err(io::Error::other(format!("Quitting failed ({status})"))) }
}

/// Builds the launch command. With `scope_unit` the app runs in its own unit via
/// `systemd-run --scope`; systemd-run executes the program itself, so the PID stays the same.
fn plan(launch: &Launch, scope_unit: Option<&str>) -> Option<(Command, Option<String>)> {
    match launch {
        Launch::None => None,
        Launch::Steam(app_id) => {
            // `-silent`: if this call starts Steam, the library window stays closed and does not
            // take the focus from the game in gamescope. Steam games are children of Steam, not
            // of us, hence no own scope.
            let mut command = Command::new("steam");
            command.args(["-silent", &format!("steam://rungameid/{app_id}")]);
            Some((command, None))
        }
        Launch::Command(argv) => {
            let (program, args) = argv.split_first()?;
            let command = match scope_unit {
                Some(unit) => {
                    let mut command = Command::new("systemd-run");
                    command.args(["--user", "--scope", "--collect", "--quiet", &format!("--unit={unit}"), "--"]);
                    command.args(argv);
                    (command, Some(format!("{unit}.scope")))
                }
                None => {
                    let mut command = Command::new(program);
                    command.args(args);
                    (command, None)
                }
            };
            Some(without_outer_wsi(command, program))
        }
    }
}

/// A nested gamescope (Story 7.10) inside the gamescope session would load the outer gamescope's
/// WSI layer for its own swapchain (`ENABLE_GAMESCOPE_WSI=1` is inherited), which fails with
/// "Creating swapchain for non-Gamescope swapchain". Unset, the nested gamescope sets it for its
/// own children again.
fn without_outer_wsi((mut command, scope): (Command, Option<String>), program: &str) -> (Command, Option<String>) {
    if Path::new(program).file_name().is_some_and(|name| name == crate::game_options::GAMESCOPE) {
        command.env_remove("ENABLE_GAMESCOPE_WSI");
    }
    (command, scope)
}

/// Starts the app detached from the shell (own process group, so Ctrl+C in the terminal does
/// not stop it too). A background thread waits for the exit so no zombie remains.
pub fn spawn(launch: &Launch) -> io::Result<ProcessHandle> {
    if let Launch::Command(argv) = launch
        && let Some(program) = argv.first()
        && !program_exists(program)
    {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("{program} not found")));
    }
    let unit = systemd_run_available().then(unique_unit_name);
    let Some((mut command, scope)) = plan(launch, unit.as_deref()) else { return Ok(ProcessHandle::finished()) };
    let mut child =
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0).spawn()?;
    let stopper = match scope {
        Some(unit) => Stopper::Scope { unit, pid: child.id() },
        None if matches!(launch, Launch::Command(_)) => Stopper::ProcessGroup(child.id()),
        None => Stopper::None,
    };
    let exited = Arc::new(AtomicBool::new(false));
    let flag = exited.clone();
    std::thread::spawn(move || {
        let _ = child.wait();
        flag.store(true, Ordering::Relaxed);
    });
    Ok(ProcessHandle { exited, stopper })
}

fn unique_unit_name() -> String {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default();
    format!("consoleui-app-{millis}")
}

/// systemd-run needs the user bus; without it (e.g. without systemd) we start directly.
fn systemd_run_available() -> bool {
    program_exists("systemd-run")
        && std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|dir| Path::new(&dir).join("bus").exists())
}

pub fn program_exists(program: &str) -> bool {
    let path = Path::new(program);
    if path.is_absolute() {
        return path.is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn argv(command: &Command) -> Vec<String> {
        std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    fn wait_for_exit(handle: &ProcessHandle) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !handle.has_exited() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn plans_commands() {
        assert!(plan(&Launch::None, None).is_none());
        assert!(plan(&Launch::Command(vec![]), None).is_none());
        let (steam, scope) = plan(&Launch::Steam(730), Some("x")).unwrap();
        assert_eq!(argv(&steam), ["steam", "-silent", "steam://rungameid/730"]);
        assert_eq!(scope, None);

        let vlc = Launch::Command(vec!["vlc".into(), "--fullscreen".into()]);
        let (direct, _) = plan(&vlc, None).unwrap();
        assert_eq!(argv(&direct), ["vlc", "--fullscreen"]);
        let (scoped, scope) = plan(&vlc, Some("consoleui-app-1")).unwrap();
        assert_eq!(
            argv(&scoped),
            [
                "systemd-run",
                "--user",
                "--scope",
                "--collect",
                "--quiet",
                "--unit=consoleui-app-1",
                "--",
                "vlc",
                "--fullscreen"
            ]
        );
        assert_eq!(scope.as_deref(), Some("consoleui-app-1.scope"));
        assert!(scoped.get_envs().next().is_none());
    }

    #[test]
    fn nested_gamescope_does_not_inherit_the_outer_wsi_layer() {
        let wrapped = Launch::Command(["gamescope", "-f", "--", "glxgears"].map(String::from).to_vec());
        let (command, scope) = plan(&wrapped, Some("consoleui-app-2")).unwrap();
        assert_eq!(argv(&command)[6..], ["--", "gamescope", "-f", "--", "glxgears"]);
        assert_eq!(scope.as_deref(), Some("consoleui-app-2.scope"));
        let envs: Vec<_> = command.get_envs().collect();
        assert_eq!(envs, [(std::ffi::OsStr::new("ENABLE_GAMESCOPE_WSI"), None)]);
    }

    #[test]
    fn handle_reports_process_exit() {
        let handle = spawn(&Launch::Command(vec!["true".into()])).unwrap();
        wait_for_exit(&handle);
        assert!(handle.has_exited());
        assert!(spawn(&Launch::None).unwrap().has_exited());
    }

    #[test]
    fn stop_terminates_running_process() {
        let handle = spawn(&Launch::Command(vec!["sleep".into(), "30".into()])).unwrap();
        assert!(!handle.has_exited());
        handle.stop().unwrap();
        wait_for_exit(&handle);
        assert!(handle.has_exited());
    }

    #[test]
    fn missing_program_is_an_error() {
        assert!(spawn(&Launch::Command(vec!["gibt-es-nicht-consoleui".into()])).is_err());
    }
}
