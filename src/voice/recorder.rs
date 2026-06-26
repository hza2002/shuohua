//! 流式 cpal 录音：default input → stateful resample → 16k mono s16le 帧 → mpsc。
//!
//! 一次录音一个 [`RecordingStream`]，背后是一个专用 std 线程跑 cpal stream
//! (cpal::Stream 是 !Send，不能跨线程移动)。callback 把 PCM 帧 push 到 tokio
//! unbounded mpsc，async 端按需 recv。启用 retained audio 时，同一线程先写
//! 临时 WAV，停止后再交给 `voice::audio` 转成最终 FLAC/AAC。
//!
//! Stop 协议：voice 端显式选择发布或丢弃 retained audio，cpal 线程收到信号后
//! drop stream。发布路径 finalize 临时 WAV 并完成格式转换；丢弃路径只清理临时
//! 文件。drop stream 时 cpal 自动 drain callback in-flight 的 buffer。
//!
use anyhow::{anyhow, bail, Context, Result};
use audioadapter_buffers::direct::InterleavedSlice;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, SupportedStreamConfig};
use rubato::{Fft, FixedSync, Indexing, Resampler};
use std::path::{Path, PathBuf};
use tokio::sync::{mpsc, oneshot};

use crate::voice::audio::AudioOutput;

const DST_RATE_HZ: u32 = 16_000;

pub struct RecordingStream {
    pcm_rx: mpsc::UnboundedReceiver<Vec<i16>>,
    stop: Option<std::sync::mpsc::Sender<FinishMode>>,
    audio_result: Option<oneshot::Receiver<Result<Option<PathBuf>>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinishMode {
    Publish,
    Discard,
}

pub(crate) fn probe_default_input() -> Result<crate::platform::audio_capture::InputDeviceInfo> {
    crate::platform::audio_capture::probe_default_input()
}

impl RecordingStream {
    pub async fn recv(&mut self) -> Option<Vec<i16>> {
        self.pcm_rx.recv().await
    }

    /// 立刻收 cpal 线程；之后 recv() 会很快变成 None (取决于 cpal drain)。
    pub fn stop(&mut self) {
        self.request_stop(FinishMode::Publish);
    }

    pub async fn drain_after_stop(&mut self) -> Vec<Vec<i16>> {
        let stop_requested = self.request_stop(FinishMode::Publish);
        let mut out = Vec::new();
        if stop_requested {
            while let Some(samples) = self.recv().await {
                out.push(samples);
            }
        } else {
            while let Some(samples) = self.try_recv() {
                out.push(samples);
            }
        }
        out
    }

    pub async fn discard_audio(&mut self) -> Result<()> {
        self.request_stop(FinishMode::Discard);
        let _ = self.take_audio_result().await?;
        Ok(())
    }

    /// 非 await 的 try_recv（finishing 阶段一次性吸干残余帧用）。
    pub fn try_recv(&mut self) -> Option<Vec<i16>> {
        self.pcm_rx.try_recv().ok()
    }

    pub async fn finish_audio(&mut self) -> Result<Option<PathBuf>> {
        self.stop();
        self.take_audio_result().await
    }

    fn request_stop(&mut self, mode: FinishMode) -> bool {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(mode);
            true
        } else {
            false
        }
    }

    async fn take_audio_result(&mut self) -> Result<Option<PathBuf>> {
        let Some(receiver) = self.audio_result.take() else {
            return Ok(None);
        };
        receiver
            .await
            .context("recorder audio result channel closed")?
    }
}

impl Drop for RecordingStream {
    fn drop(&mut self) {
        self.request_stop(FinishMode::Discard);
    }
}

#[cfg(test)]
impl RecordingStream {
    /// 测试用构造：跳过 cpal/wav，外部 mpsc::UnboundedSender 驱动 PCM；
    /// `finish_audio()` 立即返回 `Ok(None)`。
    pub(crate) fn for_test(pcm_rx: mpsc::UnboundedReceiver<Vec<i16>>) -> Self {
        let (audio_tx, audio_rx) = oneshot::channel();
        let _ = audio_tx.send(Ok(None));
        Self {
            pcm_rx,
            stop: None,
            audio_result: Some(audio_rx),
        }
    }

    /// 测试用构造：与 [`for_test`] 相同，但暴露 stop 信号通道，便于断言收尾
    /// 走 Publish 还是 Discard（retained audio 留存 vs 丢弃）。
    ///
    /// 注意：本构造持有真实 stop sender，`drain_after_stop` 会阻塞 `recv` 直到
    /// PCM 通道关闭（与真实 recorder 一致）。stop 路径的测试需在发出 stop 后
    /// drop PCM 发送端，模拟 cpal stream 关闭，否则 drain 不会返回。
    pub(crate) fn for_test_observe(
        pcm_rx: mpsc::UnboundedReceiver<Vec<i16>>,
    ) -> (Self, std::sync::mpsc::Receiver<FinishMode>) {
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let (audio_tx, audio_rx) = oneshot::channel();
        let _ = audio_tx.send(Ok(None));
        (
            Self {
                pcm_rx,
                stop: Some(stop_tx),
                audio_result: Some(audio_rx),
            },
            stop_rx,
        )
    }
}

/// 启动一路录音。`audio_output = Some(...)` 时把同一份 PCM 写入临时 WAV，
/// 录音线程停止后转换成最终 retained-audio 文件。
pub fn start(audio_output: Option<AudioOutput>) -> Result<RecordingStream> {
    let (pcm_tx, pcm_rx) = mpsc::unbounded_channel::<Vec<i16>>();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<FinishMode>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
    let (audio_result_tx, audio_result_rx) = oneshot::channel();

    std::thread::Builder::new()
        .name("cpal-recorder".into())
        .spawn(move || {
            if let Err(e) = run_recorder(pcm_tx, stop_rx, audio_output, ready_tx, audio_result_tx) {
                tracing::warn!(error = ?e, "recorder thread exited with error");
            }
        })
        .context("spawn recorder thread")?;

    ready_rx
        .recv()
        .context("wait for recorder startup")?
        .context("start recorder")?;

    Ok(RecordingStream {
        pcm_rx,
        stop: Some(stop_tx),
        audio_result: Some(audio_result_rx),
    })
}

fn run_recorder(
    pcm_tx: mpsc::UnboundedSender<Vec<i16>>,
    stop_rx: std::sync::mpsc::Receiver<FinishMode>,
    audio_output: Option<AudioOutput>,
    ready_tx: std::sync::mpsc::Sender<Result<()>>,
    audio_result_tx: oneshot::Sender<Result<Option<PathBuf>>>,
) -> Result<()> {
    let audio_wav_path = audio_output.as_ref().map(|output| output.wav_path.clone());
    let startup = build_recorder_stream(audio_wav_path);
    match startup {
        Ok((stream, audio_rx, wav, src_rate)) => {
            let _ = ready_tx.send(Ok(()));
            let mut processor = PcmProcessor::new(src_rate, pcm_tx, wav);
            let mut finish_mode = FinishMode::Discard;

            loop {
                match stop_rx.try_recv() {
                    Ok(mode) => {
                        finish_mode = mode;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }

                match audio_rx.recv_timeout(std::time::Duration::from_millis(20)) {
                    Ok(chunk) => processor.process_mono_chunk(&chunk),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }

            drop(stream); // cpal drain & close

            if finish_mode == FinishMode::Publish {
                while let Ok(chunk) = audio_rx.recv() {
                    processor.process_mono_chunk(&chunk);
                }
            }

            let finalize_result = processor.finish(finish_mode);
            let audio_result = complete_audio(finish_mode, finalize_result, audio_output);
            let _ = audio_result_tx.send(audio_result);
            Ok(())
        }
        Err(e) => {
            let msg = format!("{e:#}");
            if let Some(output) = audio_output {
                output.discard();
            }
            let _ = audio_result_tx.send(Err(anyhow!(msg.clone())));
            let _ = ready_tx.send(Err(anyhow!(msg)));
            Err(e)
        }
    }
}

fn complete_audio(
    finish_mode: FinishMode,
    finalize_result: Result<()>,
    audio_output: Option<AudioOutput>,
) -> Result<Option<PathBuf>> {
    if finish_mode == FinishMode::Discard {
        if let Some(output) = audio_output {
            output.discard();
        }
        return Ok(None);
    }

    match (finalize_result, audio_output) {
        (Ok(()), Some(output)) => output.finish().map(Some),
        (Ok(()), None) => Ok(None),
        (Err(error), Some(output)) => {
            output.discard();
            Err(error)
        }
        (Err(error), None) => Err(error),
    }
}

type WavWriter = hound::WavWriter<std::io::BufWriter<std::fs::File>>;

/// build_recorder_stream 的返回组件：cpal stream、原始 f32 帧通道、可选 WAV
/// writer、输入采样率。
type RecorderStreamSetup = (
    cpal::Stream,
    std::sync::mpsc::Receiver<Vec<f32>>,
    Option<WavWriter>,
    u32,
);

fn build_recorder_stream(audio_wav_path: Option<PathBuf>) -> Result<RecorderStreamSetup> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("no default input device"))?;
    let supported = device
        .default_input_config()
        .context("query default input config")?;
    validate_supported_config(&supported)?;
    let src_rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    tracing::debug!(src_rate, channels, "recorder format selected");

    let wav = open_wav(audio_wav_path.as_deref())?;
    let (audio_tx, audio_rx) = std::sync::mpsc::channel::<Vec<f32>>();

    let sample_format = supported.sample_format();
    let config = supported.into();
    let stream = match sample_format {
        SampleFormat::F32 => build_input_stream::<f32>(&device, config, channels, audio_tx),
        SampleFormat::I16 => build_input_stream::<i16>(&device, config, channels, audio_tx),
        SampleFormat::U16 => build_input_stream::<u16>(&device, config, channels, audio_tx),
        other => bail!("recorder requires F32/I16/U16 input, got {other:?}"),
    }
    .context("build input stream")?;

    stream.play().context("start input stream")?;
    Ok((stream, audio_rx, wav, src_rate))
}

fn validate_supported_config(supported: &SupportedStreamConfig) -> Result<()> {
    let sample_format = supported.sample_format();
    if !is_supported_input_format(sample_format) {
        bail!("recorder requires F32/I16/U16 input, got {sample_format:?}");
    }
    if supported.channels() == 0 {
        bail!("default input device reports 0 channels");
    }
    Ok(())
}

fn is_supported_input_format(format: SampleFormat) -> bool {
    matches!(
        format,
        SampleFormat::F32 | SampleFormat::I16 | SampleFormat::U16
    )
}

fn open_wav(path: Option<&Path>) -> Result<Option<WavWriter>> {
    let Some(path) = path else { return Ok(None) };
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: DST_RATE_HZ,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let writer = hound::WavWriter::create(path, spec)
        .with_context(|| format!("create wav {}", path.display()))?;
    Ok(Some(writer))
}

fn build_input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    audio_tx: std::sync::mpsc::Sender<Vec<f32>>,
) -> Result<cpal::Stream>
where
    T: InputSample + SizedSample,
{
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let mono = input_to_mono(data, channels);
            if mono.is_empty() {
                return;
            }
            // std mpsc send 不会阻塞 cpal callback。voice 消费滞后只是增加
            // 内存占用，不会丢帧。重采样和 WAV 写入在 recorder 线程完成。
            let _ = audio_tx.send(mono);
        },
        |err| tracing::warn!(error = %err, "recorder stream error"),
        None,
    )?;
    Ok(stream)
}

trait InputSample: Copy + Send + 'static {
    fn to_f32(self) -> f32;
}

impl InputSample for f32 {
    fn to_f32(self) -> f32 {
        self
    }
}

impl InputSample for i16 {
    fn to_f32(self) -> f32 {
        if self == i16::MIN {
            -1.0
        } else {
            self as f32 / i16::MAX as f32
        }
    }
}

impl InputSample for u16 {
    fn to_f32(self) -> f32 {
        (self as f32 - 32768.0) / 32767.0
    }
}

fn input_to_mono<T: InputSample>(data: &[T], channels: usize) -> Vec<f32> {
    if channels == 0 || data.is_empty() {
        return Vec::new();
    }
    data.chunks_exact(channels)
        .map(|frame| frame[0].to_f32())
        .collect()
}

struct PcmProcessor {
    resampler: Resampler16k,
    pcm_tx: mpsc::UnboundedSender<Vec<i16>>,
    wav: Option<WavWriter>,
    resample_error: Option<anyhow::Error>,
    audio_error: Option<anyhow::Error>,
}

impl PcmProcessor {
    fn new(src_rate: u32, pcm_tx: mpsc::UnboundedSender<Vec<i16>>, wav: Option<WavWriter>) -> Self {
        Self {
            resampler: Resampler16k::new(src_rate),
            pcm_tx,
            wav,
            resample_error: None,
            audio_error: None,
        }
    }

    fn process_mono_chunk(&mut self, mono: &[f32]) {
        if self.resample_error.is_some() {
            return;
        }
        match self.resampler.process(mono) {
            Ok(pcm) => self.publish_pcm(&pcm),
            Err(error) => self.resample_error = Some(error),
        }
    }

    fn finish(&mut self, finish_mode: FinishMode) -> Result<()> {
        if finish_mode == FinishMode::Discard {
            let _ = self.wav.take();
            return Ok(());
        }

        if self.resample_error.is_none() {
            match self.resampler.finish() {
                Ok(pcm) => self.publish_pcm(&pcm),
                Err(error) => self.resample_error = Some(error),
            }
        }

        if let Some(wav) = self.wav.take() {
            wav.finalize().context("finalize wav")?;
        }

        if let Some(error) = self.resample_error.take() {
            return Err(error);
        }
        if let Some(error) = self.audio_error.take() {
            return Err(error);
        }
        Ok(())
    }

    fn publish_pcm(&mut self, pcm: &[i16]) {
        if pcm.is_empty() {
            return;
        }
        if let Some(wav) = self.wav.as_mut() {
            for &s in pcm {
                if let Err(error) = wav.write_sample(s).context("write wav sample") {
                    self.audio_error = Some(error);
                    self.wav.take();
                    break;
                }
            }
        }
        let _ = self.pcm_tx.send(pcm.to_vec());
    }
}

enum Resampler16k {
    Passthrough,
    Rubato(Box<RubatoResampler16k>),
}

impl Resampler16k {
    fn new(src_rate: u32) -> Self {
        if src_rate == DST_RATE_HZ {
            Self::Passthrough
        } else {
            Self::Rubato(Box::new(RubatoResampler16k::new(src_rate)))
        }
    }

    fn process(&mut self, mono: &[f32]) -> Result<Vec<i16>> {
        match self {
            Self::Passthrough => Ok(mono_to_i16(mono)),
            Self::Rubato(resampler) => resampler.process(mono),
        }
    }

    fn finish(&mut self) -> Result<Vec<i16>> {
        match self {
            Self::Passthrough => Ok(Vec::new()),
            Self::Rubato(resampler) => resampler.finish(),
        }
    }
}

struct RubatoResampler16k {
    src_rate: u32,
    inner: Fft<f32>,
    pending: Vec<f32>,
    output_buf: Vec<f32>,
    input_frames: usize,
    output_frames: usize,
    trim_output_frames: usize,
}

impl RubatoResampler16k {
    fn new(src_rate: u32) -> Self {
        let inner = Fft::<f32>::new(
            src_rate as usize,
            DST_RATE_HZ as usize,
            1024,
            2,
            1,
            FixedSync::Input,
        )
        .expect("valid recorder sample rates");
        let output_buf = vec![0.0; inner.output_frames_max()];
        let trim_output_frames = inner.output_delay();
        Self {
            src_rate,
            inner,
            pending: Vec::new(),
            output_buf,
            input_frames: 0,
            output_frames: 0,
            trim_output_frames,
        }
    }

    fn process(&mut self, mono: &[f32]) -> Result<Vec<i16>> {
        if mono.is_empty() {
            return Ok(Vec::new());
        }
        self.input_frames += mono.len();
        self.pending.extend_from_slice(mono);

        let mut output = Vec::new();
        while self.pending.len() >= self.inner.input_frames_next() {
            let (used, produced) = self.process_next(None)?;
            self.pending.drain(..used);
            self.append_output(&mut output, produced, None);
        }
        Ok(output)
    }

    fn finish(&mut self) -> Result<Vec<i16>> {
        let mut output = Vec::new();
        let mut pending_output = None;
        if !self.pending.is_empty() {
            let partial_len = self.pending.len();
            self.pending.resize(self.inner.input_frames_next(), 0.0);
            let (used, produced) = self.process_next(Some(partial_len))?;
            self.pending.drain(..used.min(self.pending.len()));
            pending_output = Some(produced);
        }

        let target_frames = ((self.input_frames as f64 * DST_RATE_HZ as f64) / self.src_rate as f64)
            .round() as usize;
        if !self.pending.is_empty() {
            unreachable!("partial processing drains pending input");
        }
        if let Some(produced) = pending_output {
            self.append_output(&mut output, produced, Some(target_frames));
        }
        while self.output_frames < target_frames {
            self.pending.resize(self.inner.input_frames_next(), 0.0);
            let (_, produced) = self.process_next(Some(0))?;
            self.pending.clear();
            self.append_output(&mut output, produced, Some(target_frames));
        }
        Ok(output)
    }

    fn process_next(&mut self, partial_len: Option<usize>) -> Result<(usize, usize)> {
        let input = InterleavedSlice::new(&self.pending, 1, self.pending.len())
            .context("create rubato input adapter")?;
        let output_len = self.output_buf.len();
        let mut output = InterleavedSlice::new_mut(&mut self.output_buf, 1, output_len)
            .context("create rubato output adapter")?;
        let indexing = partial_len.map(|partial_len| Indexing {
            input_offset: 0,
            output_offset: 0,
            active_channels_mask: None,
            partial_len: Some(partial_len),
        });
        self.inner
            .process_into_buffer(&input, &mut output, indexing.as_ref())
            .context("resample recorder audio")
    }

    fn append_output(&mut self, output: &mut Vec<i16>, produced: usize, cap: Option<usize>) {
        let mut start = 0;
        if self.trim_output_frames > 0 {
            let trimmed = self.trim_output_frames.min(produced);
            self.trim_output_frames -= trimmed;
            start = trimmed;
        }

        let mut end = produced;
        if let Some(cap) = cap {
            let remaining = cap.saturating_sub(self.output_frames);
            end = end.min(start + remaining);
        }
        if end <= start {
            return;
        }

        output.extend(mono_to_i16(&self.output_buf[start..end]));
        self.output_frames += end - start;
    }
}

#[cfg(test)]
fn resample_input_to_16k_mono_i16<T: InputSample>(
    data: &[T],
    channels: usize,
    src_rate: u32,
) -> Vec<i16> {
    let mono = input_to_mono(data, channels);
    resample_mono_to_16k_i16(&mono, src_rate)
}

#[cfg(test)]
fn resample_to_16k_mono_i16(data: &[f32], channels: usize, src_rate: u32) -> Vec<i16> {
    if channels == 0 || data.is_empty() {
        return Vec::new();
    }
    let mono: Vec<f32> = data.chunks_exact(channels).map(|f| f[0]).collect();
    resample_mono_to_16k_i16(&mono, src_rate)
}

#[cfg(test)]
fn resample_mono_to_16k_i16(mono: &[f32], src_rate: u32) -> Vec<i16> {
    let mut resampler = Resampler16k::new(src_rate);
    let mut out = resampler.process(mono).unwrap_or_default();
    out.extend(resampler.finish().unwrap_or_default());
    out
}

fn mono_to_i16(mono: &[f32]) -> Vec<i16> {
    mono.iter()
        .map(|&s| {
            let scaled = (s.clamp(-1.0, 1.0) * 32767.0).round();
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
fn resample_mono_chunks_to_16k_i16(
    chunks: impl IntoIterator<Item = Vec<f32>>,
    src_rate: u32,
) -> Vec<i16> {
    let mut resampler = Resampler16k::new(src_rate);
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend(resampler.process(&chunk).unwrap());
    }
    out.extend(resampler.finish().unwrap());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RecordAudioMode;
    use std::time::Duration;

    #[test]
    fn resample_passthrough_when_rates_match() {
        let data = vec![0.5f32, -0.5, 0.25, -0.25];
        let out = resample_to_16k_mono_i16(&data, 1, 16_000);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0], (0.5_f32 * 32767.0).round() as i16);
        assert_eq!(out[1], (-0.5_f32 * 32767.0).round() as i16);
    }

    #[test]
    fn resample_downsamples_48k_to_16k() {
        // 48k → 16k 比率 3；输出应约为输入 1/3
        let data: Vec<f32> = (0..480).map(|i| (i as f32 / 480.0) - 0.5).collect();
        let out = resample_to_16k_mono_i16(&data, 1, 48_000);
        assert!(out.len() >= 158 && out.len() <= 162, "got {}", out.len());
    }

    #[test]
    fn resample_takes_first_channel_for_stereo() {
        // 2ch interleaved；左声道 = 0.5，右声道 = -0.5
        let data = vec![0.5, -0.5, 0.5, -0.5];
        let out = resample_to_16k_mono_i16(&data, 2, 16_000);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], out[1]);
        assert!(out[0] > 0); // 左声道是正值
    }

    #[test]
    fn split_callbacks_preserve_44100_to_16k_duration() {
        let input = vec![0.0f32; 44_100];
        let output =
            resample_mono_chunks_to_16k_i16(input.chunks(512).map(|chunk| chunk.to_vec()), 44_100);

        assert_eq!(output.len(), 16_000);
    }

    #[test]
    fn i16_input_passthrough_preserves_existing_pipeline_format() {
        let data = vec![100i16, -100, 300, -300];
        let out = resample_input_to_16k_mono_i16(&data, 1, 16_000);

        assert_eq!(out, data);
    }

    #[test]
    fn u16_input_is_centered_before_pipeline_conversion() {
        let data = vec![0u16, 32768, 65535];
        let out = resample_input_to_16k_mono_i16(&data, 1, 16_000);

        assert_eq!(out, vec![-32767, 0, 32767]);
    }

    #[test]
    fn supported_input_formats_match_recorder_stream_dispatch() {
        assert!(is_supported_input_format(SampleFormat::F32));
        assert!(is_supported_input_format(SampleFormat::I16));
        assert!(is_supported_input_format(SampleFormat::U16));
        assert!(!is_supported_input_format(SampleFormat::F64));
    }

    #[test]
    fn resample_clamps_overshoot() {
        let data = vec![2.0f32, -2.0]; // 超出 [-1, 1]
        let out = resample_to_16k_mono_i16(&data, 1, 16_000);
        assert_eq!(out[0], 32767);
        assert_eq!(out[1], -32767);
    }

    #[test]
    fn discard_finish_mode_removes_retained_audio_without_conversion() {
        let dir = std::env::temp_dir().join(format!("shuohua-recorder-{}", ulid::Ulid::new()));
        let output =
            crate::voice::audio::prepare_in_dir(&dir, "discard", RecordAudioMode::Lossless)
                .unwrap()
                .unwrap();
        std::fs::write(&output.wav_path, b"invalid wav").unwrap();
        let wav_path = output.wav_path.clone();

        let result = complete_audio(FinishMode::Discard, Ok(()), Some(output)).unwrap();

        assert!(result.is_none());
        assert!(!wav_path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "opens the default Windows input device; run only during Windows audio runtime smoke"]
    fn windows_input_stream_runtime_smoke_receives_pcm_chunks() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("test runtime");

        runtime.block_on(async {
            let mut recording = start(None).expect("start default input stream");
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            let mut chunk_count = 0usize;
            let mut sample_count = 0usize;
            let mut peak = 0i16;

            while tokio::time::Instant::now() < deadline && chunk_count < 3 {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                match tokio::time::timeout(remaining, recording.recv()).await {
                    Ok(Some(chunk)) => {
                        chunk_count += 1;
                        sample_count += chunk.len();
                        let chunk_peak = chunk
                            .iter()
                            .map(|sample| sample.saturating_abs())
                            .max()
                            .unwrap_or(0);
                        peak = peak.max(chunk_peak);
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            recording.discard_audio().await.expect("discard audio");

            assert!(
                chunk_count > 0 && sample_count > 0,
                "expected PCM chunks from default Windows input device"
            );

            if std::env::var_os("SHUOHUA_WINDOWS_AUDIO_REQUIRE_SIGNAL").is_some() {
                assert!(
                    peak > 256,
                    "expected non-silent microphone signal, peak={peak}; speak near the microphone or disable SHUOHUA_WINDOWS_AUDIO_REQUIRE_SIGNAL"
                );
            }
        });
    }
}
