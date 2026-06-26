//! Lightweight energy VAD backend.
//!
//! This backend is intentionally local and dependency-free. It exists to let
//! Windows exercise the shared VadPause state machine before the Silero/ONNX
//! Runtime distribution strategy is pinned down.

use crate::voice::vad::VadFrame;

const SAMPLE_RATE: u64 = 16_000;
const ENERGY_CHUNK_SAMPLES: usize = 512;

#[derive(Debug, Clone, Copy)]
pub(crate) struct EnergyVadConfig {
    pub(crate) threshold: f32,
}

impl EnergyVadConfig {
    pub(crate) const fn frame_ms() -> u32 {
        (ENERGY_CHUNK_SAMPLES as u32) * 1000 / SAMPLE_RATE as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EnergyVadFrame {
    pub(crate) start_sample: u64,
    pub(crate) probability: f32,
    pub(crate) frame: VadFrame,
}

#[derive(Debug)]
pub(crate) struct EnergyVad {
    threshold: f32,
    buffer: Vec<i16>,
    sample_offset: u64,
}

impl EnergyVad {
    pub(crate) fn new(config: EnergyVadConfig) -> Self {
        Self {
            threshold: config.threshold.clamp(0.0, 1.0),
            buffer: Vec::with_capacity(ENERGY_CHUNK_SAMPLES),
            sample_offset: 0,
        }
    }

    pub(crate) fn accept(&mut self, samples: &[i16]) -> Vec<EnergyVadFrame> {
        let mut out = Vec::new();
        self.buffer.extend_from_slice(samples);
        while self.buffer.len() >= ENERGY_CHUNK_SAMPLES {
            let chunk: Vec<i16> = self.buffer.drain(..ENERGY_CHUNK_SAMPLES).collect();
            let start_sample = self.sample_offset;
            self.sample_offset += ENERGY_CHUNK_SAMPLES as u64;
            let probability = speech_probability(&chunk);
            let frame = if probability >= self.threshold {
                VadFrame::Speech
            } else {
                VadFrame::Silence
            };
            out.push(EnergyVadFrame {
                start_sample,
                probability,
                frame,
            });
        }
        out
    }
}

fn speech_probability(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let sum_squares: f64 = samples
        .iter()
        .map(|sample| {
            let normalized = f64::from(*sample) / f64::from(i16::MAX);
            normalized * normalized
        })
        .sum();
    let rms = (sum_squares / samples.len() as f64).sqrt() as f32;
    (rms / 0.03).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_emits_one_frame_per_512_samples() {
        let mut vad = EnergyVad::new(EnergyVadConfig { threshold: 0.5 });
        let loud = vec![8_000i16; 1024];
        let frames = vad.accept(&loud);

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].start_sample, 0);
        assert_eq!(frames[1].start_sample, 512);
        assert_eq!(frames[0].frame, VadFrame::Speech);
    }

    #[test]
    fn accept_buffers_partial_chunks_until_full() {
        let mut vad = EnergyVad::new(EnergyVadConfig { threshold: 0.5 });
        assert!(vad.accept(&vec![8_000i16; 200]).is_empty());
        assert!(vad.accept(&vec![8_000i16; 200]).is_empty());

        let frames = vad.accept(&vec![8_000i16; 200]);

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].start_sample, 0);
    }

    #[test]
    fn low_energy_is_silence() {
        let mut vad = EnergyVad::new(EnergyVadConfig { threshold: 0.5 });
        let frames = vad.accept(&vec![100i16; 512]);

        assert_eq!(frames[0].frame, VadFrame::Silence);
        assert!(frames[0].probability < 0.5);
    }

    #[test]
    fn moderate_voice_level_crosses_default_threshold() {
        let mut vad = EnergyVad::new(EnergyVadConfig { threshold: 0.5 });
        let frames = vad.accept(&vec![1_000i16; 512]);

        assert_eq!(frames[0].frame, VadFrame::Speech);
        assert!(frames[0].probability >= 0.5);
    }
}
