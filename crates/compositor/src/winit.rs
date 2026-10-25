//! Nested backend: the compositor runs in a window on the current desktop (development).
//!
//! Based on anvil's `winit.rs` (MIT, Copyright (c) 2017 Victor Berger).

use std::os::fd::OwnedFd;
use std::time::Duration;

use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::egl::EGLDevice;
use smithay::backend::egl::context::GlAttributes;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{ImportDma, ImportEgl, ImportMemWl};
use smithay::backend::winit::{self, WinitEvent, WinitGraphicsBackend};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::reexports::wayland_server::Display;
use smithay::reexports::winit::dpi::LogicalSize;
use smithay::reexports::winit::event_loop::pump_events::PumpStatus;
use smithay::reexports::winit::window::WindowAttributes;
use smithay::utils::{DeviceFd, Transform};
use smithay::wayland::dmabuf::DmabufFeedbackBuilder;
use smithay::wayland::drm_syncobj::{DrmSyncobjState, supports_syncobj_eventfd};
use smithay::wayland::presentation::Refresh;
use tracing::{info, warn};

use crate::backend::Backend;
use crate::options::Options;
use crate::render;
use crate::state::State;

pub struct WinitData {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: OutputDamageTracker,
}

const REFRESH_MHZ: i32 = 60_000;

pub fn run(options: &Options) -> Result<i32, Box<dyn std::error::Error>> {
    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let display: Display<State> = Display::new()?;
    let dh = display.handle();

    let (width, height) = options.size;
    let attributes = WindowAttributes::default()
        .with_surface_size(LogicalSize::new(width as f64, height as f64))
        .with_title("ConsoleUI compositor")
        .with_visible(true);
    // vsync: swap buffers waits for the host, which paces our frames (and the clients').
    let gl = GlAttributes { version: (3, 0), profile: None, debug: false, vsync: true };
    let (mut backend, mut winit) = winit::init_from_attributes_with_gl_attr::<GlesRenderer>(attributes, gl)?;

    let output = Output::new(
        "winit".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "ConsoleUI".into(),
            model: "nested".into(),
            serial_number: "0".into(),
        },
    );
    let _global = output.create_global::<State>(&dh);
    let mode = Mode { size: backend.window_size(), refresh: REFRESH_MHZ };
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    output.set_preferred(mode);

    if backend.renderer().bind_wl_display(&dh).is_ok() {
        info!("EGL hardware acceleration (wl_drm) enabled");
    }
    let render_node = EGLDevice::device_for_display(backend.renderer().egl_context().display())
        .ok()
        .and_then(|device| device.try_get_render_node().ok().flatten());
    let formats = backend.renderer().dmabuf_formats();
    let shm_formats = backend.renderer().shm_formats();

    let data = WinitData { damage_tracker: OutputDamageTracker::from_output(&output), backend };
    let mut state = State::new(display, event_loop.handle(), event_loop.get_signal(), Backend::Winit(Box::new(data)))?;
    state.shm_state.update_formats(shm_formats);

    match render_node {
        Some(node) => {
            let feedback = DmabufFeedbackBuilder::new(node.dev_id(), formats).build()?;
            state.dmabuf_global =
                Some(state.dmabuf_state.create_global_with_default_feedback::<State>(&state.display_handle, &feedback));
            // Explicit sync (important on NVIDIA): syncobjs are imported on the render node.
            match node.dev_path().map(std::fs::File::open) {
                Some(Ok(file)) => {
                    let fd = DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(file)));
                    if supports_syncobj_eventfd(&fd) {
                        state.syncobj_state = Some(DrmSyncobjState::new::<State>(&state.display_handle, fd));
                        info!("explicit sync (linux-drm-syncobj) enabled on {node}");
                    }
                }
                Some(Err(err)) => warn!("cannot open {node} for explicit sync: {err}"),
                None => {}
            }
        }
        None => {
            warn!("no render node, linux-dmabuf v3 without feedback");
            state.dmabuf_global = Some(state.dmabuf_state.create_global::<State>(&state.display_handle, formats));
        }
    }

    state.set_output(output.clone());
    state.start(options);

    while state.running && !crate::shutdown_requested() {
        let status = winit.dispatch_new_events(|event| match event {
            WinitEvent::Resized { size, .. } => {
                let mode = Mode { size, refresh: REFRESH_MHZ };
                output.change_current_state(Some(mode), None, None, None);
                output.set_preferred(mode);
                state.set_output(output.clone());
            }
            WinitEvent::Input(event) => state.process_input(event),
            WinitEvent::CloseRequested => state.running = false,
            _ => {}
        });
        if let PumpStatus::Exit(_) = status {
            state.running = false;
            break;
        }

        state.render_winit(&output);

        if event_loop.dispatch(Some(Duration::from_millis(1)), &mut state).is_err() {
            state.running = false;
        }
        state.space.refresh();
        state.popups.cleanup();
        state.control.flush();
        let _ = state.display_handle.flush_clients();
    }
    Ok(state.shutdown())
}

impl State {
    fn render_winit(&mut self, output: &Output) {
        let Backend::Winit(data) = &mut self.backend else { return };
        let pointer = self.pointer.current_location();
        let age = data.backend.buffer_age().unwrap_or(0);
        let result = data.backend.bind().and_then(|(renderer, mut framebuffer)| {
            let elements = render::output_elements(renderer, &self.space, output, &self.cursor_status, pointer);
            let result = data
                .damage_tracker
                .render_output(renderer, &mut framebuffer, age, &elements, render::CLEAR_COLOR)
                .map_err(|err| match err {
                    smithay::backend::renderer::damage::Error::Rendering(err) => err.into(),
                    smithay::backend::renderer::damage::Error::OutputNoMode(_) => {
                        smithay::backend::SwapBuffersError::AlreadySwapped
                    }
                })?;
            Ok((result.damage.cloned(), result.states))
        });
        let (damage, states) = match result {
            Ok(result) => result,
            Err(smithay::backend::SwapBuffersError::ContextLost(err)) => {
                warn!("rendering context lost: {err}");
                self.running = false;
                return;
            }
            Err(err) => {
                warn!("rendering failed: {err}");
                return;
            }
        };
        let cursor_visible = !matches!(self.cursor_status, smithay::input::pointer::CursorImageStatus::Surface(_));
        if let Some(damage) = &damage
            && let Err(err) = data.backend.submit(Some(damage))
        {
            warn!("cannot submit the frame: {err}");
        }
        data.backend.window().set_cursor_visible(cursor_visible);

        render::update_primary_scanout_output(&self.space, output, &states);
        let now = self.clock.now();
        if damage.is_some() {
            let mut feedback = render::take_presentation_feedback(output, &self.space, &states);
            feedback.presented(
                now,
                Refresh::fixed(Duration::from_secs_f64(1000.0 / REFRESH_MHZ as f64)),
                0,
                wp_presentation_feedback::Kind::Vsync,
            );
        }
        render::send_frames(&self.space, output, now.into(), &self.cursor_status);
        self.take_screenshots(output);
    }

    /// Serves pending screenshot requests with the current window contents.
    pub fn take_screenshots(&mut self, output: &Output) {
        if self.screenshots.is_empty() {
            return;
        }
        let Some(size) = output.current_mode().map(|mode| mode.size) else { return };
        let pointer = self.pointer.current_location();
        let requests = std::mem::take(&mut self.screenshots);
        let Some(renderer) = self.backend.renderer() else { return };
        let elements = render::output_elements(renderer, &self.space, output, &self.cursor_status, pointer);
        let results: Vec<_> = requests
            .into_iter()
            .map(|request| {
                let result = render::save_screenshot(renderer, &elements, size, &request.path).map_err(|e| e.to_string());
                (request, result)
            })
            .collect();
        drop(elements);
        for (request, result) in results {
            self.screenshot_done(request, result);
        }
    }
}
