//! The player window that opens from the media card of the quick menu, like on a PS5, and the
//! "now playing" note when a player starts a new track. The players are read in `crate::media`,
//! the window's logic is `consoleui_core::media::PlayerView`.

use consoleui_core::media::{PlayerOutcome, PlayerView, clock};

use super::*;
use crate::PlayerState;

pub(super) struct OpenPlayer {
    view: PlayerView,
    /// Over an app: in the overlay window
    in_overlay: bool,
}

impl Shell {
    pub(super) fn open_player(&mut self, ui: &AppWindow) {
        let in_overlay = self.quick.is_some() && self.overlay_window().is_some();
        self.player = Some(OpenPlayer { view: PlayerView::new(), in_overlay });
        self.show_player(ui);
    }

    pub(super) fn apply_player(&mut self, ui: &AppWindow, action: Action) {
        let Some(open) = self.player.as_mut() else { return };
        match open.view.apply(action, self.media_player.as_ref()) {
            PlayerOutcome::Nothing => return,
            PlayerOutcome::Moved => {}
            PlayerOutcome::Command(command) => {
                if let Some(media) = &self.media {
                    media.send(command);
                }
            }
            PlayerOutcome::Close => return self.close_player(ui),
        }
        self.show_player(ui);
    }

    /// A button of the window was clicked.
    pub(super) fn click_player(&mut self, ui: &AppWindow, index: usize) {
        if let Some(open) = self.player.as_mut() {
            open.view.focused = index;
        }
        self.apply_player(ui, Action::Confirm);
    }

    pub(super) fn close_player(&mut self, ui: &AppWindow) {
        let Some(open) = self.player.take() else { return };
        match open.in_overlay.then(|| self.overlay_window()).flatten() {
            Some(overlay) => overlay.set_player_open(false),
            None => ui.set_player_open(false),
        }
        self.update_hints(ui);
    }

    /// Puts the playing track into the window (called again whenever the player reports).
    pub(super) fn show_player(&self, ui: &AppWindow) {
        let Some(open) = &self.player else { return };
        let player = self.media_player.clone().unwrap_or_default();
        let cover = player.art.as_deref().and_then(|path| slint::Image::load_from_path(path).ok());
        let progress = if player.length > 0 { player.position as f32 / player.length as f32 } else { 0.0 };
        let state = PlayerState {
            has_cover: cover.is_some(),
            cover: cover.unwrap_or_default(),
            source: player.identity.to_uppercase().into(),
            title: if player.title.is_empty() { tr("Nothing plays").into() } else { player.title.as_str().into() },
            artist: player.artist.as_str().into(),
            album: player.album.as_str().into(),
            playing: player.playing,
            progress,
            time: if player.length > 0 { clock(player.position).into() } else { "".into() },
            length: if player.length > 0 { clock(player.length).into() } else { "".into() },
            focused: open.view.focused as i32,
            can_previous: player.can_previous,
            can_next: player.can_next,
            has_shuffle: player.shuffle.is_some(),
            shuffle: player.shuffle.unwrap_or(false),
        };
        let hint = |button, label: &str| Hint { button, label: label.into() };
        let mut hints = vec![hint(GlyphButton::Dpad, tr("Select")), hint(GlyphButton::Confirm, tr("OK"))];
        if player.can_seek {
            hints.push(hint(GlyphButton::L1, tr("Seek")));
        }
        hints.push(hint(GlyphButton::Back, tr("Back")));
        let hints = ModelRc::new(VecModel::from(hints));
        match open.in_overlay.then(|| self.overlay_window()).flatten() {
            Some(overlay) => {
                overlay.set_player(state);
                overlay.set_player_hints(hints);
                overlay.set_player_open(self.dialog.is_none());
            }
            None => {
                ui.set_player(state);
                ui.set_player_hints(hints);
                ui.set_player_open(self.dialog.is_none());
            }
        }
    }

    /// A player started a new track: a note with its cover, unless the player window or the
    /// quick menu shows it anyway.
    pub(super) fn announce_track(&mut self, ui: &AppWindow, player: &Player) {
        if !self.settings.notifications || self.quick.is_some() || self.player.is_some() {
            return;
        }
        self.next_toast_picture = player.art.as_deref().and_then(|path| slint::Image::load_from_path(path).ok());
        let detail = match (player.artist.is_empty(), player.identity.is_empty()) {
            (false, false) => format!("{} · {}", player.artist, player.identity),
            (false, true) => player.artist.clone(),
            _ => player.identity.clone(),
        };
        self.toast(ui, "♫", &player.title, &detail);
    }
}
