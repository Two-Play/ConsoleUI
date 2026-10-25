//! The two backends behind one type, so protocol handlers do not need to be generic.

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::input::keyboard::LedState;

use crate::drm::DrmData;
use crate::winit::WinitData;

pub enum Backend {
    /// Nested in a window (development)
    Winit(Box<WinitData>),
    /// DRM/KMS on a TTY (the session)
    Drm(Box<DrmData>),
}

impl Backend {
    pub fn seat_name(&self) -> String {
        match self {
            Backend::Winit(_) => "winit".into(),
            Backend::Drm(data) => data.seat_name(),
        }
    }

    pub fn renderer(&mut self) -> Option<&mut GlesRenderer> {
        match self {
            Backend::Winit(data) => Some(data.backend.renderer()),
            Backend::Drm(data) => data.renderer(),
        }
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Backend::Drm(data) = self {
            data.change_vt(vt);
        }
    }

    pub fn update_led_state(&mut self, leds: LedState) {
        if let Backend::Drm(data) = self {
            data.update_led_state(leds);
        }
    }
}
