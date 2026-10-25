//! Rounded corners for the app image during a transition (Story 7.7): a surface element drawn with
//! a texture shader that cuts the corners of the transition's clip rectangle (antialiased).
//!
//! The approach (wrap each surface element and swap the default texture shader while it draws)
//! follows niri's `ClippedSurfaceRenderElement` (niri, GPL-3.0-or-later, Copyright (c) Ivan
//! Molodetskikh); the code here is our own.

use smithay::backend::renderer::Texture;
use smithay::backend::renderer::element::surface::{WaylandSurfaceRenderElement, WaylandSurfaceTexture};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Transform};

/// Smithay's default texture shader plus the rounded clip. `geo_scale`/`geo_offset` map the
/// texture coordinates to pixels in the clip rectangle (`geo_size`).
const SHADER: &str = r#"
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

uniform vec2 geo_scale;
uniform vec2 geo_offset;
uniform vec2 geo_size;
uniform float radius;

void main() {
    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

    // Signed distance to the rounded rectangle, one pixel of antialiasing at the edge
    vec2 pos = v_coords * geo_scale + geo_offset;
    vec2 half_size = geo_size * 0.5;
    vec2 d = abs(pos - half_size) - (half_size - vec2(radius));
    float dist = length(max(d, 0.0)) + min(max(d.x, d.y), 0.0) - radius;
    color *= clamp(0.5 - dist, 0.0, 1.0);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
"#;

/// The compiled shader, kept in the EGL context's user data (`None`: it failed to compile).
struct Program(Option<GlesTexProgram>);

/// The rounded-clip shader of this renderer, compiled on first use.
pub fn program(renderer: &mut GlesRenderer) -> Option<GlesTexProgram> {
    if let Some(Program(program)) = renderer.egl_context().user_data().get::<Program>() {
        return program.clone();
    }
    let uniforms = [
        UniformName::new("geo_scale", UniformType::_2f),
        UniformName::new("geo_offset", UniformType::_2f),
        UniformName::new("geo_size", UniformType::_2f),
        UniformName::new("radius", UniformType::_1f),
    ];
    let program = renderer
        .compile_custom_texture_shader(SHADER, &uniforms)
        .inspect_err(|err| tracing::warn!("rounded-corner shader: {err}"))
        .ok();
    renderer.egl_context().user_data().insert_if_missing(|| Program(program.clone()));
    program
}

/// A window surface cut to `clip` with corners of `radius` (both in physical output pixels).
#[derive(Debug)]
pub struct RoundedSurface {
    inner: WaylandSurfaceRenderElement<GlesRenderer>,
    program: Option<GlesTexProgram>,
    clip: Rectangle<f64, Physical>,
    radius: f64,
}

impl RoundedSurface {
    pub fn new(
        inner: WaylandSurfaceRenderElement<GlesRenderer>,
        program: Option<GlesTexProgram>,
        clip: Rectangle<f64, Physical>,
        radius: f64,
    ) -> Self {
        Self { inner, program, clip, radius }
    }

    fn rounded(&self) -> bool {
        self.program.is_some() && self.radius >= 0.5
    }

    /// Uniforms for drawing `src` of the texture to `dst`; `None` where the shader does not apply.
    fn uniforms(&self, src: Rectangle<f64, Buffer>, dst: Rectangle<i32, Physical>) -> Option<Vec<Uniform<'static>>> {
        let WaylandSurfaceTexture::Texture(texture) = self.inner.texture() else { return None };
        if self.inner.transform() != Transform::Normal {
            return None;
        }
        clip_uniforms(texture, src, dst, self.clip, self.radius)
    }
}

/// Uniforms of the rounded-clip shader for drawing `src` of `texture` (untransformed) to `dst`,
/// cut to `clip` with corners of `radius`; `None` for an empty `src`.
pub fn clip_uniforms(
    texture: &GlesTexture,
    src: Rectangle<f64, Buffer>,
    dst: Rectangle<i32, Physical>,
    clip: Rectangle<f64, Physical>,
    radius: f64,
) -> Option<Vec<Uniform<'static>>> {
    if src.size.w <= 0.0 || src.size.h <= 0.0 {
        return None;
    }
    // Texture coordinates are `(src.loc + p * src.size / dst.size) / texture` for a pixel `p`
    // of `dst` (y negated for y-inverted textures); this is the inverse, relative to the clip.
    let size = texture.size();
    let (sx, sy) = (dst.size.w as f64 / src.size.w, dst.size.h as f64 / src.size.h);
    let flip = if texture.is_y_inverted() { -1.0 } else { 1.0 };
    let scale = ((size.w as f64 * sx) as f32, (flip * size.h as f64 * sy) as f32);
    let offset = (
        (dst.loc.x as f64 - clip.loc.x - src.loc.x * sx) as f32,
        (dst.loc.y as f64 - clip.loc.y - src.loc.y * sy) as f32,
    );
    Some(vec![
        Uniform::new("geo_scale", scale),
        Uniform::new("geo_offset", offset),
        Uniform::new("geo_size", (clip.size.w as f32, clip.size.h as f32)),
        Uniform::new("radius", radius as f32),
    ])
}

impl Element for RoundedSurface {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    // The cut corners show what is below, so nothing is opaque any more.
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        if self.rounded() { OpaqueRegions::default() } else { self.inner.opaque_regions(scale) }
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl RenderElement<GlesRenderer> for RoundedSurface {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let (Some(program), Some(uniforms)) =
            (self.program.clone().filter(|_| self.rounded()), self.uniforms(src, dst))
        else {
            return self.inner.draw(frame, src, dst, damage, opaque_regions, cache);
        };
        frame.override_default_tex_program(program, uniforms);
        let result = self.inner.draw(frame, src, dst, damage, &[], cache);
        frame.clear_tex_program_override();
        result
    }

    // Never scanned out directly: the corners need the shader.
    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
