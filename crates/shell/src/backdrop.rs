//! The animated background (settings row "Background"): a thread of its own renders
//! `consoleui_core::backdrop` frames (a few ms each, too much for the UI thread) and glides to the
//! accent color of the focused entry; a UI timer picks up the newest frame. With animations off
//! it renders one still frame per change; while an app is in front or the artwork covers it,
//! nothing runs.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::time::{Duration, Instant};

use consoleui_core::backdrop::{self, HEIGHT, Palette, Style, WIDTH};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode};

use crate::AppWindow;

/// About 30 frames per second; the motion is slow and the frame is scaled up smoothly anyway.
const FRAME: Duration = Duration::from_millis(33);
/// How often the UI looks for a new frame while the background runs
const POLL: Duration = Duration::from_millis(15);
/// Time constant of the color change towards a new accent
const COLOR_GLIDE: f32 = 0.45;

#[derive(Clone, Copy, PartialEq)]
struct Control {
    color: [f32; 3],
    style: Style,
    palette: Palette,
    /// Visible and the shell in front
    run: bool,
    animate: bool,
}

/// How the background looks and whether it moves
#[derive(Clone, Copy)]
pub struct Look {
    pub animate: bool,
    pub style: Style,
    pub palette: Palette,
}

pub struct Backdrop {
    sender: Sender<Control>,
    frames: std::rc::Rc<Receiver<Vec<u8>>>,
    timer: Timer,
    control: Option<Control>,
}

impl Backdrop {
    pub fn new() -> Self {
        let (sender, controls) = mpsc::channel();
        let (frame_sender, frames) = mpsc::sync_channel(2);
        std::thread::spawn(move || run(controls, frame_sender));
        Self { sender, frames: std::rc::Rc::new(frames), timer: Timer::default(), control: None }
    }

    /// `shown`: the background is visible; `in_front`: the shell is in front (no app, no launch
    /// screen); `look`: style, colors and the animations setting; `accent`: the color to glide to.
    pub fn update(&mut self, ui: &AppWindow, shown: bool, in_front: bool, look: Look, accent: slint::Color) {
        let Look { animate, style, palette } = look;
        ui.set_backdrop_shown(shown);
        let color = [accent.red(), accent.green(), accent.blue()].map(|c| c as f32 / 255.0);
        let control = Control { color, style, palette, run: shown && in_front, animate };
        if self.control == Some(control) {
            return;
        }
        self.control = Some(control);
        let _ = self.sender.send(control);
        if control.run && !self.timer.running() {
            let frames = self.frames.clone();
            let weak = ui.as_weak();
            self.timer.start(TimerMode::Repeated, POLL, move || {
                let Some(ui) = weak.upgrade() else { return };
                if let Some(bytes) = frames.try_iter().last() {
                    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&bytes, WIDTH as u32, HEIGHT as u32);
                    ui.set_backdrop(Image::from_rgba8(buffer));
                }
            });
        } else if !control.run {
            self.timer.stop();
        }
    }
}

/// The render thread: frames at [`FRAME`] intervals while running and animated, one frame per
/// change otherwise.
fn run(controls: Receiver<Control>, frames: SyncSender<Vec<u8>>) {
    let start = Instant::now();
    let mut control = Control {
        color: [0.4, 0.4, 0.9],
        style: Style::default(),
        palette: Palette::default(),
        run: false,
        animate: false,
    };
    let mut color: Option<[f32; 3]> = None;
    let mut pending = false;
    let mut last = Instant::now();
    loop {
        let busy = control.run && (control.animate || pending);
        let next = if busy {
            match controls.try_recv() {
                Ok(next) => Some(next),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else {
            match controls.recv_timeout(Duration::from_secs(60)) {
                Ok(next) => Some(next),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        if let Some(next) = next {
            control = next;
            pending = true;
            continue;
        }
        if !busy {
            continue;
        }
        let now = Instant::now();
        let dt = now.duration_since(last).as_secs_f32();
        last = now;
        let follow = if control.animate { 1.0 - (-dt / COLOR_GLIDE).exp() } else { 1.0 };
        let mut current = color.unwrap_or(control.color);
        for (c, target) in current.iter_mut().zip(control.color) {
            *c += (target - *c) * follow;
        }
        color = Some(current);
        let time = if control.animate { start.elapsed().as_secs_f32() } else { 12.0 };
        let mut bytes = vec![0; WIDTH * HEIGHT * 4];
        backdrop::render(control.style, control.palette, WIDTH, HEIGHT, time, current, &mut bytes);
        // The UI takes the newest frame; one that is still waiting is simply replaced next time.
        let _ = frames.try_send(bytes);
        pending = false;
        if control.animate {
            std::thread::sleep(FRAME.saturating_sub(now.elapsed()));
        }
    }
}
