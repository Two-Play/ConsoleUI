use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::library::Launch;

pub fn command(launch: &Launch) -> Option<Command> {
    match launch {
        Launch::None => None,
        Launch::Steam(app_id) => {
            let mut command = Command::new("steam");
            command.arg(format!("steam://rungameid/{app_id}"));
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

/// Startet die App losgelöst von der Shell (eigene Prozessgruppe, damit Strg+C im Terminal sie
/// nicht mitbeendet). Ein Hintergrund-Thread wartet auf das Ende, damit kein Zombie bleibt.
pub fn spawn(launch: &Launch) -> io::Result<()> {
    let Some(mut command) = command(launch) else { return Ok(()) };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(argv(&command(&Launch::Steam(730)).unwrap()), ["steam", "steam://rungameid/730"]);
        let custom = Launch::Command(vec!["vlc".into(), "--fullscreen".into()]);
        assert_eq!(argv(&command(&custom).unwrap()), ["vlc", "--fullscreen"]);
    }
}
