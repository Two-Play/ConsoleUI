//! What a frame consists of (the space's windows plus the client cursor), frame callbacks and
//! presentation feedback after a frame, and the offscreen screenshot.
//!
//! The frame bookkeeping follows anvil (`anvil/src/state.rs`, `render.rs`, MIT, Copyright (c)
//! 2017 Victor Berger).

use std::error::Error;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree};
use smithay::backend::renderer::element::{Kind, RenderElementStates, default_primary_scanout_output_compare};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, ExportMem, Offscreen, TextureMapping};
use smithay::desktop::space::SpaceRenderElements;
use smithay::desktop::utils::{
    OutputPresentationFeedback, send_frames_surface_tree, surface_presentation_feedback_flags_from_states,
    surface_primary_scanout_output, update_surface_primary_scanout_output,
};
use smithay::desktop::{Space, Window};
use smithay::input::pointer::{CursorImageAttributes, CursorImageStatus};
use smithay::output::Output;
use smithay::utils::{IsAlive, Logical, Physical, Point, Rectangle, Scale, Size, Transform};
use smithay::wayland::compositor::with_states;

/// Background where no window covers the output
pub const CLEAR_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

smithay::backend::renderer::element::render_elements! {
    pub OutputElement<=GlesRenderer>;
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Cursor=WaylandSurfaceRenderElement<GlesRenderer>,
}

/// Elements of one frame, front to back.
pub fn output_elements(
    renderer: &mut GlesRenderer,
    space: &Space<Window>,
    output: &Output,
    cursor: &CursorImageStatus,
    pointer: Point<f64, Logical>,
) -> Vec<OutputElement> {
    let scale = Scale::from(output.current_scale().fractional_scale());
    let mut elements: Vec<OutputElement> = Vec::new();
    // Only a cursor the client drew itself; the named default cursor is the host's (winit) or
    // hidden (a console is driven by the gamepad).
    if let CursorImageStatus::Surface(surface) = cursor
        && surface.alive()
    {
        let hotspot = with_states(surface, |states| {
            states
                .data_map
                .get::<Mutex<CursorImageAttributes>>()
                .map(|attributes| attributes.lock().unwrap().hotspot)
                .unwrap_or_default()
        });
        let position = (pointer - hotspot.to_f64()).to_physical(scale).to_i32_round();
        elements.extend(
            render_elements_from_surface_tree(renderer, surface, position, scale, 1.0, Kind::Cursor)
                .into_iter()
                .map(OutputElement::Cursor),
        );
    }
    if let Ok(space_elements) = smithay::desktop::space::space_render_elements(renderer, [space], output, 1.0) {
        elements.extend(space_elements.into_iter().map(OutputElement::Space));
    }
    elements
}

/// Which output each surface was last shown on (drives frame throttling and feedback).
pub fn update_primary_scanout_output(space: &Space<Window>, output: &Output, states: &RenderElementStates) {
    for window in space.elements() {
        window.with_surfaces(|surface, surface_states| {
            update_surface_primary_scanout_output(
                surface,
                output,
                surface_states,
                None,
                states,
                default_primary_scanout_output_compare,
            );
        });
    }
}

pub fn take_presentation_feedback(
    output: &Output,
    space: &Space<Window>,
    states: &RenderElementStates,
) -> OutputPresentationFeedback {
    let mut feedback = OutputPresentationFeedback::new(output);
    for window in space.elements() {
        if space.outputs_for_element(window).contains(output) {
            window.take_presentation_feedback(&mut feedback, surface_primary_scanout_output, |surface, _| {
                surface_presentation_feedback_flags_from_states(surface, None, states)
            });
        }
    }
    feedback
}

/// Frame callbacks after a frame. Windows hidden behind the one in front get theirs at most once
/// a second, so background apps barely render.
pub fn send_frames(space: &Space<Window>, output: &Output, time: Duration, cursor: &CursorImageStatus) {
    let throttle = Some(Duration::from_secs(1));
    for window in space.elements() {
        window.send_frame(output, time, throttle, surface_primary_scanout_output);
    }
    if let CursorImageStatus::Surface(surface) = cursor {
        send_frames_surface_tree(surface, output, time, throttle, |_, _| Some(output.clone()));
    }
}

/// Renders `elements` into an offscreen texture and saves it as PNG. Offscreen instead of reading
/// back the window/scanout buffer: the spike's read-back of anvil's EGL window surface caused
/// `EGL_BAD_ALLOC` on NVIDIA.
pub fn save_screenshot(
    renderer: &mut GlesRenderer,
    elements: &[OutputElement],
    size: Size<i32, Physical>,
    path: &Path,
) -> Result<(), Box<dyn Error>> {
    let buffer_size = size.to_logical(1).to_buffer(1, Transform::Normal);
    let mut texture: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, buffer_size)?;
    let mut damage_tracker = OutputDamageTracker::new(size, 1.0, Transform::Normal);
    let mut framebuffer = renderer.bind(&mut texture)?;
    damage_tracker.render_output(renderer, &mut framebuffer, 0, elements, CLEAR_COLOR)?;
    let mapping = renderer.copy_framebuffer(&framebuffer, Rectangle::from_size(buffer_size), Fourcc::Abgr8888)?;
    let flipped = mapping.flipped();
    let pixels = renderer.map_texture(&mapping)?;
    let (width, height) = (size.w as usize, size.h as usize);
    let stride = width * 4;
    let mut rows = Vec::with_capacity(stride * height);
    for row in 0..height {
        // glReadPixels delivers the bottom row first unless the mapping says it is already flipped.
        let source = if flipped { row } else { height - 1 - row };
        rows.extend_from_slice(&pixels[source * stride..(source + 1) * stride]);
    }
    // Opaque output: drop whatever alpha the clients left in the buffer.
    for pixel in rows.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rows)?;
    Ok(())
}
