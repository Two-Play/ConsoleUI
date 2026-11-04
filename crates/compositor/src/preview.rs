//! The small images of the window switcher, without Smithay types.

/// Previews are at most about this wide.
const WIDTH: usize = 480;

/// Reduces RGBA rows by a whole factor, averaging each block of pixels so text and thin lines stay
/// readable. Returns the pixels and their size.
pub fn shrink(rgba: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let factor = width.div_ceil(WIDTH).max(1);
    let (out_width, out_height) = ((width / factor).max(1), (height / factor).max(1));
    let mut out = Vec::with_capacity(out_width * out_height * 4);
    for y in 0..out_height {
        for x in 0..out_width {
            let mut sum = [0u32; 4];
            let mut count = 0;
            for source_y in y * factor..((y + 1) * factor).min(height) {
                for source_x in x * factor..((x + 1) * factor).min(width) {
                    let at = (source_y * width + source_x) * 4;
                    for (total, value) in sum.iter_mut().zip(&rgba[at..at + 4]) {
                        *total += u32::from(*value);
                    }
                    count += 1;
                }
            }
            out.extend(sum.iter().map(|total| (total / count.max(1)) as u8));
        }
    }
    (out, out_width, out_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_images_stay_as_they_are() {
        let pixels = vec![9; 4 * 4 * 4];
        assert_eq!(shrink(&pixels, 4, 4), (pixels, 4, 4));
    }

    #[test]
    fn full_hd_becomes_a_quarter() {
        let (pixels, width, height) = shrink(&vec![255; 1920 * 1080 * 4], 1920, 1080);
        assert_eq!((width, height), (480, 270));
        assert_eq!(pixels.len(), 480 * 270 * 4);
    }

    #[test]
    fn blocks_are_averaged() {
        // 960 wide gives a factor of 2: black and white pixels become gray.
        let row: Vec<u8> = (0..960).flat_map(|x| [if x % 2 == 0 { 0 } else { 200 }, 0, 0, 255]).collect();
        let pixels = [row.clone(), row].concat();
        let (out, width, height) = shrink(&pixels, 960, 2);
        assert_eq!((width, height), (480, 1));
        assert_eq!(&out[..4], [100, 0, 0, 255]);
    }

    #[test]
    fn odd_sizes_do_not_read_past_the_end() {
        let (_, width, height) = shrink(&vec![1; 1001 * 3 * 4], 1001, 3);
        assert_eq!((width, height), (333, 1));
    }
}
