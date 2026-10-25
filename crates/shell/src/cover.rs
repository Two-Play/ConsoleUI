use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Creates a placeholder cover (gradient with a light spot and stripes),
/// until real artwork is loaded in Sprint 2. `seed` varies the pattern per tile.
pub fn generate(from: [u8; 3], to: [u8; 3], seed: u32, size: u32) -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(size, size);
    let (cx, cy) = (0.2 + 0.6 * unit(seed, 1), 0.15 + 0.5 * unit(seed, 2));
    let phase = unit(seed, 3);
    let inv = 1.0 / size as f32;

    for (i, px) in buffer.make_mut_slice().iter_mut().enumerate() {
        let x = (i as u32 % size) as f32 * inv;
        let y = (i as u32 / size) as f32 * inv;
        let t = (x * 0.4 + y * 0.6).clamp(0.0, 1.0);
        let dist = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        let light = (1.0 - dist / 0.55).max(0.0).powi(2) * 0.55;
        let stripe = if ((x - y + phase) * 7.0).rem_euclid(1.0) < 0.06 { 0.08 } else { 0.0 };
        let shade = 1.0 - y * 0.25;
        let channel = |a: u8, b: u8| {
            let base = (a as f32 + (b as f32 - a as f32) * t) * shade;
            (base + (255.0 - base) * (light + stripe)).clamp(0.0, 255.0) as u8
        };
        *px = Rgba8Pixel { r: channel(from[0], to[0]), g: channel(from[1], to[1]), b: channel(from[2], to[2]), a: 255 };
    }
    Image::from_rgba8(buffer)
}

/// Deterministic pseudo-random number in [0, 1).
fn unit(seed: u32, salt: u32) -> f32 {
    let mut h = seed.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h % 10_000) as f32 / 10_000.0
}
