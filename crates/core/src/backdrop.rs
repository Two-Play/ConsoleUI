//! The animated background ("shader" background): soft aurora bands in the accent color of the
//! focused entry, computed per pixel at a low resolution (the shell scales it up smoothly, which
//! suits the soft shapes). Shown instead of the hero artwork, or where an entry has none
//! (settings row "Background").

/// Resolution of one frame; the shell stretches it to the window.
pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 135;

/// The page background (`Theme.base`) as linear-ish floats
const BASE: [f32; 3] = [0.024, 0.027, 0.047];

/// Renders the frame at `time` seconds into `out` (RGBA, `width * height * 4` bytes). `accent` is
/// the color of the focused entry (0–1 per channel).
pub fn render(width: usize, height: usize, time: f32, accent: [f32; 3], out: &mut [u8]) {
    assert_eq!(out.len(), width * height * 4);
    let aspect = width as f32 / height as f32;
    // Three bands: the accent, a cooler shift of it, and a pale highlight
    let colors = [accent, mix(accent, [0.35, 0.45, 1.0], 0.55), mix(accent, [1.0, 1.0, 1.0], 0.6)];
    let bands: [(f32, f32, f32, f32); 3] = [
        // (height, thickness, speed, strength)
        (0.42, 0.010, 0.22, 0.55),
        (0.55, 0.018, 0.16, 0.40),
        (0.66, 0.006, 0.29, 0.25),
    ];
    for y in 0..height {
        let v = y as f32 / (height - 1) as f32;
        for x in 0..width {
            let u = x as f32 / (width - 1) as f32 * aspect;
            // Background: dark, a little of the accent towards the top
            let mut color = mix(BASE, scale(accent, 0.22), (1.0 - v).powi(2) * 0.8);
            for (k, (&(center, thickness, speed, strength), band_color)) in bands.iter().zip(colors).enumerate() {
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

    fn frame(time: f32, accent: [f32; 3]) -> Vec<u8> {
        let mut out = vec![0; WIDTH * HEIGHT * 4];
        render(WIDTH, HEIGHT, time, accent, &mut out);
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
    fn is_dark_opaque_and_tinted_by_the_accent() {
        let red = frame(3.0, [1.0, 0.2, 0.2]);
        assert!(red.chunks(4).all(|p| p[3] == 255));
        let [r, g, b] = average(&red);
        // A background, not a light source: stays dark on average
        assert!(r < 90.0 && g < 90.0 && b < 90.0, "{r} {g} {b}");
        assert!(r > g, "red accent tints the frame: {r} {g}");
        let [r, _, b] = average(&frame(3.0, [0.2, 0.3, 1.0]));
        assert!(b > r);
    }

    #[test]
    fn moves_over_time() {
        let a = frame(0.0, [0.5, 0.5, 1.0]);
        let b = frame(2.0, [0.5, 0.5, 1.0]);
        let changed = a.iter().zip(&b).filter(|(x, y)| x.abs_diff(**y) > 4).count();
        assert!(changed > a.len() / 10, "{changed}");
        assert_eq!(a, frame(0.0, [0.5, 0.5, 1.0]), "deterministic");
    }
}
