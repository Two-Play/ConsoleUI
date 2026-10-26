//! The animated background ("shader" background): soft aurora bands in the accent color of the
//! focused entry, computed per pixel at a low resolution (the shell scales it up smoothly, which
//! suits the soft shapes). Shown instead of the hero artwork, or where an entry has none
//! (settings row "Background").

use serde::{Deserialize, Serialize};

use crate::i18n::tr;

/// Resolution of one frame; the shell stretches it to the window.
pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 135;

/// The page background (`Theme.base`) as linear-ish floats
const BASE: [f32; 3] = [0.024, 0.027, 0.047];

/// Look of the animated background (settings row "Animation style")
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// Soft bands like a northern light
    #[default]
    Aurora,
    /// Slowly flowing color clouds
    Plasma,
    /// Large soft circles drifting around
    Bubbles,
    /// Layered waves with glowing crests
    Waves,
    /// A perspective floor grid rushing towards a low sun
    Grid,
    /// Flying through a ringed tunnel
    Tunnel,
    /// Flying through a field of stars
    Stars,
    /// Glossy spheres floating in space
    Orbs,
}

impl Style {
    pub const ALL: [Style; 8] =
        [Self::Aurora, Self::Plasma, Self::Bubbles, Self::Waves, Self::Grid, Self::Tunnel, Self::Stars, Self::Orbs];

    pub fn label(self) -> &'static str {
        match self {
            Self::Aurora => tr("Aurora"),
            Self::Plasma => tr("Plasma"),
            Self::Bubbles => tr("Bubbles"),
            Self::Waves => tr("Waves"),
            Self::Grid => tr("Grid"),
            Self::Tunnel => tr("Tunnel"),
            Self::Stars => tr("Stars"),
            Self::Orbs => tr("Orbs"),
        }
    }
}

/// Colors of the animated background (settings row "Animation colors")
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Palette {
    /// The accent color of the focused entry
    #[default]
    Entry,
    /// All hues, slowly rotating
    Spectrum,
    Sunset,
    Ocean,
    Neon,
}

impl Palette {
    pub const ALL: [Palette; 5] = [Self::Entry, Self::Spectrum, Self::Sunset, Self::Ocean, Self::Neon];

    pub fn label(self) -> &'static str {
        match self {
            Self::Entry => tr("Entry color"),
            Self::Spectrum => tr("Spectrum"),
            Self::Sunset => tr("Sunset"),
            Self::Ocean => tr("Ocean"),
            Self::Neon => tr("Neon"),
        }
    }
}

/// The three colors a style paints with: a main one, a shifted one and a pale highlight.
#[derive(Debug, Clone, Copy)]
struct Colors {
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
}

impl Colors {
    fn new(palette: Palette, accent: [f32; 3], time: f32) -> Self {
        match palette {
            Palette::Entry => {
                Self { a: accent, b: mix(accent, [0.35, 0.45, 1.0], 0.55), c: mix(accent, [1.0, 1.0, 1.0], 0.6) }
            }
            Palette::Spectrum => {
                let hue = (time * 0.02).fract();
                Self { a: hsv(hue, 0.7), b: hsv(hue + 0.18, 0.7), c: mix(hsv(hue + 0.5, 0.5), [1.0; 3], 0.4) }
            }
            Palette::Sunset => Self { a: [1.0, 0.45, 0.25], b: [0.85, 0.2, 0.55], c: [1.0, 0.8, 0.5] },
            Palette::Ocean => Self { a: [0.15, 0.6, 0.9], b: [0.1, 0.85, 0.75], c: [0.7, 0.95, 1.0] },
            Palette::Neon => Self { a: [1.0, 0.2, 0.7], b: [0.3, 0.5, 1.0], c: [0.2, 1.0, 0.9] },
        }
    }
}

/// Renders the frame at `time` seconds into `out` (RGBA, `width * height * 4` bytes). `accent` is
/// the color of the focused entry (0–1 per channel), used by [`Palette::Entry`].
pub fn render(
    style: Style,
    palette: Palette,
    width: usize,
    height: usize,
    time: f32,
    accent: [f32; 3],
    out: &mut [u8],
) {
    assert_eq!(out.len(), width * height * 4);
    let aspect = width as f32 / height as f32;
    let colors = Colors::new(palette, accent, time);
    for y in 0..height {
        let v = y as f32 / (height - 1) as f32;
        for x in 0..width {
            let u = x as f32 / (width - 1) as f32 * aspect;
            let mut color = match style {
                Style::Aurora => aurora(u, v, time, colors),
                Style::Plasma => plasma(u, v, time, colors),
                Style::Bubbles => bubbles(u, v, aspect, time, colors),
                Style::Waves => waves(u, v, time, colors),
                Style::Grid => grid(u, v, aspect, time, colors),
                Style::Tunnel => tunnel(u, v, aspect, time, colors),
                Style::Stars => stars(u, v, aspect, time, colors),
                Style::Orbs => orbs(u, v, aspect, time, colors),
            };
            // Vignette
            let dx = u / aspect - 0.5;
            let dy = v - 0.45;
            let vignette = 1.0 - (dx * dx * 1.1 + dy * dy * 1.6).min(1.0) * 0.75;
            color = scale(color, vignette);
            // Dither against banding once the low resolution is scaled up
            let noise = (hash(x as u32, y as u32, (time * 30.0) as u32) - 0.5) * 2.5 / 255.0;
            let i = (y * width + x) * 4;
            for (channel, value) in color.iter().enumerate() {
                out[i + channel] = ((value + noise).clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            out[i + 3] = 255;
        }
    }
}

/// Dark background with a little of the main color towards the top
fn sky(v: f32, colors: Colors, strength: f32) -> [f32; 3] {
    mix(BASE, scale(colors.a, 0.22), (1.0 - v).powi(2) * strength)
}

fn aurora(u: f32, v: f32, time: f32, colors: Colors) -> [f32; 3] {
    // Three bands: the main color, the shifted one, and a pale highlight
    let band_colors = [colors.a, colors.b, colors.c];
    let bands: [(f32, f32, f32, f32); 3] = [
        // (height, thickness, speed, strength)
        (0.42, 0.010, 0.22, 0.55),
        (0.55, 0.018, 0.16, 0.40),
        (0.66, 0.006, 0.29, 0.25),
    ];
    let mut color = sky(v, colors, 0.8);
    for (k, (&(center, thickness, speed, strength), band_color)) in bands.iter().zip(band_colors).enumerate() {
        let k = k as f32;
        let wave = center
            + 0.07 * (u * (2.2 + 0.7 * k) + time * speed + k * 1.7).sin()
            + 0.03 * (u * (4.7 + 1.3 * k) - time * speed * 1.6 + k).sin()
            + 0.012 * (u * 9.0 + time * speed * 2.3 + k * 3.0).sin();
        let distance = v - wave;
        // Curtain: a sharp lower edge with a long glow upwards, like an aurora
        let falloff = if distance > 0.0 { thickness } else { thickness * 9.0 };
        let shape = (-(distance * distance) / falloff).exp();
        let shimmer = 0.55 + 0.45 * (u * 3.4 - time * 0.35 * (1.0 + k * 0.4) + k * 2.0).sin();
        color = add(color, scale(band_color, shape * shimmer * strength));
    }
    color
}

/// Sum of a few sines, the classic plasma, mapped to dark tones of the colors
fn plasma(u: f32, v: f32, time: f32, colors: Colors) -> [f32; 3] {
    let (x, y) = (u * 3.0, v * 3.0);
    let sum = (x * 1.3 + time * 0.20).sin()
        + (y * 1.7 - time * 0.15).sin()
        + ((x + y) * 1.1 + time * 0.10).sin()
        + ((x - 2.4).hypot(y - 1.4) * 2.2 - time * 0.25).sin();
    let t = sum * 0.125 + 0.5;
    let base = mix(BASE, scale(colors.a, 0.55), t * t);
    add(base, scale(colors.b, (t - 0.5).max(0.0) * 1.1))
}

/// Soft circles that drift slowly, overlapping into each other
fn bubbles(u: f32, v: f32, aspect: f32, time: f32, colors: Colors) -> [f32; 3] {
    let mut color = sky(v, colors, 0.6);
    for k in 0..14 {
        let k = k as f32;
        let cx = aspect * (0.5 + 0.48 * (k * 1.7 + time * 0.035 * (1.0 + (k * 0.37).fract())).sin());
        let cy = 0.5 + 0.55 * (k * 2.9 + time * 0.025 * (1.0 + (k * 0.53).fract())).sin();
        let radius = 0.07 + 0.12 * (k * 0.37).fract();
        let closeness = (1.0 - (u - cx).hypot(v - cy) / radius).clamp(0.0, 1.0);
        let strength = closeness * closeness * (0.26 + 0.30 * (k * 0.61).fract());
        let tint = mix(colors.a, mix(colors.b, colors.c, 0.4), (k * 0.43).fract());
        color = add(color, scale(tint, strength));
    }
    color
}

/// A few layers of waves; each fills below its crest and glows along it. Everything is smooth
/// across the crest, since the frame is small and scaled up.
fn waves(u: f32, v: f32, time: f32, colors: Colors) -> [f32; 3] {
    let mut color = sky(v, colors, 0.5);
    for k in 0..4 {
        let k = k as f32;
        let crest = 0.42
            + 0.12 * k
            + 0.045 * (u * (2.0 + k) + time * (0.20 + 0.05 * k) + k * 1.3).sin()
            + 0.015 * (u * 7.0 - time * 0.4 + k * 2.1).sin();
        let tint = mix(colors.a, colors.b, k / 3.0);
        let depth = v - crest;
        // Fades in over the crest and then dies out below it
        let fill = smoothstep(-0.03, 0.05, depth) * (-depth.max(0.0) * 7.0).exp();
        color = add(color, scale(tint, (0.16 - 0.025 * k) * fill));
        color = add(color, scale(mix(tint, colors.c, 0.5), 0.2 * (-(depth * depth) / 0.0018).exp()));
    }
    color
}

/// A perspective floor with lines coming towards the viewer, a low sun above the horizon
fn grid(u: f32, v: f32, aspect: f32, time: f32, colors: Colors) -> [f32; 3] {
    let horizon = 0.5;
    let mut color = sky(v.min(horizon) * 1.2, colors, 0.5);
    let x = u - aspect / 2.0;
    if v < horizon {
        // The sun: a disc with a gradient and horizontal gaps that grow towards the bottom
        let (dx, dy) = (x, v - (horizon - 0.12));
        let inside = smoothstep(0.2, 0.17, dx.hypot(dy));
        let band = smoothstep(0.0, 0.02, ((v * 60.0).sin() * 0.5 + 0.5) - (v - 0.3).max(0.0) * 3.0);
        color = add(color, scale(mix(colors.a, colors.b, (v - 0.3) * 5.0), inside * band * 0.75));
        return color;
    }
    // Floor: depth grows towards the horizon
    let d = (v - horizon).max(0.004);
    let z = 0.12 / d;
    let (gx, gz) = (x * z * 2.0, z + time * 1.2);
    let line = |f: f32, width: f32| smoothstep(width, 0.0, (f - f.round()).abs());
    // Thinner towards the horizon, so the lines do not shimmer
    let width = 0.05 + d * 0.5;
    let lines = line(gx, width).max(line(gz, width));
    let fade = smoothstep(0.0, 0.12, d);
    color = add(color, scale(mix(colors.a, colors.b, d * 2.0), lines * fade * 0.55));
    // Glow on the horizon
    add(color, scale(colors.c, (-d * 40.0).exp() * 0.25))
}

/// Rings and spokes receding into the distance
fn tunnel(u: f32, v: f32, aspect: f32, time: f32, colors: Colors) -> [f32; 3] {
    let (x, y) = (u - aspect / 2.0, v - 0.5);
    let r = x.hypot(y).max(0.001);
    let angle = y.atan2(x);
    let depth = 0.18 / r + time * 0.6;
    let rings = (depth * 6.0).sin() * 0.5 + 0.5;
    let spokes = (angle * 6.0 + depth * 1.5).sin() * 0.5 + 0.5;
    let fade = smoothstep(0.0, 0.25, r);
    let pattern = rings.powi(3) * 0.7 + spokes.powi(6) * 0.5;
    let tint = mix(colors.a, colors.b, (depth * 0.15).sin() * 0.5 + 0.5);
    let color = add(BASE, scale(tint, pattern * fade * 0.55));
    add(color, scale(colors.c, (-r * 9.0).exp() * 0.25))
}

/// Stars at different depths coming towards the viewer
fn stars(u: f32, v: f32, aspect: f32, time: f32, colors: Colors) -> [f32; 3] {
    let mut color = sky(v, colors, 0.4);
    let (x, y) = (u - aspect / 2.0, v - 0.5);
    for k in 0..70 {
        let k = k as f32;
        // Position on the "sky" and the start depth of this star
        let sx = ((k * 12.9898).sin() * 43758.547).fract() - 0.5;
        let sy = ((k * 78.233).sin() * 12543.321).fract() - 0.5;
        let depth = 1.0 - (((k * 0.618) + time * 0.12).fract());
        let (px, py) = (sx * 2.6 * aspect / depth.max(0.05) * 0.35, sy * 2.6 / depth.max(0.05) * 0.35);
        let near = (1.0 - depth).clamp(0.0, 1.0);
        let size = 0.004 + 0.012 * near;
        let glow = (-((x - px).powi(2) + (y - py).powi(2)) / (size * size)).exp();
        let tint = mix(colors.c, colors.b, (k * 0.37).fract());
        color = add(color, scale(tint, glow * (0.2 + 0.8 * near * near)));
    }
    color
}

/// Glossy spheres, lit from the upper left: analytic ray-sphere hits, diffuse light, a specular
/// highlight and a rim
fn orbs(u: f32, v: f32, aspect: f32, time: f32, colors: Colors) -> [f32; 3] {
    let mut color = sky(v, colors, 0.5);
    let (x, y) = (u - aspect / 2.0, v - 0.5);
    let light = [-0.45, -0.6, 0.65];
    // Back to front, so nearer spheres cover farther ones
    let mut spheres: Vec<(f32, f32, f32, f32, f32)> = (0..5)
        .map(|k| {
            let k = k as f32;
            let cx = 0.62 * aspect * (k * 1.9 + time * 0.07 * (1.0 + k * 0.15)).sin() * 0.8;
            let cy = 0.30 * (k * 2.7 + time * 0.05 * (1.2 + k * 0.1)).sin();
            let z = 0.4 + (k * 0.77).fract();
            (z, cx, cy, 0.10 + 0.07 * (k * 0.53).fract(), k)
        })
        .collect();
    spheres.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (z, cx, cy, radius, k) in spheres {
        let (px, py) = ((x - cx) / (radius / z * 0.0 + radius), (y - cy) / radius);
        let q = px * px + py * py;
        if q >= 1.0 {
            // A soft halo around the sphere
            let halo = (-(q - 1.0) * 6.0).exp() * 0.06;
            color = add(color, scale(mix(colors.a, colors.b, (k * 0.3).fract()), halo));
            continue;
        }
        let nz = (1.0 - q).sqrt();
        let diffuse = (px * light[0] + py * light[1] + nz * light[2]).max(0.0);
        let specular = diffuse.powi(24) * 0.8;
        let rim = (1.0 - nz).powi(3) * 0.35;
        let base = mix(colors.a, colors.b, (k * 0.3).fract());
        let lit = add(scale(base, 0.08 + 0.55 * diffuse), scale(colors.c, specular + rim));
        // Edge antialiasing: blends into what is behind over the last bit of the radius
        let cover = smoothstep(1.0, 0.94, q.sqrt());
        color = mix(color, lit, cover);
    }
    color
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A bright color of hue `h` (0–1, wraps around) with saturation `s`
fn hsv(h: f32, s: f32) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let channel = |offset: f32| {
        let k = (offset + h).rem_euclid(6.0);
        1.0 - s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    [channel(5.0), channel(3.0), channel(1.0)]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn scale(a: [f32; 3], f: f32) -> [f32; 3] {
    [a[0] * f, a[1] * f, a[2] * f]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// 0…1 pseudo-random per pixel and frame
fn hash(x: u32, y: u32, frame: u32) -> f32 {
    let mut h = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263) ^ frame.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    (h ^ (h >> 16)) as f32 / u32::MAX as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(style: Style, palette: Palette, time: f32, accent: [f32; 3]) -> Vec<u8> {
        let mut out = vec![0; WIDTH * HEIGHT * 4];
        render(style, palette, WIDTH, HEIGHT, time, accent, &mut out);
        out
    }

    fn average(frame: &[u8]) -> [f32; 3] {
        let mut sum = [0f32; 3];
        for pixel in frame.chunks(4) {
            for c in 0..3 {
                sum[c] += pixel[c] as f32;
            }
        }
        sum.map(|s| s / (frame.len() / 4) as f32)
    }

    #[test]
    fn every_style_is_dark_opaque_and_tinted_by_the_accent() {
        for style in Style::ALL {
            let red = frame(style, Palette::Entry, 3.0, [1.0, 0.2, 0.2]);
            assert!(red.chunks(4).all(|p| p[3] == 255));
            let [r, g, b] = average(&red);
            // A background, not a light source: stays dark on average
            assert!(r < 90.0 && g < 90.0 && b < 90.0, "{style:?}: {r} {g} {b}");
            assert!(r > g, "{style:?}: red accent tints the frame: {r} {g}");
            let [r, _, b] = average(&frame(style, Palette::Entry, 3.0, [0.2, 0.3, 1.0]));
            assert!(b > r, "{style:?}");
        }
    }

    #[test]
    fn every_style_and_palette_stays_dark_and_moves() {
        for style in Style::ALL {
            for palette in Palette::ALL {
                let a = frame(style, palette, 1.0, [0.5, 0.5, 1.0]);
                let b = frame(style, palette, 3.0, [0.5, 0.5, 1.0]);
                let [r, g, bl] = average(&a);
                assert!(r < 100.0 && g < 100.0 && bl < 100.0, "{style:?}/{palette:?}: {r} {g} {bl}");
                let changed = a.iter().zip(&b).filter(|(x, y)| x.abs_diff(**y) > 4).count();
                assert!(changed > 100, "{style:?}/{palette:?}: {changed}");
                assert_eq!(a, frame(style, palette, 1.0, [0.5, 0.5, 1.0]), "deterministic");
            }
        }
    }

    #[test]
    fn styles_and_palettes_differ() {
        let frames: Vec<_> = Style::ALL.iter().map(|&s| frame(s, Palette::Entry, 5.0, [0.5, 0.5, 1.0])).collect();
        for i in 0..frames.len() {
            for j in i + 1..frames.len() {
                assert_ne!(frames[i], frames[j], "{:?} {:?}", Style::ALL[i], Style::ALL[j]);
            }
        }
        let other = frame(Style::Aurora, Palette::Sunset, 5.0, [0.5, 0.5, 1.0]);
        assert_ne!(other, frame(Style::Aurora, Palette::Ocean, 5.0, [0.5, 0.5, 1.0]));
        // Sunset is warm whatever the entry color is
        let [r, _, b] = average(&frame(Style::Plasma, Palette::Sunset, 3.0, [0.2, 0.3, 1.0]));
        assert!(r > b);
    }

    #[test]
    fn hsv_covers_the_hues() {
        assert_eq!(hsv(0.0, 1.0), [1.0, 0.0, 0.0]);
        let green = hsv(1.0 / 3.0, 1.0);
        assert!(green[1] > 0.99 && green[0] < 0.01 && green[2] < 0.01, "{green:?}");
        assert_eq!(hsv(1.0, 1.0), hsv(0.0, 1.0));
    }
}
