//! Verfolgt eine gestartete App, damit die Shell weiß, wann sie zurückkommen soll (Story 4.3).

use std::time::{Duration, Instant};

use crate::launch::ProcessHandle;

/// Mindestdauer des Startbildschirms, auch wenn der Prozess sofort endet
/// (z. B. weil die App schon lief und nur ein Fenster öffnet).
pub const MIN_HOLD: Duration = Duration::from_millis(2600);
/// So lange warten wir, bis Steam ein Spiel als laufend meldet, bevor wir aufgeben.
pub const STEAM_START_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug)]
enum Liveness {
    Process(ProcessHandle),
    /// `steam://` kehrt sofort zurück; ob das Spiel läuft, meldet Steam über `RunningAppID`.
    Steam { app_id: u32, seen: bool },
}

#[derive(Debug)]
pub struct RunningApp {
    liveness: Liveness,
    started: Instant,
}

/// Phase für die Anzeige im Startbildschirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Starting,
    Running,
    Finished,
}

impl RunningApp {
    pub fn process(handle: ProcessHandle, now: Instant) -> Self {
        Self { liveness: Liveness::Process(handle), started: now }
    }

    pub fn steam(app_id: u32, now: Instant) -> Self {
        Self { liveness: Liveness::Steam { app_id, seen: false }, started: now }
    }

    /// `steam_running` ist das Ergebnis von [`crate::steam::running_app_id`].
    pub fn phase(&mut self, now: Instant, steam_running: Option<u32>) -> Phase {
        let elapsed = now.saturating_duration_since(self.started);
        let alive = match &mut self.liveness {
            Liveness::Process(handle) => !handle.has_exited(),
            Liveness::Steam { app_id, seen } => {
                if steam_running == Some(*app_id) {
                    *seen = true;
                    true
                } else {
                    !*seen && elapsed < STEAM_START_TIMEOUT
                }
            }
        };
        match (alive, elapsed < MIN_HOLD) {
            (_, true) => Phase::Starting,
            (true, false) => match self.liveness {
                Liveness::Steam { seen: false, .. } => Phase::Starting,
                _ => Phase::Running,
            },
            (false, false) => Phase::Finished,
        }
    }

    pub fn is_steam(&self) -> bool {
        matches!(self.liveness, Liveness::Steam { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finished_process_still_holds_minimum_time() {
        let t0 = Instant::now();
        let mut app = RunningApp::process(ProcessHandle::finished(), t0);
        assert_eq!(app.phase(t0 + Duration::from_secs(1), None), Phase::Starting);
        assert_eq!(app.phase(t0 + MIN_HOLD, None), Phase::Finished);
    }

    #[test]
    fn steam_game_runs_until_steam_reports_it_stopped() {
        let t0 = Instant::now();
        let mut app = RunningApp::steam(730, t0);
        let t = |s| t0 + Duration::from_secs(s);
        assert_eq!(app.phase(t(5), None), Phase::Starting);
        assert_eq!(app.phase(t(10), Some(730)), Phase::Running);
        assert_eq!(app.phase(t(500), Some(730)), Phase::Running);
        assert_eq!(app.phase(t(501), None), Phase::Finished);
    }

    #[test]
    fn steam_game_that_never_starts_times_out() {
        let t0 = Instant::now();
        let mut app = RunningApp::steam(730, t0);
        assert_eq!(app.phase(t0 + STEAM_START_TIMEOUT - Duration::from_secs(1), Some(440)), Phase::Starting);
        assert_eq!(app.phase(t0 + STEAM_START_TIMEOUT, None), Phase::Finished);
    }
}
