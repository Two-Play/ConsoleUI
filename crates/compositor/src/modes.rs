//! Smithay-free choice of the output mode (Story 5.5).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeInfo {
    pub size: (u32, u32),
    pub refresh: u32,
    pub preferred: bool,
    pub interlaced: bool,
}

/// The mode for the wanted size: the preferred one if it has that size, else the fastest of that
/// size. Without a wish, or for a size this monitor does not have, the preferred mode (else the
/// first).
pub fn pick(modes: &[ModeInfo], wanted: Option<(u32, u32)>) -> Option<usize> {
    let preferred = modes.iter().position(|mode| mode.preferred).or(if modes.is_empty() { None } else { Some(0) });
    let Some(wanted) = wanted else { return preferred };
    preferred
        .filter(|&index| modes[index].size == wanted)
        .or_else(|| {
            let fitting = modes.iter().enumerate().filter(|(_, mode)| mode.size == wanted && !mode.interlaced);
            fitting.max_by_key(|(_, mode)| mode.refresh).map(|(index, _)| index)
        })
        .or(preferred)
}

/// The different sizes, largest first; interlaced modes are left out.
pub fn sizes(modes: &[ModeInfo]) -> Vec<(u32, u32)> {
    let mut sizes: Vec<_> = modes.iter().filter(|mode| !mode.interlaced).map(|mode| mode.size).collect();
    sizes.sort_by_key(|&(width, height)| std::cmp::Reverse((width * height, width)));
    sizes.dedup();
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: u32, height: u32, refresh: u32) -> ModeInfo {
        ModeInfo { size: (width, height), refresh, preferred: false, interlaced: false }
    }

    fn modes() -> Vec<ModeInfo> {
        vec![
            mode(1280, 720, 50),
            mode(1280, 720, 60),
            ModeInfo { preferred: true, ..mode(1920, 1080, 60) },
            mode(1920, 1080, 120),
            ModeInfo { interlaced: true, ..mode(1440, 480, 60) },
        ]
    }

    #[test]
    fn picks_the_preferred_mode_without_a_wish() {
        assert_eq!(pick(&modes(), None), Some(2));
        assert_eq!(pick(&[mode(800, 600, 60)], None), Some(0));
        assert_eq!(pick(&[], None), None);
    }

    #[test]
    fn picks_the_fastest_of_the_wanted_size_but_keeps_the_preferred() {
        assert_eq!(pick(&modes(), Some((1280, 720))), Some(1));
        assert_eq!(pick(&modes(), Some((1920, 1080))), Some(2));
    }

    #[test]
    fn unknown_or_interlaced_sizes_fall_back() {
        assert_eq!(pick(&modes(), Some((3840, 2160))), Some(2));
        assert_eq!(pick(&modes(), Some((1440, 480))), Some(2));
    }

    #[test]
    fn lists_each_progressive_size_once_largest_first() {
        assert_eq!(sizes(&modes()), vec![(1920, 1080), (1280, 720)]);
    }
}
