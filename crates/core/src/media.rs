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
