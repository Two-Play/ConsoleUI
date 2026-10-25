//! Where the shell's overlay and notification windows get a blurred backdrop (Story 7.13), and how
//! it fades: the Smithay-free part of the blur. The shell announces its panels with
//! `ShellMessage::Blur`; `blur.rs` renders what is below them and draws it blurred.

use std::time::Duration;

use consoleui_core::protocol::{BlurRegion, WindowRole};

/// The blurred panels of one window role, fading in when they appear and out when they go.
#[derive(Debug, Clone, Default)]
pub struct Backdrop {
    /// The last non-empty regions (kept while fading out)
    regions: Vec<BlurRegion>,
    from: f32,
    to: f32,
    start: Duration,
    fade: Duration,
}

impl Backdrop {
    /// New regions at `now`; an empty list fades the current ones out. Moving regions (a sliding
    /// panel sends them every frame) keep a running fade going.
    pub fn set(&mut self, regions: Vec<BlurRegion>, fade: Duration, now: Duration) {
        let regions: Vec<BlurRegion> = regions.into_iter().filter(|r| r.rect.width > 0 && r.rect.height > 0).collect();
        let to = if regions.is_empty() { 0.0 } else { 1.0 };
        if !regions.is_empty() {
            self.regions = regions;
        }
        if to != self.to {
            self.from = self.alpha(now);
            self.to = to;
            self.start = now;
            self.fade = fade;
        }
    }

    /// Opacity of the blur at `now` (0 to 1).
    pub fn alpha(&self, now: Duration) -> f32 {
        let elapsed = now.saturating_sub(self.start);
        if self.fade.is_zero() || elapsed >= self.fade {
            return self.to;
        }
        let t = elapsed.as_secs_f32() / self.fade.as_secs_f32();
        self.from + (self.to - self.from) * t
    }

    /// The regions to blur at `now` with their opacity; `None` once faded out.
    pub fn visible(&self, now: Duration) -> Option<(&[BlurRegion], f32)> {
        let alpha = self.alpha(now);
        (alpha > 0.0 && !self.regions.is_empty()).then_some((self.regions.as_slice(), alpha))
    }
}

/// The backdrops of the overlay (quick menu) and notification (toast) windows.
#[derive(Debug, Clone, Default)]
pub struct Backdrops {
    overlay: Backdrop,
    notification: Backdrop,
}

impl Backdrops {
    pub fn get(&self, role: WindowRole) -> &Backdrop {
        match role {
            WindowRole::Overlay => &self.overlay,
            WindowRole::Notification => &self.notification,
        }
    }

    pub fn get_mut(&mut self, role: WindowRole) -> &mut Backdrop {
        match role {
            WindowRole::Overlay => &mut self.overlay,
            WindowRole::Notification => &mut self.notification,
        }
    }
}

/// A rectangle in physical output pixels: `(x, y, width, height)`.
pub type PixelRect = (i32, i32, i32, i32);

/// What has to be rendered to blur `regions`: their bounding box grown by `padding` (the blur
/// reaches that far), clipped to the output. `None` if nothing of it is on the output.
pub fn capture_rect(regions: &[PixelRect], padding: i32, output: (i32, i32)) -> Option<PixelRect> {
    let mut regions = regions.iter().filter(|r| r.2 > 0 && r.3 > 0);
    let first = *regions.next()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0 + first.2, first.1 + first.3);
    for r in regions {
        x0 = x0.min(r.0);
        y0 = y0.min(r.1);
        x1 = x1.max(r.0 + r.2);
        y1 = y1.max(r.1 + r.3);
    }
    let (x0, y0) = ((x0 - padding).max(0), (y0 - padding).max(0));
    let (x1, y1) = ((x1 + padding).min(output.0), (y1 + padding).min(output.1));
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

#[cfg(test)]
mod tests {
    use consoleui_core::protocol::Rect;

    use super::*;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn region(x: i32) -> BlurRegion {
        BlurRegion { rect: Rect { x, y: 0, width: 10, height: 10 }, radius: 2 }
    }

    #[test]
    fn fades_in_and_out() {
        let mut backdrop = Backdrop::default();
        assert!(backdrop.visible(ms(0)).is_none());
        backdrop.set(vec![region(5)], ms(200), ms(100));
        assert!(backdrop.visible(ms(100)).is_none());
        assert!((backdrop.alpha(ms(200)) - 0.5).abs() < 1e-4);
        assert_eq!(backdrop.visible(ms(300)), Some((&[region(5)][..], 1.0)));

        // Fading out keeps the old regions until the blur is gone.
        backdrop.set(Vec::new(), ms(200), ms(400));
        assert_eq!(backdrop.visible(ms(500)).map(|(regions, _)| regions), Some(&[region(5)][..]));
        assert!(backdrop.visible(ms(600)).is_none());
    }

    #[test]
    fn a_new_fade_starts_where_the_old_one_is() {
        let mut backdrop = Backdrop::default();
        backdrop.set(vec![region(0)], ms(100), ms(0));
        backdrop.set(Vec::new(), ms(100), ms(50));
        assert!((backdrop.alpha(ms(50)) - 0.5).abs() < 1e-4);
        assert!((backdrop.alpha(ms(100)) - 0.25).abs() < 1e-4);
        // Without a fade the change is instant; empty regions count as none.
        backdrop.set(vec![region(1)], Duration::ZERO, ms(80));
        assert_eq!(backdrop.alpha(ms(80)), 1.0);
        let empty = BlurRegion { rect: Rect { x: 0, y: 0, width: 0, height: 5 }, radius: 0 };
        backdrop.set(vec![empty], Duration::ZERO, ms(90));
        assert!(backdrop.visible(ms(90)).is_none());
    }

    #[test]
    fn moving_regions_keep_the_fade() {
        let mut backdrop = Backdrop::default();
        backdrop.set(vec![region(0)], ms(100), ms(0));
        backdrop.set(vec![region(3)], ms(100), ms(50));
        assert_eq!(backdrop.visible(ms(100)), Some((&[region(3)][..], 1.0)));
    }

    #[test]
    fn roles_have_their_own_backdrop() {
        let mut backdrops = Backdrops::default();
        backdrops.get_mut(WindowRole::Notification).set(vec![region(0)], Duration::ZERO, ms(0));
        assert!(backdrops.get(WindowRole::Notification).visible(ms(0)).is_some());
        assert!(backdrops.get(WindowRole::Overlay).visible(ms(0)).is_none());
    }

    #[test]
    fn capture_covers_the_regions_plus_padding_on_the_output() {
        assert_eq!(capture_rect(&[], 8, (100, 100)), None);
        assert_eq!(capture_rect(&[(10, 20, 30, 10)], 5, (100, 100)), Some((5, 15, 40, 20)));
        assert_eq!(capture_rect(&[(10, 20, 30, 10), (60, 70, 10, 10)], 0, (100, 100)), Some((10, 20, 60, 60)));
        // Clipped to the output; off the output there is nothing to capture.
        assert_eq!(capture_rect(&[(-10, 90, 30, 30)], 4, (100, 100)), Some((0, 86, 24, 14)));
        assert_eq!(capture_rect(&[(200, 0, 10, 10)], 4, (100, 100)), None);
        assert_eq!(capture_rect(&[(10, 10, 0, 10)], 4, (100, 100)), None);
    }
}
