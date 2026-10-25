//! Akzentfarben aus einem Cover-Bild ableiten.

/// Gewichteter Mittelwert, bei dem kräftige, helle Pixel mehr zählen, danach auf eine
/// gut lesbare Helligkeit gebracht. `rgba` sind RGBA8-Pixel hintereinander.
pub fn from_rgba(rgba: &[u8]) -> Option<([u8; 3], [u8; 3])> {
    const MAX_SAMPLES: usize = 4096;
    let pixels = rgba.len() / 4;
    if pixels == 0 {
        return None;
    }
    let (mut r, mut g, mut b, mut total) = (0.0, 0.0, 0.0, 0.0);
    for px in rgba.as_chunks::<4>().0.iter().step_by((pixels / MAX_SAMPLES).max(1)) {
        let [pr, pg, pb, pa] = px.map(|c| c as f32 / 255.0);
        let max = pr.max(pg).max(pb);
        let min = pr.min(pg).min(pb);
        let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
        let weight = pa * max * (0.05 + saturation * saturation);
        r += pr * weight;
        g += pg * weight;
        b += pb * weight;
        total += weight;
    }
    if total <= 0.0 {
        return None;
    }
    let (h, s, v) = rgb_to_hsv(r / total, g / total, b / total);
    // Graue Bilder nicht künstlich einfärben.
    let s = if s < 0.08 { s } else { s.clamp(0.45, 0.9) };
    let primary = hsv_to_rgb(h, s, v.clamp(0.45, 0.75));
    let secondary = hsv_to_rgb((h + 0.08).fract(), s * 0.7, (v + 0.25).min(0.95));
    Some((to_u8(primary), to_u8(secondary)))
}

fn to_u8((r, g, b): (f32, f32, f32)) -> [u8; 3] {
    [r, g, b].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    let s = if max == 0.0 { 0.0 } else { delta / max };
    (h, s, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h6 % 2.0 - 1.0).abs());
    let (r, g, b) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (r + m, g + m, b + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(pixel: [u8; 4], count: usize) -> Vec<u8> {
        pixel.repeat(count)
    }

    #[test]
    fn empty_image_has_no_accent() {
        assert_eq!(from_rgba(&[]), None);
        assert_eq!(from_rgba(&image([0, 0, 0, 255], 10)), None);
    }

    #[test]
    fn red_image_gives_red_accent() {
        let ([r, g, b], _) = from_rgba(&image([220, 30, 30, 255], 100)).unwrap();
        assert!(r > 150 && g < 80 && b < 80, "{r} {g} {b}");
    }

    #[test]
    fn vivid_pixels_outweigh_gray_ones() {
        let mut pixels = image([128, 128, 128, 255], 900);
        pixels.extend(image([20, 60, 230, 255], 100));
        let ([r, _, b], _) = from_rgba(&pixels).unwrap();
        assert!(b > r + 60, "{r} {b}");
    }

    #[test]
    fn gray_image_stays_gray() {
        let ([r, g, b], _) = from_rgba(&image([150, 150, 150, 255], 100)).unwrap();
        assert!(r == g && g == b);
    }

    #[test]
    fn hsv_roundtrip() {
        let (h, s, v) = rgb_to_hsv(0.2, 0.6, 0.4);
        let (r, g, b) = hsv_to_rgb(h, s, v);
        assert!((r - 0.2).abs() < 1e-5 && (g - 0.6).abs() < 1e-5 && (b - 0.4).abs() < 1e-5);
    }
}
