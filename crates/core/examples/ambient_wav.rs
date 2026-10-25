//! Writes the built-in background music loop to a WAV file, to listen to it outside the shell:
//! `cargo run -p consoleui-core --example ambient_wav -- /tmp/ambient.wav`

use consoleui_core::music;
use consoleui_core::sound::SAMPLE_RATE;

fn main() -> std::io::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "ambient.wav".into());
    let samples = music::ambient_loop(music::CHORD_SECONDS);
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    // PCM, 2 channels, 16 bit
    for value in [16u32, 1 | (2 << 16), SAMPLE_RATE, SAMPLE_RATE * 4, 4 | (16 << 16)] {
        wav.extend_from_slice(&value.to_le_bytes());
    }
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(&path, wav)?;
    println!("{path}: {} s", music::CHORD_SECONDS * 4.0);
    Ok(())
}
