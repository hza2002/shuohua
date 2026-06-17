use crate::state::AudioMeter;

const SAMPLE_RATE: usize = 16_000;
const WINDOW_MS: usize = 50;
const WINDOW_SAMPLES: usize = SAMPLE_RATE * WINDOW_MS / 1000;

/// Interval at which `AudioMeter` events are emitted, in milliseconds.
/// The TUI render tick should match this to avoid wasted frames.
pub const METER_INTERVAL_MS: u64 = WINDOW_MS as u64;

#[derive(Debug, Clone)]
pub struct MeterCollector {
    sum_squares: f64,
    peak_abs: u16,
    samples: usize,
    vad_probability: Option<f32>,
    vad_speech: Option<bool>,
    clipped: bool,
}

impl MeterCollector {
    pub fn new() -> Self {
        Self {
            sum_squares: 0.0,
            peak_abs: 0,
            samples: 0,
            vad_probability: None,
            vad_speech: None,
            clipped: false,
        }
    }

    pub fn observe_vad(&mut self, probability: f32, speech: bool) {
        self.vad_probability = Some(probability.clamp(0.0, 1.0));
        self.vad_speech = Some(speech);
    }

    pub fn accept(&mut self, samples: &[i16]) -> Vec<AudioMeter> {
        let mut out = Vec::new();
        for &sample in samples {
            let abs = sample.unsigned_abs();
            self.peak_abs = self.peak_abs.max(abs);
            self.clipped |= abs >= i16::MAX as u16;
            let normalized = f64::from(sample) / f64::from(i16::MAX);
            self.sum_squares += normalized * normalized;
            self.samples += 1;
            if self.samples >= WINDOW_SAMPLES {
                out.push(self.finish_window());
            }
        }
        out
    }

    fn finish_window(&mut self) -> AudioMeter {
        let rms = if self.samples == 0 {
            0.0
        } else {
            (self.sum_squares / self.samples as f64).sqrt() as f32
        };
        let meter = AudioMeter {
            rms: rms.clamp(0.0, 1.0),
            peak: (self.peak_abs as f32 / i16::MAX as f32).clamp(0.0, 1.0),
            clipped: self.clipped,
            vad_probability: self.vad_probability,
            vad_speech: self.vad_speech,
        };
        self.sum_squares = 0.0;
        self.peak_abs = 0;
        self.samples = 0;
        self.clipped = false;
        meter
    }
}

impl Default for MeterCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_one_meter_per_50ms_window() {
        let mut collector = MeterCollector::new();
        let meters = collector.accept(&vec![i16::MAX / 2; WINDOW_SAMPLES]);

        assert_eq!(meters.len(), 1);
        assert!((0.49..0.51).contains(&meters[0].rms));
        assert!((0.49..0.51).contains(&meters[0].peak));
    }

    #[test]
    fn carries_latest_vad_values_into_meter() {
        let mut collector = MeterCollector::new();
        collector.observe_vad(0.8, true);
        let meters = collector.accept(&vec![0; WINDOW_SAMPLES]);

        assert_eq!(meters[0].vad_probability, Some(0.8));
        assert_eq!(meters[0].vad_speech, Some(true));
    }

    #[test]
    fn marks_clipped_windows() {
        let mut collector = MeterCollector::new();
        let mut samples = vec![0; WINDOW_SAMPLES];
        samples[0] = i16::MAX;
        let meters = collector.accept(&samples);

        assert!(meters[0].clipped);
        assert_eq!(meters[0].peak, 1.0);
    }
}
