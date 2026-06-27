//! VAD-only audio preprocessing.
//!
//! This stage prepares a copy of canonical PCM for local VAD backends. It must
//! not mutate recorder audio, ASR audio, or retained audio files.

const TARGET_RMS: f32 = 0.10;
const NOISE_FLOOR_RMS: f32 = 0.0002;
const MIN_PEAK: f32 = 0.0015;
const MIN_GAIN: f32 = 1.0;
const MAX_GAIN: f32 = 24.0;
const GAIN_ATTACK: f32 = 0.35;
const GAIN_RELEASE: f32 = 0.05;

#[derive(Debug, Clone)]
pub(crate) struct VadPreprocessor {
    gain: f32,
}

impl VadPreprocessor {
    pub(crate) fn new() -> Self {
        Self { gain: MIN_GAIN }
    }

    pub(crate) fn process(&mut self, samples: &[i16]) -> Vec<i16> {
        #[cfg(target_os = "windows")]
        {
            self.process_adaptive_gain(samples)
        }
        #[cfg(not(target_os = "windows"))]
        {
            samples.to_vec()
        }
    }

    #[cfg(target_os = "windows")]
    fn process_adaptive_gain(&mut self, samples: &[i16]) -> Vec<i16> {
        let rms = normalized_rms(samples);
        let peak = normalized_peak(samples);
        if rms < NOISE_FLOOR_RMS || peak < MIN_PEAK {
            self.gain = smooth_gain(self.gain, MIN_GAIN);
            return apply_gain(samples, self.gain);
        }

        let target_gain = (TARGET_RMS / rms).clamp(MIN_GAIN, MAX_GAIN);
        self.gain = smooth_gain(self.gain, target_gain);
        apply_gain(samples, self.gain)
    }
}

impl Default for VadPreprocessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "windows")]
fn smooth_gain(current: f32, target: f32) -> f32 {
    let factor = if target > current {
        GAIN_ATTACK
    } else {
        GAIN_RELEASE
    };
    current + (target - current) * factor
}

#[cfg(target_os = "windows")]
fn normalized_rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let square_sum = samples
        .iter()
        .map(|sample| {
            let normalized = *sample as f32 / i16::MAX as f32;
            normalized * normalized
        })
        .sum::<f32>();
    (square_sum / samples.len() as f32).sqrt()
}

#[cfg(target_os = "windows")]
fn normalized_peak(samples: &[i16]) -> f32 {
    samples
        .iter()
        .map(|sample| sample.unsigned_abs() as f32 / i16::MAX as f32)
        .fold(0.0f32, f32::max)
}

#[cfg(target_os = "windows")]
fn apply_gain(samples: &[i16], gain: f32) -> Vec<i16> {
    samples
        .iter()
        .map(|sample| {
            let scaled = (*sample as f32 * gain).round();
            if scaled < i16::MIN as f32 {
                i16::MIN
            } else if scaled > i16::MAX as f32 {
                i16::MAX
            } else {
                scaled as i16
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_vad_preprocessor_amplifies_quiet_speech_like_frames() {
        let mut preprocessor = VadPreprocessor::new();
        let processed = preprocessor.process(&[-200, -100, 0, 100, 200]);

        assert!(processed[0] < -200);
        assert!(processed[4] > 200);
        assert_eq!(processed[2], 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_vad_preprocessor_leaves_tiny_noise_unchanged() {
        let noise = [-10, -5, 0, 5, 10];
        let mut preprocessor = VadPreprocessor::new();

        assert_eq!(preprocessor.process(&noise), noise);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_vad_preprocessor_keeps_weak_peaky_input_above_noise_gate() {
        let input = [-60, 0, 0, 0, 60];
        let mut preprocessor = VadPreprocessor::new();
        let processed = preprocessor.process(&input);

        assert!(processed[0] < input[0]);
        assert!(processed[4] > input[4]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_vad_preprocessor_smooths_gain_across_frames() {
        let mut preprocessor = VadPreprocessor::new();
        let first = preprocessor.process(&[-200, 0, 200]);
        let second = preprocessor.process(&[-200, 0, 200]);

        assert!(second[2] > first[2]);
        assert!(second[2] < 200 * MAX_GAIN as i16);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn apply_gain_saturates_to_i16_bounds() {
        let amplified = apply_gain(&[-5000, -100, 0, 100, 5000], 16.0);

        assert_eq!(amplified, [i16::MIN, -1600, 0, 1600, i16::MAX]);
    }
}
