//! What a frame consists of (the windows, transformed during a transition, plus the client
//! cursor), frame callbacks and presentation feedback after a frame, and the offscreen screenshot.
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
use smithay::backend::renderer::element::utils::{
    CropRenderElement, Relocate, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::{
    AsRenderElements, Kind, RenderElementStates, default_primary_scanout_output_compare,
};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, ExportMem, Offscreen, TextureMapping};
use smithay::desktop::utils::{
    OutputPresentationFeedback, send_frames_surface_tree, surface_presentation_feedback_flags_from_states,
    surface_primary_scanout_output, update_surface_primary_scanout_output,
};
use smithay::desktop::{Space, Window};
use smithay::input::pointer::{CursorImageAttributes, CursorImageStatus};
use smithay::output::Output;
use smithay::utils::{IsAlive, Logical, Physical, Point, Rectangle, Scale, Size, Transform};
use smithay::wayland::compositor::with_states;

use consoleui_core::protocol::{BlurRegion, WindowRole};

use crate::blur::{self, BackdropElement, BlurCache};
use crate::rounded::{self, RoundedSurface};
use crate::transition::Effect;

/// Background where no window covers the output
pub const CLEAR_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

type Transformed = CropRenderElement<RelocateRenderElement<RescaleRenderElement<RoundedSurface>>>;

smithay::backend::renderer::element::render_elements! {
    pub OutputElement<=GlesRenderer>;
    /// Windows and the client cursor
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    /// Windows during a transition
    Transformed=Transformed,
    /// Blurred backdrop under an overlay or notification window (Story 7.13)
    Backdrop=BackdropElement,
}

/// A window in a frame: where it is drawn (its render location) and how a transition changes it.
pub struct Drawn {
    pub window: Window,
    pub location: Point<i32, Logical>,
    pub effect: Option<Effect>,
    /// The window's panels get a blurred backdrop
    pub blur: Option<Blur>,
}

/// Blur under a shell window (Story 7.13): its regions in window coordinates and the opacity.
pub struct Blur {
    pub role: WindowRole,
    pub regions: Vec<BlurRegion>,
    pub alpha: f32,
}

/// Elements of one frame, front to back; `windows` front to back as well. `cache` keeps the
/// blurred backdrops between frames.
pub fn output_elements(
    renderer: &mut GlesRenderer,
    windows: &[Drawn],
    output: &Output,
    cursor: &CursorImageStatus,
    pointer: Point<f64, Logical>,
    cache: &mut BlurCache,
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
                .map(OutputElement::Surface),
        );
    }
    let output_size = output.current_mode().map(|mode| mode.size).unwrap_or_default();
    let mut blurred = Vec::new();
    for (index, drawn) in windows.iter().enumerate() {
        elements.extend(window_elements(renderer, drawn, scale, output_size));
        let Some(window_blur) = &drawn.blur else { continue };
        let regions: Vec<(Rectangle<i32, Physical>, f64)> = window_blur
            .regions
            .iter()
            .map(|region| {
                let rect = Rectangle::<i32, Logical>::new(
                    drawn.location + Point::from((region.rect.x, region.rect.y)),
                    (region.rect.width, region.rect.height).into(),
                );
                (rect.to_physical_precise_round(scale), region.radius as f64 * scale.x)
            })
            .collect();
        let below: Vec<OutputElement> = windows[index + 1..]
            .iter()
            .flat_map(|drawn| window_elements(renderer, drawn, scale, output_size))
            .collect();
        let backdrop =
            blur::backdrop(renderer, cache, window_blur.role, &regions, window_blur.alpha, below, output_size, scale);
        elements.extend(backdrop.into_iter().map(OutputElement::Backdrop));
        blurred.push(window_blur.role);
    }
    cache.retain(&blurred);
    elements
}

/// The elements of one window, transformed by its transition.
fn window_elements(
    renderer: &mut GlesRenderer,
    drawn: &Drawn,
    scale: Scale<f64>,
    output_size: Size<i32, Physical>,
) -> Vec<OutputElement> {
    let location = drawn.location.to_physical_precise_round(scale);
    let Some(effect) = drawn.effect else {
        let window_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            drawn.window.render_elements(renderer, location, scale, 1.0);
        return window_elements.into_iter().map(OutputElement::Surface).collect();
    };
    let window_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        drawn.window.render_elements(renderer, location, scale, effect.alpha);
    let offset = Point::<f64, Logical>::from(effect.offset).to_physical(scale).to_i32_round();
    let clip = match effect.clip {
        Some(area) => {
            Rectangle::<f64, Logical>::new((area.x, area.y).into(), (area.w, area.h).into()).to_physical(scale)
        }
        None => Rectangle::from_size(output_size).to_f64(),
    };
    let radius = effect.radius * scale.x;
    let program = if radius >= 0.5 { rounded::program(renderer) } else { None };
    window_elements
        .into_iter()
        .filter_map(|element| {
            let element = RoundedSurface::new(element, program.clone(), clip, radius);
            let element = RescaleRenderElement::from_element(element, Point::default(), effect.scale);
            let element = RelocateRenderElement::from_element(element, offset, Relocate::Relative);
            CropRenderElement::from_element(element, scale, clip.to_i32_round()).map(OutputElement::Transformed)
        })
        .collect()
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
