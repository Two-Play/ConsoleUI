//! Where the app runs and which settings of the user apply. The shell starts apps inside its own
//! compositor (or gamescope) and keeps the settings per profile; the app reads the settings of the
//! profile that was chosen last, so accent color and animations match the shell. A running shell
//! answers on the session bus (Story 8.7): that is the active profile, and it works from a sandbox.

use consoleui_core::profiles::{ProfilePaths, Profiles};
use consoleui_core::protocol::SOCKET_ENV;
use consoleui_core::service;
use consoleui_core::settings::{Accent, Settings};
use std::sync::mpsc::{self, Receiver, Sender};
use zbus::blocking::{Connection, Proxy, fdo};

/// Animation durations in milliseconds as in `theme.slint`: focus, background, pulse, launch, page.
pub const DURATIONS: [i64; 5] = [220, 500, 120, 450, 200];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A normal desktop window
    Desktop,
    /// Started by the shell inside our own compositor; every toplevel is fullscreen there
    Compositor,
    /// Started by the shell inside the gamescope session
    Gamescope,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub kind: Kind,
    pub settings: Settings,
}

impl Session {
    pub fn detect() -> Self {
        let kind = if std::env::var_os(SOCKET_ENV).is_some() {
            Kind::Compositor
        } else if std::env::var("CONSOLEUI_GAMESCOPE").is_ok_and(|v| v == "steam") {
            Kind::Gamescope
        } else {
            Kind::Desktop
        };
        let mut settings = load_settings();
        if let Err(err) = read_from_shell(&mut settings) {
            // The normal case on a desktop without a running shell
            if !matches!(err, zbus::Error::FDO(_) | zbus::Error::MethodError(..)) {
                eprintln!("Cannot ask the shell for the settings: {err}");
            }
        }
        Self { kind, settings }
    }

    /// The session again whenever the shell reports a changed setting. Nothing arrives without
    /// a running shell. An app built on `consoleui-sdk` uses `watch_session` instead.
    pub fn watch(&self) -> Receiver<Session> {
        let (sender, receiver) = mpsc::channel();
        let session = self.clone();
        std::thread::spawn(move || {
            if let Err(err) = forward_changes(session, sender) {
                eprintln!("Cannot follow the shell's settings: {err}");
            }
        });
        receiver
    }

    /// Inside a session the app fills the screen like every other window there.
    pub fn fullscreen(&self) -> bool {
        self.kind != Kind::Desktop
    }

    /// Durations for the theme; all zero when the user switched animations off.
    pub fn durations(&self) -> [i64; 5] {
        if self.settings.animations { DURATIONS } else { [0; 5] }
    }

    pub fn accent_rgb(&self) -> [u8; 3] {
        self.settings.accent.rgb()
    }
}

fn shell_proxy(connection: &Connection) -> zbus::Result<Proxy<'_>> {
    zbus::blocking::proxy::Builder::new(connection)
        .destination(service::NAME)?
        .path(service::PATH)?
        .interface(service::NAME)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

fn read_from_shell(settings: &mut Settings) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let names = fdo::DBusProxy::new(&connection)?;
    if !names.name_has_owner(service::NAME.try_into()?)? {
        return Ok(());
    }
    let shell = shell_proxy(&connection)?;
    let accent: String = shell.get_property("AccentColor")?;
    let rgb = service::color_from_hex(&accent);
    settings.accent = Accent::ALL.into_iter().find(|a| Some(a.rgb()) == rgb).unwrap_or(settings.accent);
    settings.animations = shell.get_property("Animations")?;
    settings.sounds = shell.get_property("Sounds")?;
    // Newer than the others: a shell without it keeps the profile's value
    if let Ok(glow) = shell.get_property("FocusGlow") {
        settings.focus_glow = glow;
    }
    Ok(())
}

fn forward_changes(mut session: Session, sender: Sender<Session>) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let builder = fdo::PropertiesProxy::builder(&connection).destination(service::NAME)?.path(service::PATH)?;
    let properties = builder.cache_properties(zbus::proxy::CacheProperties::No).build()?;
    for _ in properties.receive_properties_changed()? {
        if read_from_shell(&mut session.settings).is_ok() && sender.send(session.clone()).is_err() {
            break;
        }
    }
    Ok(())
}

/// Shows a toast through the shell, also over a game; nothing happens without a running shell.
pub fn toast(title: &str, text: &str) {
    let (title, text) = (title.to_string(), text.to_string());
    // Not on the UI thread: the bus may take a moment
    std::thread::spawn(move || {
        let sent = Connection::session().and_then(|connection| {
            shell_proxy(&connection)?.call_method("Toast", &(title, text))?;
            Ok(())
        });
        if let Err(err) = sent {
            eprintln!("No toast: {err}");
        }
    });
}

/// Tells the shell that apps were installed or removed, so its lists catch up.
pub fn library_changed() {
    std::thread::spawn(|| {
        let sent = Connection::session().and_then(|connection| {
            shell_proxy(&connection)?.call_method("RefreshLibrary", &())?;
            Ok(())
        });
        if let Err(err) = sent {
            eprintln!("The shell's library stays as it is: {err}");
        }
    });
}

/// Asks the shell to start an entry of its library (`desktop:<file>`, `steam:<id>`), so it shows up
/// like any other launch. False without a running shell. Blocks for a moment: not for the UI thread.
pub fn launch(entry_id: &str) -> bool {
    let sent = Connection::session().and_then(|connection| {
        shell_proxy(&connection)?.call_method("Launch", &(entry_id,))?;
        Ok(())
    });
    sent.is_ok()
}

fn load_settings() -> Settings {
    let Some(root) = Profiles::default_root() else { return Settings::default() };
    match Profiles::load(&root).last() {
        Some(profile) => Settings::load(&ProfilePaths::new(&root, &profile.id).settings),
        None => Settings::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_animations_means_zero_durations() {
        let mut session = Session { kind: Kind::Desktop, settings: Settings::default() };
        session.settings.animations = false;
        assert_eq!(session.durations(), [0; 5]);
        session.settings.animations = true;
        assert_eq!(session.durations(), DURATIONS);
        assert!(!session.fullscreen());
    }
}
