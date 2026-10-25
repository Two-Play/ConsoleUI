//! Backdrop blur behind the shell's overlay and notification windows (Story 7.13).
//!
//! What is below a window's blur regions (plus a margin the blur reaches into) is rendered at
//! half resolution into an offscreen texture, blurred with the dual Kawase filter (Marius Bjørge,
//! "Bandwidth-Efficient Rendering", SIGGRAPH 2015: a few downsampling passes, then as many
//! upsampling passes, each reading five or eight bilinear samples) and drawn under the window,
//! cut to the regions with the rounded-clip shader of `rounded.rs`. Only the captured area is
//! rendered, at most 1/4 of its pixels per pass, and nothing is rendered again while the windows
//! below do not change, so a paused game or the static shell costs nothing.
//!
//! niri (GPL-3.0-or-later, Copyright (c) Ivan Molodetskikh) and KWin blur the same way; the code
//! here is our own.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement, RescaleRenderElement};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType, ffi,
};
use smithay::backend::renderer::utils::{CommitCounter, OpaqueRegions};
use smithay::backend::renderer::{Bind, Frame, Offscreen, Renderer, Texture};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform};
use tracing::warn;

use consoleui_core::protocol::WindowRole;

use crate::backdrop;
use crate::rounded;

/// Downsampling passes after the half-resolution capture (and as many upsampling passes): the
/// smallest texture is 1/16 of the captured area per side.
const PASSES: usize = 3;
/// Sample distance in texels of each pass (larger: wider blur, at some point blocky)
const OFFSET: f32 = 1.6;
/// How far (physical pixels) the blur reaches outside a region; that much more is captured.
pub const PADDING: i32 = 64;

/// The texture shader template of Smithay with the dual Kawase sampling in place of one sample.
fn shader(sampling: &str) -> String {
    format!(
        r#"
#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform vec2 half_pixel;

void main() {{
    vec2 uv = v_coords;
    vec2 o = half_pixel;
    {sampling}

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}}
"#
    )
}

const DOWN: &str = r#"
    vec4 color = texture2D(tex, uv) * 4.0;
    color += texture2D(tex, uv - o);
    color += texture2D(tex, uv + o);
    color += texture2D(tex, uv + vec2(o.x, -o.y));
    color += texture2D(tex, uv - vec2(o.x, -o.y));
    color /= 8.0;
"#;

const UP: &str = r#"
    vec4 color = texture2D(tex, uv + vec2(-o.x * 2.0, 0.0));
    color += texture2D(tex, uv + vec2(-o.x, o.y)) * 2.0;
    color += texture2D(tex, uv + vec2(0.0, o.y * 2.0));
    color += texture2D(tex, uv + vec2(o.x, o.y)) * 2.0;
    color += texture2D(tex, uv + vec2(o.x * 2.0, 0.0));
    color += texture2D(tex, uv + vec2(o.x, -o.y)) * 2.0;
    color += texture2D(tex, uv + vec2(0.0, -o.y * 2.0));
    color += texture2D(tex, uv + vec2(-o.x, -o.y)) * 2.0;
    color /= 12.0;
"#;

#[derive(Clone)]
struct Programs {
    down: GlesTexProgram,
    up: GlesTexProgram,
}

/// The compiled blur shaders, kept in the EGL context's user data (`None`: they failed to compile).
struct CompiledPrograms(Option<Programs>);

fn programs(renderer: &mut GlesRenderer) -> Option<Programs> {
    if let Some(CompiledPrograms(programs)) = renderer.egl_context().user_data().get::<CompiledPrograms>() {
        return programs.clone();
    }
    let uniforms = [UniformName::new("half_pixel", UniformType::_2f)];
    let programs = renderer
        .compile_custom_texture_shader(shader(DOWN), &uniforms)
        .and_then(|down| Ok(Programs { down, up: renderer.compile_custom_texture_shader(shader(UP), &uniforms)? }))
        .inspect_err(|err| warn!("blur shaders: {err}"))
        .ok();
    renderer.egl_context().user_data().insert_if_missing(|| CompiledPrograms(programs.clone()));
    programs
}

/// What the last blur of one window was made from: the elements below (id, commit, geometry,
/// alpha) and the captured area.
type Key = (Vec<(Id, CommitCounter, Rectangle<i32, Physical>, u32)>, Rectangle<i32, Physical>);

/// The blur of one window role, kept across frames.
struct Entry {
    role: WindowRole,
    key: Option<Key>,
    /// Half resolution capture (the result after the passes), then each downsampled level
    textures: Vec<GlesTexture>,
    /// One element id per region
    ids: Vec<Id>,
    commit: CommitCounter,
    alpha: f32,
}

/// Blur textures and element state per window role.
#[derive(Default)]
pub struct BlurCache {
    entries: Vec<Entry>,
}

impl BlurCache {
    fn entry(&mut self, role: WindowRole) -> &mut Entry {
        let index = match self.entries.iter().position(|e| e.role == role) {
            Some(index) => index,
            None => {
                self.entries.push(Entry {
                    role,
                    key: None,
                    textures: Vec::new(),
                    ids: Vec::new(),
                    commit: CommitCounter::default(),
                    alpha: 0.0,
                });
                self.entries.len() - 1
            }
        };
        &mut self.entries[index]
    }

    /// Frees the textures of roles without a blur in this frame.
    pub fn retain(&mut self, roles: &[WindowRole]) {
        self.entries.retain(|entry| roles.contains(&entry.role));
    }
}

/// The blurred backdrop of `regions` (physical output pixels, with their corner radius), made from
/// `below` (the elements under the window, front to back) and drawn with `alpha`.
#[allow(clippy::too_many_arguments)]
pub fn backdrop<E: RenderElement<GlesRenderer>>(
    renderer: &mut GlesRenderer,
    cache: &mut BlurCache,
    role: WindowRole,
    regions: &[(Rectangle<i32, Physical>, f64)],
    alpha: f32,
    below: Vec<E>,
    output_size: Size<i32, Physical>,
    scale: Scale<f64>,
) -> Vec<BackdropElement> {
    let Some(programs) = programs(renderer) else { return Vec::new() };
    let rects: Vec<_> = regions.iter().map(|(r, _)| (r.loc.x, r.loc.y, r.size.w, r.size.h)).collect();
    let Some((x, y, w, h)) = backdrop::capture_rect(&rects, PADDING, (output_size.w, output_size.h)) else {
        return Vec::new();
    };
    let capture = Rectangle::new((x, y).into(), (w, h).into());
    let entry = cache.entry(role);
    let key: Key = (
        below.iter().map(|e| (e.id().clone(), e.current_commit(), e.geometry(scale), e.alpha().to_bits())).collect(),
        capture,
    );
    if entry.key.as_ref() != Some(&key) {
        match blur(renderer, &programs, &mut entry.textures, below, capture) {
            Ok(()) => entry.key = Some(key),
            Err(err) => {
                warn!("blur failed: {err}");
                entry.key = None;
                return Vec::new();
            }
        }
        entry.commit.increment();
    } else if entry.alpha != alpha {
        entry.commit.increment();
    }
    entry.alpha = alpha;
    entry.ids.resize_with(regions.len(), Id::new);
    let Some(texture) = entry.textures.first().cloned() else { return Vec::new() };
    let program = rounded::program(renderer);
    regions
        .iter()
        .zip(&entry.ids)
        .filter_map(|(&(region, radius), id)| {
            let region = region.intersection(capture)?;
            // The texture holds the capture at half size.
            let src = Rectangle::<f64, Buffer>::new(
                ((region.loc.x - capture.loc.x) as f64 / 2.0, (region.loc.y - capture.loc.y) as f64 / 2.0).into(),
                (region.size.w as f64 / 2.0, region.size.h as f64 / 2.0).into(),
            );
            Some(BackdropElement {
                id: id.clone(),
                commit: entry.commit,
                texture: texture.clone(),
                program: program.clone(),
                src,
                geometry: region,
                radius,
                alpha,
            })
        })
        .collect()
}

/// Renders `below` (minus `capture.loc`, at half size) into the first texture and blurs it there.
fn blur<E: RenderElement<GlesRenderer>>(
    renderer: &mut GlesRenderer,
    programs: &Programs,
    textures: &mut Vec<GlesTexture>,
    below: Vec<E>,
    capture: Rectangle<i32, Physical>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sizes = vec![half(capture.size)];
    for _ in 0..PASSES {
        sizes.push(half(sizes[sizes.len() - 1]));
    }
    let current: Vec<Size<i32, Physical>> = textures.iter().map(|t| (t.size().w, t.size().h).into()).collect();
    if current != sizes {
        textures.clear();
        for size in &sizes {
            let texture: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, (size.w, size.h).into())?;
            // Samples past the edge repeat the edge instead of wrapping around.
            let id = texture.tex_id();
            renderer.with_context(|gl| unsafe {
                gl.BindTexture(ffi::TEXTURE_2D, id);
                gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE as i32);
                gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE as i32);
                gl.BindTexture(ffi::TEXTURE_2D, 0);
            })?;
            textures.push(texture);
        }
    }

    let elements: Vec<_> = below
        .into_iter()
        .map(|element| {
            let element =
                RelocateRenderElement::from_element(element, Point::default() - capture.loc, Relocate::Relative);
            RescaleRenderElement::from_element(element, Point::default(), 0.5)
        })
        .collect();
    let mut damage_tracker = OutputDamageTracker::new(sizes[0], 1.0, Transform::Normal);
    {
        let mut framebuffer = renderer.bind(&mut textures[0])?;
        damage_tracker.render_output(renderer, &mut framebuffer, 0, &elements, crate::render::CLEAR_COLOR)?;
    }
    for level in 0..PASSES {
        let source = textures[level].clone();
        pass(renderer, &programs.down, &source, &mut textures[level + 1])?;
    }
    for level in (0..PASSES).rev() {
        let source = textures[level + 1].clone();
        pass(renderer, &programs.up, &source, &mut textures[level])?;
    }
    Ok(())
}

fn half(size: Size<i32, Physical>) -> Size<i32, Physical> {
    ((size.w + 1) / 2, (size.h + 1) / 2).into()
}

/// Draws all of `source` scaled to all of `target` with a blur shader.
fn pass(
    renderer: &mut GlesRenderer,
    program: &GlesTexProgram,
    source: &GlesTexture,
    target: &mut GlesTexture,
) -> Result<(), GlesError> {
    let size: Size<i32, Physical> = (target.size().w, target.size().h).into();
    let area = Rectangle::from_size(size);
    // Half a texel of the target, in source texture coordinates (Bjørge's `halfpixel`)
    let half_pixel = (OFFSET * 0.5 / size.w as f32, OFFSET * 0.5 / size.h as f32);
    let mut framebuffer = renderer.bind(target)?;
    let mut frame = renderer.render(&mut framebuffer, size, Transform::Normal)?;
    frame.render_texture_from_to(
        source,
        Rectangle::from_size(source.size()).to_f64(),
        area,
        &[area],
        &[],
        Transform::Normal,
        1.0,
        Some(program),
        &[Uniform::new("half_pixel", half_pixel)],
    )?;
    // The next pass and the frame run on the same GL context, in order: no fence to wait for.
    let _ = frame.finish()?;
    Ok(())
}

/// One blurred region under a window: part of the blur texture, cut to the region with rounded
/// corners.
#[derive(Debug)]
pub struct BackdropElement {
    id: Id,
    commit: CommitCounter,
    texture: GlesTexture,
    program: Option<GlesTexProgram>,
    src: Rectangle<f64, Buffer>,
    geometry: Rectangle<i32, Physical>,
    radius: f64,
    alpha: f32,
}

impl Element for BackdropElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.src
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    // Rounded corners and the fade show what is below.
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for BackdropElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let uniforms = rounded::clip_uniforms(&self.texture, src, dst, dst.to_f64(), self.radius);
        let (program, uniforms) = match (&self.program, uniforms) {
            (Some(program), Some(uniforms)) if self.radius >= 0.5 => (Some(program), uniforms),
            _ => (None, Vec::new()),
        };
        frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            self.alpha,
            program,
            &uniforms,
        )
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
