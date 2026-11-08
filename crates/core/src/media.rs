//! What plays right now, for the quick menu's media card. The shell reads the players over MPRIS;
//! this is the part that needs no bus.

/// A media player on the session bus (browser, music app, ...) that has something loaded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Player {
    /// Its name on the bus
    pub bus_name: String,
    /// The app's name, e.g. "Firefox"
    pub identity: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub can_next: bool,
    pub can_previous: bool,
    pub album: String,
    /// `mpris:trackid`, or the title when the player has none: a new one is a new track
    pub track: String,
    /// `mpris:artUrl`, and where the shell keeps a copy of it
    pub art_url: String,
    pub art: Option<std::path::PathBuf>,
    /// Microseconds; 0 when the player does not say
    pub length: u64,
    pub position: u64,
    pub can_seek: bool,
    /// `None` when the player has no shuffle
    pub shuffle: Option<bool>,
}

impl Player {
    /// The big line of the card: the track, or the app if it does not say.
    pub fn headline(&self) -> &str {
        if self.title.is_empty() { &self.identity } else { &self.title }
    }

    /// The small line of the card, never the same as the big one.
    pub fn detail(&self) -> &str {
        if !self.artist.is_empty() {
            &self.artist
        } else if self.title.is_empty() {
            ""
        } else {
            &self.identity
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCommand {
    PlayPause,
    Next,
    Previous,
    /// Microseconds forward (or back when negative)
    Seek(i64),
    Shuffle(bool),
}

/// "3:18", from microseconds
pub fn clock(micros: u64) -> String {
    let seconds = micros / 1_000_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// How far a press of L1/R1 seeks in the player
const SEEK_STEP: i64 = 10_000_000;

/// The buttons of the player window, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerButton {
    Previous,
    PlayPause,
    Next,
    Shuffle,
}

/// What an input on the player window leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerOutcome {
    Nothing,
    Moved,
    Command(MediaCommand),
    Close,
}

/// The player window that opens from the media card, like on a PS5: big cover, the track and
/// buttons. The focus starts on play/pause.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerView {
    pub focused: usize,
}

impl PlayerView {
    pub fn new() -> Self {
        Self { focused: 1 }
    }

    /// The buttons this player offers.
    pub fn buttons(player: &Player) -> Vec<PlayerButton> {
        let mut buttons = vec![PlayerButton::Previous, PlayerButton::PlayPause, PlayerButton::Next];
        if player.shuffle.is_some() {
            buttons.push(PlayerButton::Shuffle);
        }
        buttons
    }

    pub fn apply(&mut self, action: crate::Action, player: Option<&Player>) -> PlayerOutcome {
        use crate::Action;
        let Some(player) = player else {
            return if matches!(action, Action::Back | Action::Guide) {
                PlayerOutcome::Close
            } else {
                PlayerOutcome::Nothing
            };
        };
        let buttons = Self::buttons(player);
        self.focused = self.focused.min(buttons.len() - 1);
        match action {
            Action::Left if self.focused > 0 => {
                self.focused -= 1;
                PlayerOutcome::Moved
            }
            Action::Right if self.focused + 1 < buttons.len() => {
                self.focused += 1;
                PlayerOutcome::Moved
            }
            Action::Confirm => match buttons[self.focused] {
                PlayerButton::Previous if player.can_previous => PlayerOutcome::Command(MediaCommand::Previous),
                PlayerButton::Next if player.can_next => PlayerOutcome::Command(MediaCommand::Next),
                PlayerButton::PlayPause => PlayerOutcome::Command(MediaCommand::PlayPause),
                PlayerButton::Shuffle => {
                    PlayerOutcome::Command(MediaCommand::Shuffle(!player.shuffle.unwrap_or(false)))
                }
                _ => PlayerOutcome::Nothing,
            },
            Action::PrevTab if player.can_seek => PlayerOutcome::Command(MediaCommand::Seek(-SEEK_STEP)),
            Action::NextTab if player.can_seek => PlayerOutcome::Command(MediaCommand::Seek(SEEK_STEP)),
            Action::Back | Action::Guide => PlayerOutcome::Close,
            _ => PlayerOutcome::Nothing,
        }
    }
}

/// Whether `now` is worth a "now playing" note: it plays, it is a track the user did not hear
/// announced last, and it is not the first look (the shell just started).
pub fn announce(last: Option<&Player>, now: &Player) -> bool {
    now.playing
        && !now.title.is_empty()
        && last.is_some_and(|last| last.bus_name != now.bus_name || last.track != now.track)
}

/// The player the card controls: one that is playing, else the one it showed before, else the first.
pub fn pick(players: Vec<Player>, shown: Option<&str>) -> Option<Player> {
    let shown_first = |player: &Player| (!player.playing, Some(player.bus_name.as_str()) != shown);
    players.into_iter().min_by_key(|player| shown_first(player))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(name: &str, playing: bool) -> Player {
        Player { bus_name: name.into(), playing, ..Player::default() }
    }

    fn spotify() -> Player {
        Player {
            bus_name: "org.mpris.MediaPlayer2.spotify".into(),
            identity: "Spotify".into(),
            title: "Feet".into(),
            track: "/com/spotify/track/1".into(),
            playing: true,
            can_next: true,
            can_previous: true,
            can_seek: true,
            shuffle: Some(false),
            length: 198_800_000,
            ..Player::default()
        }
    }

    #[test]
    fn the_player_window_presses_its_buttons() {
        use crate::Action;
        let player = spotify();
        let mut view = PlayerView::new();
        assert_eq!(view.apply(Action::Confirm, Some(&player)), PlayerOutcome::Command(MediaCommand::PlayPause));
        assert_eq!(view.apply(Action::Left, Some(&player)), PlayerOutcome::Moved);
        assert_eq!(view.apply(Action::Left, Some(&player)), PlayerOutcome::Nothing);
        assert_eq!(view.apply(Action::Confirm, Some(&player)), PlayerOutcome::Command(MediaCommand::Previous));
        for _ in 0..3 {
            view.apply(Action::Right, Some(&player));
        }
        assert_eq!(view.apply(Action::Confirm, Some(&player)), PlayerOutcome::Command(MediaCommand::Shuffle(true)));
        assert_eq!(view.apply(Action::NextTab, Some(&player)), PlayerOutcome::Command(MediaCommand::Seek(SEEK_STEP)));
        let no_shuffle = Player { shuffle: None, can_seek: false, ..player };
        assert_eq!(PlayerView::buttons(&no_shuffle).len(), 3);
        assert_eq!(view.apply(Action::PrevTab, Some(&no_shuffle)), PlayerOutcome::Nothing);
        assert_eq!(view.apply(Action::Back, Some(&no_shuffle)), PlayerOutcome::Close);
        assert_eq!(view.apply(Action::Back, None), PlayerOutcome::Close);
    }

    #[test]
    fn only_a_new_playing_track_is_announced() {
        let first = spotify();
        assert!(!announce(None, &first), "not at the start");
        assert!(!announce(Some(&first), &first), "the same track");
        let next = Player { track: "/com/spotify/track/2".into(), title: "Skin".into(), ..first.clone() };
        assert!(announce(Some(&first), &next));
        let paused = Player { playing: false, ..next.clone() };
        assert!(!announce(Some(&first), &paused));
        assert_eq!(clock(198_800_000), "3:18");
    }

    #[test]
    fn a_playing_player_wins() {
        let picked = pick(vec![player("a", false), player("b", true)], Some("a"));
        assert_eq!(picked.map(|p| p.bus_name), Some("b".to_string()));
    }

    #[test]
    fn stays_with_the_shown_player_while_nothing_plays() {
        let picked = pick(vec![player("a", false), player("b", false)], Some("b"));
        assert_eq!(picked.map(|p| p.bus_name), Some("b".to_string()));
        let both = pick(vec![player("a", true), player("b", true)], Some("b"));
        assert_eq!(both.map(|p| p.bus_name), Some("b".to_string()));
        assert_eq!(pick(Vec::new(), None), None);
    }

    #[test]
    fn the_card_never_says_the_same_twice() {
        let mut player = Player { identity: "Firefox".into(), ..Player::default() };
        assert_eq!((player.headline(), player.detail()), ("Firefox", ""));
        player.title = "Song".into();
        assert_eq!((player.headline(), player.detail()), ("Song", "Firefox"));
        player.artist = "Band".into();
        assert_eq!((player.headline(), player.detail()), ("Song", "Band"));
    }
}
