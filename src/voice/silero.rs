//! Silero VAD backend wrapper.
//!
//! 把 ONNX 加载和样本缓冲都封死在本模块里，给 voice 层和 dev trace 共用同一
//! 套帧边界（每 512 样本 = 32ms @ 16kHz）。`voice_activity_detector` 是
//! M10 默认依赖，[`SileroVad`] 在所有 build 里都可用。

use crate::voice::vad::VadFrame;

const SAMPLE_RATE: u64 = 16_000;
const SILERO_CHUNK_SAMPLES: usize = 512;

#[derive(Debug, Clone, Copy)]
pub struct SileroConfig {
    pub threshold: f32,
}

impl SileroConfig {
    /// 每一帧的样本数（固定 512）。
    pub const fn frame_samples() -> usize {
        SILERO_CHUNK_SAMPLES
    }

    /// 每一帧对应的毫秒数（512 / 16 = 32ms）。
    pub const fn frame_ms() -> u32 {
        (SILERO_CHUNK_SAMPLES as u32) * 1000 / SAMPLE_RATE as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SileroFrame {
    /// 该帧第一个样本在 recording timeline 上的索引。
    pub start_sample: u64,
    /// Silero 输出的语音概率 ∈ [0, 1]。
    pub probability: f32,
    /// 经过 threshold 二值化后的 frame 决策。
    pub frame: VadFrame,
}

pub struct SileroVad {
    detector: voice_activity_detector::VoiceActivityDetector,
    threshold: f32,
    buffer: Vec<i16>,
    sample_offset: u64,
}

impl SileroVad {
    pub fn new(config: SileroConfig) -> anyhow::Result<Self> {
        let detector = voice_activity_detector::VoiceActivityDetector::builder()
            .sample_rate(SAMPLE_RATE as i64)
            .chunk_size(SILERO_CHUNK_SAMPLES)
            .build()
            .map_err(|e| anyhow::anyhow!("create Silero VAD: {e}"))?;
        Ok(Self {
            detector,
            threshold: config.threshold,
            buffer: Vec::with_capacity(SILERO_CHUNK_SAMPLES),
            sample_offset: 0,
        })
    }

    /// 喂入任意长度的 PCM；每凑齐 512 样本就产生一个 [`SileroFrame`]。
    /// 不足 512 样本时缓存到下次调用，不会延迟出帧之外的副作用。
    pub fn accept(&mut self, samples: &[i16]) -> Vec<SileroFrame> {
        let mut out = Vec::new();
        self.buffer.extend_from_slice(samples);
        while self.buffer.len() >= SILERO_CHUNK_SAMPLES {
            let chunk: Vec<i16> = self.buffer.drain(..SILERO_CHUNK_SAMPLES).collect();
            let start_sample = self.sample_offset;
            self.sample_offset += SILERO_CHUNK_SAMPLES as u64;
            let probability = self.detector.predict(chunk.iter().copied());
            let frame = if probability >= self.threshold {
                VadFrame::Speech
            } else {
                VadFrame::Silence
            };
            out.push(SileroFrame {
                start_sample,
                probability,
                frame,
            });
        }
        out
    }

    /// 当前已处理样本数（= 已 emit 帧覆盖的样本数）。
    pub fn processed_samples(&self) -> u64 {
        self.sample_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_ms_matches_chunk_size_at_16khz() {
        assert_eq!(SileroConfig::frame_samples(), 512);
        assert_eq!(SileroConfig::frame_ms(), 32);
    }

    #[test]
    fn accept_emits_one_frame_per_512_samples() {
        let mut vad = SileroVad::new(SileroConfig { threshold: 0.5 }).unwrap();
        let silence = vec![0i16; 1024];
        let frames = vad.accept(&silence);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].start_sample, 0);
        assert_eq!(frames[1].start_sample, 512);
        for f in &frames {
            assert_eq!(f.frame, VadFrame::Silence);
            assert!((0.0..=1.0).contains(&f.probability));
        }
        assert_eq!(vad.processed_samples(), 1024);
    }

    #[test]
    fn accept_buffers_partial_chunks_until_full() {
        let mut vad = SileroVad::new(SileroConfig { threshold: 0.5 }).unwrap();
        assert!(vad.accept(&vec![0i16; 200]).is_empty());
        assert!(vad.accept(&vec![0i16; 200]).is_empty());
        let frames = vad.accept(&vec![0i16; 200]); // total 600 -> 1 frame, 88 leftover
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].start_sample, 0);
    }
}
