//! Generates short UI sounds as PCM (s16, stereo, 48 kHz), without any sound files.

pub const SAMPLE_RATE: u32 = 48_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sound {
    /// Focus moves on
    Move,
    /// Confirm, launch
    Confirm,
    /// Back, close
    Back,
    /// Control center or a new page opens
    Open,
}

/// A tone: start and end frequency (glissando), duration and volume.
struct Tone {
    from_hz: f32,
    to_hz: f32,
    ms: u32,
    gain: f32,
}

fn tones(sound: Sound) -> &'static [Tone] {
    match sound {
        Sound::Move => &[Tone { from_hz: 1320.0, to_hz: 1250.0, ms: 28, gain: 0.08 }],
        Sound::Confirm => &[
            Tone { from_hz: 880.0, to_hz: 880.0, ms: 45, gain: 0.12 },
            Tone { from_hz: 1320.0, to_hz: 1320.0, ms: 80, gain: 0.12 },
        ],
        Sound::Back => &[Tone { from_hz: 740.0, to_hz: 520.0, ms: 70, gain: 0.10 }],
        Sound::Open => &[
            Tone { from_hz: 660.0, to_hz: 660.0, ms: 40, gain: 0.10 },
            Tone { from_hz: 990.0, to_hz: 990.0, ms: 40, gain: 0.10 },
            Tone { from_hz: 1320.0, to_hz: 1320.0, ms: 90, gain: 0.10 },
        ],
    }
}

/// PCM data as bytes (little-endian, alternating left/right), ready for `pw-cat --raw`.
pub fn render(sound: Sound) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut phase = 0.0f32;
    for tone in tones(sound) {
        let samples = (SAMPLE_RATE * tone.ms / 1000) as usize;
        // A short fade in and out prevents clicks.
        let fade = (SAMPLE_RATE as usize / 1000 * 4).min(samples / 2).max(1);
        for i in 0..samples {
            let t = i as f32 / samples as f32;
            let hz = tone.from_hz + (tone.to_hz - tone.from_hz) * t;
            phase = (phase + hz / SAMPLE_RATE as f32).fract();
            let envelope = (i.min(samples - 1 - i) as f32 / fade as f32).min(1.0) * (1.0 - t * 0.5);
            // Sine plus a little overtone sounds softer than a pure beep.
            let wave = (phase * std::f32::consts::TAU).sin() * 0.85 + (phase * 2.0 * std::f32::consts::TAU).sin() * 0.15;
            let sample = (wave * envelope * tone.gain * i16::MAX as f32) as i16;
            let le = sample.to_le_bytes();
            bytes.extend_from_slice(&le);
            bytes.extend_from_slice(&le);
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_stereo_pcm_of_expected_length() {
        let pcm = render(Sound::Back);
        assert_eq!(pcm.len(), (SAMPLE_RATE * 70 / 1000) as usize * 4);
        let confirm = render(Sound::Confirm);
        assert_eq!(confirm.len(), (SAMPLE_RATE * 125 / 1000) as usize * 4);
    }

    #[test]
    fn starts_and_ends_silent_and_stays_quiet() {
        for sound in [Sound::Move, Sound::Confirm, Sound::Back, Sound::Open] {
            let samples: Vec<i16> = render(sound).as_chunks::<2>().0.iter().map(|&b| i16::from_le_bytes(b)).collect();
            assert_eq!(samples[0], 0);
            assert!(samples.last().unwrap().abs() < 200, "{sound:?}");
            let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
            assert!(peak > 500 && peak < (i16::MAX as f32 * 0.15) as u16, "{sound:?}: {peak}");
        }
    }
}
