//! Where the app runs and which settings of the user apply. The shell starts apps inside its own
//! compositor (or gamescope) and keeps the settings per profile; the app reads the settings of the
//! profile that was chosen last, so accent color and animations match the shell.

use consoleui_core::profiles::{ProfilePaths, Profiles};
use consoleui_core::protocol::SOCKET_ENV;
use consoleui_core::settings::Settings;

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
        Self { kind, settings: load_settings() }
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
