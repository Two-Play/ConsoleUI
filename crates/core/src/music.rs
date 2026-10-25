//! Background music for the home screen, like the menu music of a console: a generated ambient
//! loop (no sound files), or the user's own files from a folder.
//!
//! The loop is a slow pad over four chords with sparse bell notes and an echo. Everything wraps
//! around the loop length (chord crossfades, bell decays and the echo), so it repeats without a
//! seam. PCM format as in [`crate::sound`]: s16, stereo, 48 kHz.

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};

use crate::sound::SAMPLE_RATE;

/// Seconds per chord of the built-in loop (four chords)
pub const CHORD_SECONDS: f32 = 12.0;

/// The chords as MIDI notes: Fmaj7, Em7, Dm9, Cmaj7.
const CHORDS: [[u8; 4]; 4] = [[53, 57, 60, 64], [52, 55, 59, 62], [50, 53, 57, 64], [48, 52, 55, 59]];

fn midi_hz(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
}

/// Small deterministic random generator (the loop sounds the same on every start).
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Renders the ambient loop with `chord_seconds` per chord as interleaved stereo samples.
pub fn ambient_loop(chord_seconds: f32) -> Vec<i16> {
    let rate = SAMPLE_RATE as f32;
    let chord_frames = (chord_seconds * rate) as usize;
    let frames = chord_frames * CHORDS.len();
    let mut left = vec![0f32; frames];
    let mut right = vec![0f32; frames];

    // Pad: each chord fades in and out with a raised cosine over 1.5 chords, so neighbours
    // overlap; two slightly detuned sines per note plus a soft octave.
    let span = chord_frames + chord_frames / 2;
    for (index, chord) in CHORDS.iter().enumerate() {
        let start = index * chord_frames + frames - chord_frames / 4;
        let voices: Vec<(f32, f32, f32)> = chord
            .iter()
            .enumerate()
            .map(|(i, &note)| (midi_hz(note), 0.07, i as f32 / 3.0))
            .chain(std::iter::once((midi_hz(chord[0] - 12), 0.09, 0.5)))
            .collect();
        for (hz, gain, pan) in voices {
            let mut phases = [0f32; 3];
            let steps = [hz * 0.9985 / rate, hz * 1.0015 / rate, hz * 2.0 / rate];
            for i in 0..span {
                let t = i as f32 / span as f32;
                let envelope = 0.5 - 0.5 * (t * TAU).cos();
                // Slow "breathing" of the octave, like a filter opening and closing
                let shimmer = 0.15 + 0.1 * (t * TAU * 2.0).sin();
                let mut wave = 0.0;
                for (phase, step) in phases.iter_mut().zip(steps) {
                    *phase = (*phase + step).fract();
                }
                wave += (phases[0] * TAU).sin() + (phases[1] * TAU).sin();
                wave += (phases[2] * TAU).sin() * shimmer;
                let sample = wave * envelope * gain;
                let at = (start + i) % frames;
                left[at] += sample * (1.0 - pan * 0.6);
                right[at] += sample * (0.4 + pan * 0.6);
            }
        }
    }

    // Bells: now and then a chord tone two octaves up, decaying over a few seconds.
    let mut random = Lcg(0x5eed);
    let step = (rate * 0.75) as usize;
    let decay_frames = (rate * 4.0) as usize;
    for slot in 0..frames / step {
        if random.next() > 0.32 {
            continue;
        }
        let chord = CHORDS[(slot * step / chord_frames) % CHORDS.len()];
        let note = chord[(random.next() * 4.0) as usize % 4] + 24;
        let hz = midi_hz(note);
        let pan = random.next();
        let gain = 0.035 + 0.025 * random.next();
        let (mut phase, mut overtone) = (0f32, 0f32);
        for i in 0..decay_frames {
            let t = i as f32 / rate;
            let attack = (t / 0.004).min(1.0);
            phase = (phase + hz / rate).fract();
            overtone = (overtone + hz * 2.76 / rate).fract();
            let wave = (phase * TAU).sin() * (-t / 1.1).exp() + (overtone * TAU).sin() * 0.3 * (-t / 0.25).exp();
            let sample = wave * attack * gain;
            let at = (slot * step + i) % frames;
            left[at] += sample * (1.0 - pan);
            right[at] += sample * pan;
        }
    }

    // Ping-pong echo, run twice around the loop so the tail at the end wraps to the beginning.
    let delay = (rate * 0.375) as usize;
    let (dry_left, dry_right) = (left.clone(), right.clone());
    let (mut wet_left, mut wet_right) = (vec![0f32; frames], vec![0f32; frames]);
    for i in 0..frames * 2 {
        let at = i % frames;
        let from = (i + frames - delay) % frames;
        wet_left[at] = dry_right[from] * 0.3 + wet_right[from] * 0.35;
        wet_right[at] = dry_left[from] * 0.3 + wet_left[from] * 0.35;
    }

    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        for value in [left[i] + wet_left[i], right[i] + wet_right[i]] {
            // Soft limiter instead of clipping
            let limited = value.tanh() * 0.6;
            samples.push((limited * i16::MAX as f32) as i16);
        }
    }
    samples
}

/// Audio files a user can put into the music folder (played by `pw-cat`, which reads them with
/// libsndfile).
const EXTENSIONS: [&str; 5] = ["flac", "ogg", "oga", "opus", "wav"];

/// The user's music folder: `$XDG_DATA_HOME/consoleui/music`
pub fn music_dir() -> Option<PathBuf> {
    crate::xdg::data_home().map(|dir| dir.join("consoleui/music"))
}

/// Playable files in `dir` (not recursive), sorted by name.
pub fn tracks(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut tracks: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| EXTENSIONS.iter().any(|known| known.eq_ignore_ascii_case(e)))
        })
        .collect();
    tracks.sort();
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_is_quiet_and_seamless() {
        let samples = ambient_loop(1.0);
        assert_eq!(samples.len(), SAMPLE_RATE as usize * 4 * 2);
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak > 2_000 && peak < (i16::MAX as f32 * 0.6) as u16 + 1, "peak {peak}");
        // The jump from the last frame to the first is no larger than between neighbours.
        let step = |a: i16, b: i16| (a as i32 - b as i32).abs();
        let max_step = samples.chunks(2).zip(samples.chunks(2).skip(1)).map(|(a, b)| step(a[0], b[0])).max().unwrap();
        let seam = step(samples[samples.len() - 2], samples[0]);
        assert!(seam <= max_step, "seam {seam}, max step {max_step}");
    }

    #[test]
    fn finds_audio_files_only() {
        let dir = std::env::temp_dir().join(format!("consoleui-music-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["b.flac", "a.OGG", "cover.jpg", "notes.txt"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        assert_eq!(tracks(&dir), vec![dir.join("a.OGG"), dir.join("b.flac")]);
        assert!(tracks(&dir.join("missing")).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
