//! Verfolgt eine gestartete App, damit die Shell weiß, wann sie zurückkommen soll (Story 4.3).

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::launch::ProcessHandle;

/// Mindestdauer des Startbildschirms, auch wenn der Prozess sofort endet
/// (z. B. weil die App schon lief und nur ein Fenster öffnet).
pub const MIN_HOLD: Duration = Duration::from_millis(2600);
/// So lange warten wir, bis Steam ein Spiel als laufend meldet, bevor wir aufgeben.
pub const STEAM_START_TIMEOUT: Duration = Duration::from_secs(90);

/// Lücke ohne Spielprozess, die noch nicht als Ende zählt (z. B. Launcher beendet, Spiel startet).
pub const STEAM_EXIT_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug)]
enum Liveness {
    Process(ProcessHandle),
    /// `steam://` kehrt sofort zurück; ob das Spiel läuft, zeigen seine Prozesse
    /// (siehe [`crate::steam::running_games`]).
    Steam { app_id: u32, seen: bool, missing_since: Option<Instant> },
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
        Self { liveness: Liveness::Steam { app_id, seen: false, missing_since: None }, started: now }
    }

    /// `steam_games` sind die App-IDs mit laufenden Prozessen ([`crate::steam::running_games`]).
    pub fn phase(&mut self, now: Instant, steam_games: &HashSet<u32>) -> Phase {
        let elapsed = now.saturating_duration_since(self.started);
        let alive = match &mut self.liveness {
            Liveness::Process(handle) => !handle.has_exited(),
            Liveness::Steam { app_id, seen, missing_since } => {
                if steam_games.contains(app_id) {
                    *seen = true;
                    *missing_since = None;
                    true
                } else if *seen {
                    let since = *missing_since.get_or_insert(now);
                    now.saturating_duration_since(since) < STEAM_EXIT_GRACE
                } else {
                    elapsed < STEAM_START_TIMEOUT
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

    fn games(ids: &[u32]) -> HashSet<u32> {
        ids.iter().copied().collect()
    }

    #[test]
    fn finished_process_still_holds_minimum_time() {
        let t0 = Instant::now();
        let mut app = RunningApp::process(ProcessHandle::finished(), t0);
        assert_eq!(app.phase(t0 + Duration::from_secs(1), &games(&[])), Phase::Starting);
        assert_eq!(app.phase(t0 + MIN_HOLD, &games(&[])), Phase::Finished);
    }

    #[test]
    fn steam_game_ends_shortly_after_its_processes() {
        let t0 = Instant::now();
        let mut app = RunningApp::steam(730, t0);
        let t = |ms| t0 + Duration::from_millis(ms);
        assert_eq!(app.phase(t(5_000), &games(&[])), Phase::Starting);
        assert_eq!(app.phase(t(10_000), &games(&[730])), Phase::Running);
        assert_eq!(app.phase(t(500_000), &games(&[730, 440])), Phase::Running);
        // Kurze Lücke (Launcher → Spiel) ist noch kein Ende …
        assert_eq!(app.phase(t(500_500), &games(&[])), Phase::Running);
        assert_eq!(app.phase(t(501_000), &games(&[730])), Phase::Running);
        // … zwei Sekunden ohne Prozess schon.
        assert_eq!(app.phase(t(502_000), &games(&[])), Phase::Running);
        assert_eq!(app.phase(t(502_000) + STEAM_EXIT_GRACE, &games(&[])), Phase::Finished);
    }

    #[test]
    fn steam_game_that_never_starts_times_out() {
        let t0 = Instant::now();
        let mut app = RunningApp::steam(730, t0);
        assert_eq!(app.phase(t0 + STEAM_START_TIMEOUT - Duration::from_secs(1), &games(&[440])), Phase::Starting);
        assert_eq!(app.phase(t0 + STEAM_START_TIMEOUT, &games(&[])), Phase::Finished);
    }
}
