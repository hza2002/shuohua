use anyhow::Context;
use anyhow::Result;

/// WebRTC APM 固定在 16 kHz 运行。16k 正好是 ASR 的 canonical 目标率，也是 WebRTC
/// 最成熟的工作率；任何设备率先一次性重采样到 16k 再进 APM，输出已是 16k，recorder
/// 的 `Resampler16k` 退化成 passthrough——全链路只做 1 次重采样（旧实现在非
/// 16/32/48k 设备上会重采样两次）。
const APM_RATE_HZ: u32 = 16_000;

pub(crate) struct WebRtcPreprocessor {
    inner: WebRtcInner,
    input_resampler: Option<crate::voice::recorder::ResamplerForPreprocess>,
    chunker: FrameChunker,
}

impl WebRtcPreprocessor {
    pub(crate) fn new(sample_rate: u32) -> Result<Self> {
        let input_resampler = (sample_rate != APM_RATE_HZ)
            .then(|| crate::voice::recorder::ResamplerForPreprocess::new(sample_rate, APM_RATE_HZ));
        let frame_samples = (APM_RATE_HZ / 100) as usize;
        Ok(Self {
            inner: WebRtcInner::new(APM_RATE_HZ)?,
            input_resampler,
            chunker: FrameChunker::new(frame_samples),
        })
    }

    pub(crate) fn output_sample_rate(&self) -> u32 {
        APM_RATE_HZ
    }

    pub(crate) fn process_mono_chunk(&mut self, mono: &[f32]) -> Result<Vec<f32>> {
        let input = if let Some(resampler) = self.input_resampler.as_mut() {
            resampler.process(mono)?
        } else {
            mono.to_vec()
        };
        self.process_apm_input(&input)
    }

    pub(crate) fn finish(&mut self) -> Result<Vec<f32>> {
        let mut input = if let Some(resampler) = self.input_resampler.as_mut() {
            resampler.finish()?
        } else {
            Vec::new()
        };
        let pending_len = self.chunker.pending_len() + input.len();
        if pending_len == 0 {
            return Ok(Vec::new());
        }
        let padded_len = pending_len.next_multiple_of(self.chunker.frame_samples());
        input.resize(input.len() + (padded_len - pending_len), 0.0);
        let mut out = self.process_apm_input(&input)?;
        out.truncate(pending_len);
        Ok(out)
    }

    fn process_apm_input(&mut self, input: &[f32]) -> Result<Vec<f32>> {
        let mut out = Vec::new();
        for mut frame in self.chunker.push(input) {
            self.inner.process_capture_frame(&mut frame)?;
            out.extend(frame);
        }
        Ok(out)
    }
}

/// 锁定的 curated preset——面向人声、保守、"默认就够好"，用户面不暴露任何子参数。
/// 只用 WebRTC 稳定主干：high-pass + noise suppression + digital AGC1。
/// 刻意不接（理由见 docs/modules/webrtc_backend.md）：
/// - AEC：没有 system-audio 参考帧，回声场景请用 `backend = "apple"`；
/// - capture amplifier：AGC AdaptiveDigital 已自适应增益，固定前置增益反而易削波；
/// - AGC2：新增能力（input volume controller）需要用不上的 HAL 音量耦合。
///
/// 改这里的值前先读 docs/modules/webrtc_backend.md，并做人工 A/B 验证。
fn curated_config() -> webrtc_audio_processing::config::Config {
    use webrtc_audio_processing::config::{
        Config, GainController, GainController1, GainControllerMode, HighPassFilter,
        NoiseSuppression, NoiseSuppressionLevel,
    };

    Config {
        // NS 本就强制启用 HPF；显式写出只是让 full-band(20–20kHz) 意图明确。
        high_pass_filter: Some(HighPassFilter::default()),
        // Moderate 是 crate 自己 tune 的平衡档，也是 ASR 前处理的安全默认：宁可欠抑制
        // 也别过抑制——过度 NS 会连语音谐波/清辅音一起削掉、升 WER。更激进的 High/
        // VeryHigh 需实测证明对识别有净增益才升，不作默认。
        noise_suppression: Some(NoiseSuppression {
            level: NoiseSuppressionLevel::Moderate,
            ..Default::default()
        }),
        // AdaptiveDigital 而非 crate 默认的 AdaptiveAnalog：我们没有把 OS 麦克风音量
        // 耦合进 AGC（stream_analog_level），analog 模式会空转。target=6 即 -6 dBFS，
        // 留足 headroom 防削波。
        gain_controller: Some(GainController::GainController1(GainController1 {
            mode: GainControllerMode::AdaptiveDigital,
            target_level_dbfs: 6,
            compression_gain_db: 9,
            enable_limiter: true,
            analog_gain_controller: None,
        })),
        ..Default::default()
    }
}

struct WebRtcInner {
    processor: webrtc_audio_processing::Processor,
}

impl WebRtcInner {
    fn new(sample_rate: u32) -> Result<Self> {
        let processor = webrtc_audio_processing::Processor::new(sample_rate)
            .context("create WebRTC audio processor")?;
        processor.set_config(curated_config());
        Ok(Self { processor })
    }

    fn process_capture_frame(&self, frame: &mut Vec<f32>) -> Result<()> {
        self.processor
            .process_capture_frame([frame.as_mut_slice()])
            .context("process WebRTC capture frame")
    }
}

struct FrameChunker {
    pending: Vec<f32>,
    frame_samples: usize,
}

impl FrameChunker {
    fn new(frame_samples: usize) -> Self {
        Self {
            pending: Vec::new(),
            frame_samples,
        }
    }

    fn frame_samples(&self) -> usize {
        self.frame_samples
    }

    fn pending_len(&self) -> usize {
        self.pending.len()
    }

    fn push(&mut self, input: &[f32]) -> Vec<Vec<f32>> {
        self.pending.extend_from_slice(input);
        let frame_count = self.pending.len() / self.frame_samples;
        let mut frames = Vec::with_capacity(frame_count);
        for _ in 0..frame_count {
            frames.push(self.pending.drain(..self.frame_samples).collect());
        }
        frames
    }

    #[cfg(test)]
    fn finish_padded(&mut self) -> Vec<Vec<f32>> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let padded_len = self.pending.len().next_multiple_of(self.frame_samples);
        self.pending.resize(padded_len, 0.0);
        self.push(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunker_holds_partial_frames_until_10ms_is_available() {
        let mut chunker = FrameChunker::new(480);

        assert_eq!(chunker.push(&vec![0.25; 479]).len(), 0);
        let frames = chunker.push(&[0.25]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), 480);
        assert_eq!(chunker.finish_padded().len(), 0);
    }

    #[test]
    fn finish_zero_pads_final_partial_frame_then_trims_to_original_length() {
        let mut chunker = FrameChunker::new(160);

        assert_eq!(chunker.push(&vec![0.25; 80]).len(), 0);
        let frames = chunker.finish_padded();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), 160);
        assert!(frames[0][..80].iter().all(|sample| *sample == 0.25));
        assert!(frames[0][80..].iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn any_input_rate_runs_apm_at_16k() {
        for src_rate in [16_000, 44_100, 48_000] {
            let processor = WebRtcPreprocessor::new(src_rate).unwrap();
            assert_eq!(processor.output_sample_rate(), 16_000);
        }
    }

    #[test]
    fn curated_config_locks_voice_tuned_preset() {
        use webrtc_audio_processing::config::{
            GainController, GainControllerMode, NoiseSuppressionLevel,
        };

        let config = curated_config();

        let ns = config.noise_suppression.expect("noise suppression enabled");
        assert_eq!(ns.level, NoiseSuppressionLevel::Moderate);

        let hpf = config.high_pass_filter.expect("high-pass filter enabled");
        assert!(hpf.apply_in_full_band);

        match config.gain_controller.expect("gain controller enabled") {
            GainController::GainController1(agc) => {
                // AdaptiveDigital 是关键：不能退回 crate 默认的 AdaptiveAnalog。
                assert_eq!(agc.mode, GainControllerMode::AdaptiveDigital);
                assert_eq!(agc.target_level_dbfs, 6);
                assert_eq!(agc.compression_gain_db, 9);
                assert!(agc.enable_limiter);
                assert!(agc.analog_gain_controller.is_none());
            }
            other => panic!("expected AGC1, got {other:?}"),
        }

        // 刻意不接的能力必须保持关闭。
        assert!(config.echo_canceller.is_none());
        assert!(config.capture_amplifier.is_none());
    }
}
