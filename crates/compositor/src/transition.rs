//! Animated changes of the window in front (Story 7.7), without Smithay types.
//!
//! - **Open** (shell → app): the app's windows zoom out of the tile the shell named, or out of
//!   its launch screen where that screen is at that moment, to fullscreen, and fade in. The image
//!   keeps its aspect ratio: it is scaled to cover the growing rectangle and cropped to it.
//! - **Close** (app → shell): the app's windows shrink into the tile and fade out above the
//!   shell, which settles from a slight zoom and brightens. If the app has no window any more (it
//!   quit), only the shell part runs.
//! - The zooming app image has rounded corners like the tiles and the launch screen: those of the
//!   tile at the small end, none at fullscreen (drawn by `rounded.rs`).
//!
//! The shell asks for a transition with [`ShellMessage::Focus`](consoleui_core::protocol); a
//! front change without a request is instant, except the shell coming back after an app closed
//! its window while the shell uses transitions. `State::frame_windows` asks [`Transitions`] how to
//! draw each window; the renderer turns an [`Effect`] into rescale/relocate/crop elements.

use std::time::Duration;

use consoleui_core::protocol::{Rect, Target, Transition};

use crate::policy::Role;

/// Duration of the zoom to the front
pub const OPEN: Duration = Duration::from_millis(520);
/// Duration of the return to the shell
pub const CLOSE: Duration = Duration::from_millis(440);
/// Corner radius relative to the shorter side, as on the shell's tiles (`tile.slint`)
const TILE_ROUNDING: f64 = 0.12;
/// Largest corner radius relative to the output height (for big clip rectangles)
const MAX_ROUNDING: f64 = 0.04;

/// A rectangle in output coordinates (logical pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Area {
    pub fn full(output: (f64, f64)) -> Self {
        Self { x: 0.0, y: 0.0, w: output.0, h: output.1 }
    }

    fn from_rect(rect: Rect) -> Self {
        Self { x: rect.x as f64, y: rect.y as f64, w: rect.width.max(1) as f64, h: rect.height.max(1) as f64 }
    }

    /// The output shrunk by `fraction` on every side (keeps the aspect ratio).
    fn inset(output: (f64, f64), fraction: f64) -> Self {
        let (w, h) = output;
        Self { x: w * fraction, y: h * fraction, w: w * (1.0 - 2.0 * fraction), h: h * (1.0 - 2.0 * fraction) }
    }

    fn lerp(self, other: Self, t: f64) -> Self {
        Self {
            x: lerp(self.x, other.x, t),
            y: lerp(self.y, other.y, t),
            w: lerp(self.w, other.w, t),
            h: lerp(self.h, other.h, t),
        }
    }
}

/// How the windows of one group are drawn in a frame: scaled by `scale` around the output origin,
/// then moved by `offset`, cut to `clip` (if any) with corners of `radius` and drawn with `alpha`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effect {
    pub scale: f64,
    pub offset: (f64, f64),
    pub clip: Option<Area>,
    pub radius: f64,
    pub alpha: f32,
}

impl Effect {
    /// The output-sized image scaled uniformly so it covers `clip`, centered on it, cut to it.
    /// `rounding` 0…1: how far the corners are rounded (1: like a tile of the clip's size).
    fn cover(clip: Area, output: (f64, f64), alpha: f64, rounding: f64) -> Self {
        let scale = (clip.w / output.0).max(clip.h / output.1);
        let offset = (clip.x + (clip.w - output.0 * scale) / 2.0, clip.y + (clip.h - output.1 * scale) / 2.0);
        let radius = (TILE_ROUNDING * clip.w.min(clip.h)).min(MAX_ROUNDING * output.1) * rounding.clamp(0.0, 1.0);
        Self { scale, offset, clip: Some(clip), radius, alpha: alpha as f32 }
    }

    /// The whole output scaled by `scale` around its center.
    fn zoom(scale: f64, output: (f64, f64), alpha: f64) -> Self {
        let offset = (output.0 * (1.0 - scale) / 2.0, output.1 * (1.0 - scale) / 2.0);
        Self { scale, offset, clip: None, radius: 0.0, alpha: alpha as f32 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Open {
        app_id: u32,
        from: Area,
    },
    /// `app_id`: the app being left, if it still has windows
    Close {
        app_id: Option<u32>,
        to: Area,
    },
}

#[derive(Debug, Clone, Copy)]
struct Active {
    kind: Kind,
    start: Duration,
}

#[derive(Debug, Clone, Copy)]
struct Request {
    target: Target,
    transition: Transition,
    at: Duration,
}

#[derive(Debug, Default)]
pub struct Transitions {
    active: Option<Active>,
    /// The shell's last focus request with a transition, until it brought its target to the front
    request: Option<Request>,
    /// The shell's last focus request carried a transition (it animates, e.g. animations are on)
    enabled: bool,
}

impl Transitions {
    /// The shell asked for `target`; without a transition the next change is instant.
    pub fn request(&mut self, target: Target, transition: Option<Transition>, now: Duration) {
        self.enabled = transition.is_some();
        self.request = transition.map(|transition| Request { target, transition, at: now });
    }

    /// The window in front changed from `old` to `new` (not because of the quick menu).
    /// `leaving_visible`: the app of `old` still shows a window.
    pub fn front_changed(
        &mut self,
        old: Target,
        new: Target,
        leaving_visible: bool,
        output: (f64, f64),
        now: Duration,
    ) {
        self.active = None;
        let request = self.request.filter(|r| r.target == new);
        if request.is_some() {
            self.request = None;
        }
        let kind = match (old, new) {
            (_, Target::App { app_id }) => {
                let Some(request) = request else { return };
                Kind::Open { app_id, from: open_from(&request, output, now) }
            }
            (Target::App { app_id }, Target::Shell) => {
                let tile = match request {
                    Some(request) => request.transition.tile,
                    // The app closed its window: only the shell comes back with an animation.
                    None if self.enabled && !leaving_visible => None,
                    None => return,
                };
                let to = tile.map(Area::from_rect).unwrap_or_else(|| Area::inset(output, 0.3));
                Kind::Close { app_id: leaving_visible.then_some(app_id), to }
            }
            (Target::Shell, Target::Shell) => return,
        };
        tracing::info!("transition {kind:?}");
        self.active = Some(Active { kind, start: now });
    }

    /// Forgets a request whose target is in front already (it cannot start a transition any more).
    pub fn settle(&mut self, front: Target) {
        if self.request.is_some_and(|r| r.target == front) {
            self.request = None;
        }
    }

    /// Drops a finished transition; whether one is running.
    pub fn tick(&mut self, now: Duration) -> bool {
        if let Some(active) = self.active
            && self.progress(&active, now) >= 1.0
        {
            self.active = None;
        }
        self.active.is_some()
    }

    /// How the normal-layer windows of `role` are drawn now; `None`: as they are.
    pub fn effect(&self, role: Role, now: Duration, output: (f64, f64)) -> Option<Effect> {
        let active = self.active?;
        let t = self.progress(&active, now);
        match (active.kind, role) {
            (Kind::Open { app_id, from }, Role::App(id)) if id == app_id => {
                let e = ease_out_quart(t);
                let clip = from.lerp(Area::full(output), e);
                Some(Effect::cover(clip, output, smoothstep(t / 0.3), 1.0 - e))
            }
            (Kind::Close { app_id: Some(app_id), to }, Role::App(id)) if id == app_id => {
                let e = ease_in_out_cubic(t);
                let clip = Area::full(output).lerp(to, e);
                Some(Effect::cover(clip, output, 1.0 - smoothstep((t - 0.55) / 0.45), e))
            }
            (Kind::Close { app_id, .. }, Role::Shell) => {
                let scale = 1.05 - 0.05 * ease_out_cubic(t);
                let dim = if app_id.is_some() { 0.35 } else { 0.0 };
                Some(Effect::zoom(scale, output, dim + (1.0 - dim) * ease_out_cubic(t)))
            }
            _ => None,
        }
    }

    /// Whether the normal-layer windows of `role` are left out: during a return everything below
    /// the shell, so the fading shell shows black underneath and not some background app.
    pub fn hidden(&self, role: Role) -> bool {
        match self.active.map(|a| a.kind) {
            Some(Kind::Close { app_id, .. }) => match role {
                Role::Shell => false,
                Role::App(id) => Some(id) != app_id,
                Role::Unknown => true,
            },
            _ => false,
        }
    }

    /// The app drawn above the shell (the one being left during a return).
    pub fn raised(&self) -> Option<u32> {
        match self.active.map(|a| a.kind) {
            Some(Kind::Close { app_id, .. }) => app_id,
            _ => None,
        }
    }

    fn progress(&self, active: &Active, now: Duration) -> f64 {
        let duration = match active.kind {
            Kind::Open { .. } => OPEN,
            Kind::Close { .. } => CLOSE,
        };
        (now.saturating_sub(active.start).as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0)
    }
}

/// Where an opening app starts: the tile, or the shell's launch screen where it is now (it grows
/// from the tile to fullscreen within `card_ms`). From (nearly) fullscreen a short zoom from a
/// slightly smaller rectangle remains.
fn open_from(request: &Request, output: (f64, f64), now: Duration) -> Area {
    let full = Area::full(output);
    let Some(tile) = request.transition.tile.map(Area::from_rect) else { return Area::inset(output, 0.06) };
    let card = if request.transition.card_ms == 0 {
        0.0
    } else {
        let elapsed = now.saturating_sub(request.at).as_secs_f64() * 1000.0;
        ease_in_out_quart((elapsed / request.transition.card_ms as f64).min(1.0))
    };
    let from = tile.lerp(full, card);
    if from.w * from.h > 0.8 * full.w * full.h { Area::inset(output, 0.06) } else { from }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

fn ease_out_quart(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(4)
}

fn ease_in_out_cubic(t: f64) -> f64 {
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// Slint's `ease-in-out-quart`, which the shell's launch screen uses
fn ease_in_out_quart(t: f64) -> f64 {
    if t < 0.5 { 8.0 * t.powi(4) } else { 1.0 - (-2.0 * t + 2.0).powi(4) / 2.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: (f64, f64) = (1920.0, 1080.0);
    const APP: Target = Target::App { app_id: 3 };

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn tile() -> Transition {
        Transition { tile: Some(Rect { x: 100, y: 500, width: 300, height: 300 }), card_ms: 0 }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn open_zooms_out_of_the_tile_to_fullscreen() {
        let mut transitions = Transitions::default();
        transitions.request(APP, Some(tile()), ms(0));
        transitions.front_changed(Target::Shell, APP, false, OUTPUT, ms(100));

        let start = transitions.effect(Role::App(3), ms(100), OUTPUT).unwrap();
        assert_eq!(start.clip, Some(Area { x: 100.0, y: 500.0, w: 300.0, h: 300.0 }));
        assert_eq!(start.alpha, 0.0);
        // Covers the square tile: scaled by its height, centered horizontally on it.
        assert!(close(start.scale, 300.0 / 1080.0));
        assert!(close(start.offset.0 + 1920.0 * start.scale / 2.0, 250.0));
        assert!(close(start.offset.1, 500.0));
        // Corners like the tile's
        assert!(close(start.radius, 36.0));
        // The shell underneath and other apps stay as they are.
        assert_eq!(transitions.effect(Role::Shell, ms(100), OUTPUT), None);
        assert_eq!(transitions.effect(Role::App(4), ms(100), OUTPUT), None);

        let end = transitions.effect(Role::App(3), ms(100) + OPEN, OUTPUT).unwrap();
        assert!(close(end.scale, 1.0) && close(end.offset.0, 0.0) && close(end.offset.1, 0.0));
        assert_eq!(end.alpha, 1.0);
        assert_eq!(end.radius, 0.0);
        assert!(!transitions.tick(ms(100) + OPEN));
        assert_eq!(transitions.effect(Role::App(3), ms(100) + OPEN, OUTPUT), None);
    }

    #[test]
    fn open_starts_where_the_launch_screen_is() {
        let mut transitions = Transitions::default();
        let request = Transition { card_ms: 400, ..tile() };
        // Half way the launch screen has grown half way (ease-in-out).
        transitions.request(APP, Some(request), ms(0));
        transitions.front_changed(Target::Shell, APP, false, OUTPUT, ms(200));
        let clip = transitions.effect(Role::App(3), ms(200), OUTPUT).unwrap().clip.unwrap();
        assert!(close(clip.x, 50.0) && close(clip.w, 1110.0));

        // Once it covers the screen, the app zooms in from slightly smaller than fullscreen.
        transitions.request(APP, Some(request), ms(0));
        transitions.front_changed(Target::Shell, APP, false, OUTPUT, ms(5000));
        let clip = transitions.effect(Role::App(3), ms(5000), OUTPUT).unwrap().clip.unwrap();
        assert_eq!(clip, Area::inset(OUTPUT, 0.06));
    }

    #[test]
    fn close_shrinks_the_app_into_the_tile_above_the_shell() {
        let mut transitions = Transitions::default();
        transitions.request(Target::Shell, Some(tile()), ms(0));
        transitions.front_changed(APP, Target::Shell, true, OUTPUT, ms(0));
        assert_eq!(transitions.raised(), Some(3));
        assert!(transitions.hidden(Role::App(4)) && transitions.hidden(Role::Unknown));
        assert!(!transitions.hidden(Role::App(3)) && !transitions.hidden(Role::Shell));

        let start = transitions.effect(Role::App(3), ms(0), OUTPUT).unwrap();
        assert!(close(start.scale, 1.0) && start.alpha == 1.0 && start.radius == 0.0);
        let end = transitions.effect(Role::App(3), CLOSE, OUTPUT).unwrap();
        assert_eq!(end.clip, Some(Area { x: 100.0, y: 500.0, w: 300.0, h: 300.0 }));
        assert_eq!(end.alpha, 0.0);
        assert!(close(end.radius, 36.0));

        let shell = transitions.effect(Role::Shell, ms(0), OUTPUT).unwrap();
        assert!(close(shell.scale, 1.05) && shell.alpha > 0.3 && shell.alpha < 0.4);
        assert!(close(shell.offset.0, -48.0) && close(shell.offset.1, -27.0));
        let shell = transitions.effect(Role::Shell, CLOSE, OUTPUT).unwrap();
        assert!(close(shell.scale, 1.0) && shell.alpha == 1.0);
        assert!(!transitions.tick(CLOSE));
        assert_eq!(transitions.raised(), None);
    }

    #[test]
    fn shell_comes_back_animated_after_the_app_closed_its_window() {
        let mut transitions = Transitions::default();
        transitions.request(APP, Some(tile()), ms(0));
        transitions.front_changed(Target::Shell, APP, false, OUTPUT, ms(10));
        // The window is gone before the shell noticed: no request, but the shell animates.
        transitions.front_changed(APP, Target::Shell, false, OUTPUT, ms(5000));
        assert_eq!(transitions.raised(), None);
        assert_eq!(transitions.effect(Role::Shell, ms(5000), OUTPUT).unwrap().alpha, 0.0);
        // Its late focus request changes nothing and is dropped.
        transitions.request(Target::Shell, Some(tile()), ms(5100));
        transitions.settle(Target::Shell);
        assert!(transitions.request.is_none());
    }

    #[test]
    fn changes_without_a_request_are_instant() {
        let mut transitions = Transitions::default();
        transitions.front_changed(Target::Shell, APP, false, OUTPUT, ms(0));
        assert!(!transitions.tick(ms(0)));
        transitions.front_changed(APP, Target::Shell, false, OUTPUT, ms(0));
        assert!(!transitions.tick(ms(0)));
        // Animations off: the shell sends no transition, also no automatic return.
        transitions.request(APP, None, ms(0));
        transitions.front_changed(Target::Shell, APP, false, OUTPUT, ms(0));
        transitions.front_changed(APP, Target::Shell, false, OUTPUT, ms(0));
        assert!(!transitions.tick(ms(0)));
        // The app came back without a request although it still has its window (instant).
        transitions.request(APP, Some(tile()), ms(0));
        transitions.front_changed(Target::Shell, APP, false, OUTPUT, ms(0));
        transitions.front_changed(APP, Target::Shell, true, OUTPUT, ms(1000));
        assert!(!transitions.tick(ms(1000)));
    }

    #[test]
    fn a_request_for_another_app_does_not_animate() {
        let mut transitions = Transitions::default();
        transitions.request(Target::App { app_id: 9 }, Some(tile()), ms(0));
        transitions.front_changed(Target::Shell, APP, false, OUTPUT, ms(0));
        assert!(!transitions.tick(ms(0)));
        assert!(transitions.request.is_some());
    }
}
