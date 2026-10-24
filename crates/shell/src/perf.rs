use std::time::{Duration, Instant};

/// Gezählte Frames mit CPU-Renderzeit (BeforeRendering → AfterRendering).
#[derive(Debug, Clone, Copy, Default)]
pub struct Sample {
    pub frames: u32,
    pub busy: Duration,
    pub worst: Duration,
}

impl Sample {
    fn add(&mut self, frame: Duration) {
        self.frames += 1;
        self.busy += frame;
        self.worst = self.worst.max(frame);
    }

    pub fn average(&self) -> Duration {
        if self.frames == 0 { Duration::ZERO } else { self.busy / self.frames }
    }
}

/// Wird vom Rendering-Notifier gefüttert. `current` wird jede Sekunde für die Anzeige geleert,
/// `total` sammelt über einen ganzen Benchmark-Lauf.
#[derive(Default)]
pub struct FrameStats {
    frame_start: Option<Instant>,
    current: Sample,
    total: Sample,
}

impl FrameStats {
    pub fn frame_started(&mut self) {
        self.frame_start = Some(Instant::now());
    }

    pub fn frame_finished(&mut self) {
        if let Some(start) = self.frame_start.take() {
            let frame = start.elapsed();
            self.current.add(frame);
            self.total.add(frame);
        }
    }

    pub fn take_current(&mut self) -> Sample {
        std::mem::take(&mut self.current)
    }

    pub fn reset_total(&mut self) {
        self.total = Sample::default();
    }

    pub fn total(&self) -> Sample {
        self.total
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Text für die FPS-Anzeige; `sample` deckt genau eine Sekunde ab.
pub fn format_second(sample: &Sample) -> String {
    format!(
        "{} fps · Ø {:.2} ms · max {:.2} ms",
        sample.frames,
        ms(sample.average()),
        ms(sample.worst)
    )
}

pub fn print_bench_summary(tiles: usize, steps: usize, elapsed: Duration, total: Sample) {
    let secs = elapsed.as_secs_f64().max(f64::EPSILON);
    println!("Benchmark: {tiles} Kacheln, {steps} Schritte in {secs:.1} s");
    println!(
        "  Ø {:.1} fps | Renderzeit Ø {:.2} ms | max {:.2} ms",
        total.frames as f64 / secs,
        ms(total.average()),
        ms(total.worst)
    );
}
