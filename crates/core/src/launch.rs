use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::library::Launch;

pub fn command(launch: &Launch) -> Option<Command> {
    match launch {
        Launch::None => None,
        Launch::Steam(app_id) => {
            // `-silent`: Startet Steam erst durch diesen Aufruf, bleibt das Bibliotheksfenster zu
            // und nimmt dem Spiel in gamescope nicht den Fokus weg.
            let mut command = Command::new("steam");
            command.args(["-silent", &format!("steam://rungameid/{app_id}")]);
            Some(command)
        }
        Launch::Command(argv) => {
            let (program, args) = argv.split_first()?;
            let mut command = Command::new(program);
            command.args(args);
            Some(command)
        }
    }
}

/// Meldet, ob ein gestarteter Prozess beendet ist.
#[derive(Debug, Clone)]
pub struct ProcessHandle(Arc<AtomicBool>);

impl ProcessHandle {
    /// Für Starts ohne Prozess (Dummy-Einträge).
    pub fn finished() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }

    pub fn has_exited(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Startet die App losgelöst von der Shell (eigene Prozessgruppe, damit Strg+C im Terminal sie
/// nicht mitbeendet). Ein Hintergrund-Thread wartet auf das Ende, damit kein Zombie bleibt.
pub fn spawn(launch: &Launch) -> io::Result<ProcessHandle> {
    let Some(mut command) = command(launch) else { return Ok(ProcessHandle::finished()) };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let exited = Arc::new(AtomicBool::new(false));
    let flag = exited.clone();
    std::thread::spawn(move || {
        let _ = child.wait();
        flag.store(true, Ordering::Relaxed);
    });
    Ok(ProcessHandle(exited))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn handle_reports_process_exit() {
        let handle = spawn(&Launch::Command(vec!["true".into()])).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !handle.has_exited() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(handle.has_exited());
        assert!(spawn(&Launch::None).unwrap().has_exited());
    }

    fn argv(command: &Command) -> Vec<String> {
        std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn builds_commands() {
        assert!(command(&Launch::None).is_none());
        assert!(command(&Launch::Command(vec![])).is_none());
        assert_eq!(argv(&command(&Launch::Steam(730)).unwrap()), ["steam", "-silent", "steam://rungameid/730"]);
        let custom = Launch::Command(vec!["vlc".into(), "--fullscreen".into()]);
        assert_eq!(argv(&command(&custom).unwrap()), ["vlc", "--fullscreen"]);
    }
}
