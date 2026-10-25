use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::library::Launch;
use crate::ownership::Owner;

/// Wie ein gestarteter Prozess wieder beendet wird.
#[derive(Debug, Clone, PartialEq)]
enum Stopper {
    None,
    /// Transiente systemd-Scope-Unit: erwischt auch Kindprozesse, die sich vom Starter lösen.
    /// Die PID dient als Rückfall, solange systemd die Unit noch nicht registriert hat.
    Scope { unit: String, pid: u32 },
    /// Fallback ohne systemd: die eigene Prozessgruppe.
    ProcessGroup(u32),
}

/// Meldet, ob ein gestarteter Prozess beendet ist, und kann ihn beenden.
#[derive(Debug, Clone)]
pub struct ProcessHandle {
    exited: Arc<AtomicBool>,
    stopper: Stopper,
}

impl ProcessHandle {
    /// Für Starts ohne Prozess (Dummy-Einträge).
    pub fn finished() -> Self {
        Self { exited: Arc::new(AtomicBool::new(true)), stopper: Stopper::None }
    }

    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::Relaxed)
    }

    /// Woran sich die Prozesse dieser App erkennen lassen (für die Fensterzuordnung in gamescope).
    pub fn owner(&self) -> Option<Owner> {
        match &self.stopper {
            Stopper::None => None,
            Stopper::Scope { unit, .. } => Some(Owner::Scope(unit.clone())),
            Stopper::ProcessGroup(pid) => Some(Owner::Process(*pid)),
        }
    }

    /// Bittet die App per SIGTERM, sich zu beenden.
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
    if status.success() { Ok(()) } else { Err(io::Error::other(format!("Beenden fehlgeschlagen ({status})"))) }
}

/// Baut den Startbefehl. Mit `scope_unit` läuft die App über `systemd-run --scope` in einer eigenen
/// Unit; systemd-run führt das Programm dabei selbst aus, die PID bleibt also dieselbe.
fn plan(launch: &Launch, scope_unit: Option<&str>) -> Option<(Command, Option<String>)> {
    match launch {
        Launch::None => None,
        Launch::Steam(app_id) => {
            // `-silent`: Startet Steam erst durch diesen Aufruf, bleibt das Bibliotheksfenster zu
            // und nimmt dem Spiel in gamescope nicht den Fokus weg. Steam-Spiele sind Kinder von
            // Steam, nicht von uns, deshalb ohne eigenen Scope.
            let mut command = Command::new("steam");
            command.args(["-silent", &format!("steam://rungameid/{app_id}")]);
            Some((command, None))
        }
        Launch::Command(argv) => {
            let (program, args) = argv.split_first()?;
            match scope_unit {
                Some(unit) => {
                    let mut command = Command::new("systemd-run");
                    command.args(["--user", "--scope", "--collect", "--quiet", &format!("--unit={unit}"), "--"]);
                    command.args(argv);
                    Some((command, Some(format!("{unit}.scope"))))
                }
                None => {
                    let mut command = Command::new(program);
                    command.args(args);
                    Some((command, None))
                }
            }
        }
    }
}

/// Startet die App losgelöst von der Shell (eigene Prozessgruppe, damit Strg+C im Terminal sie
/// nicht mitbeendet). Ein Hintergrund-Thread wartet auf das Ende, damit kein Zombie bleibt.
pub fn spawn(launch: &Launch) -> io::Result<ProcessHandle> {
    if let Launch::Command(argv) = launch
        && let Some(program) = argv.first()
        && !program_exists(program)
    {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("{program} nicht gefunden")));
    }
    let unit = systemd_run_available().then(unique_unit_name);
    let Some((mut command, scope)) = plan(launch, unit.as_deref()) else { return Ok(ProcessHandle::finished()) };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
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

/// systemd-run braucht den Benutzer-Bus; ohne ihn (z. B. ohne systemd) starten wir direkt.
fn systemd_run_available() -> bool {
    program_exists("systemd-run")
        && std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|dir| Path::new(&dir).join("bus").exists())
}

pub fn program_exists(program: &str) -> bool {
    let path = Path::new(program);
    if path.is_absolute() {
        return path.is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
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
            ["systemd-run", "--user", "--scope", "--collect", "--quiet", "--unit=consoleui-app-1", "--", "vlc", "--fullscreen"]
        );
        assert_eq!(scope.as_deref(), Some("consoleui-app-1.scope"));
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
