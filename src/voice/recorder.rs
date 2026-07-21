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
use rubato::{Fft, FixedSync, Indexing, Resampler, WindowFunction};
use std::path::{Path, PathBuf};
use tokio::sync::{mpsc, oneshot};

use crate::config::VoicePreprocessBackend;
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

#[derive(Debug, Clone)]
pub struct InputDeviceInfo {
    pub name: Option<String>,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: SampleFormat,
}

pub fn probe_default_input() -> Result<InputDeviceInfo> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("no default input device"))?;
    let name = Some(device.to_string());
    let supported = device
        .default_input_config()
        .context("query default input config")?;
    validate_supported_config(&supported)?;
    Ok(InputDeviceInfo {
        name,
        sample_rate: supported.sample_rate(),
        channels: supported.channels(),
        sample_format: supported.sample_format(),
    })
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
pub fn start(
    audio_output: Option<AudioOutput>,
    backend: VoicePreprocessBackend,
    channel_probe: bool,
) -> Result<RecordingStream> {
    let (pcm_tx, pcm_rx) = mpsc::unbounded_channel::<Vec<i16>>();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<FinishMode>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
    let (audio_result_tx, audio_result_rx) = oneshot::channel();

    std::thread::Builder::new()
        .name("cpal-recorder".into())
        .spawn(move || {
            if let Err(e) = run_recorder(
                pcm_tx,
                stop_rx,
                audio_output,
                backend,
                channel_probe,
                ready_tx,
                audio_result_tx,
            ) {
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
    backend: VoicePreprocessBackend,
    channel_probe: bool,
    ready_tx: std::sync::mpsc::Sender<Result<()>>,
    audio_result_tx: oneshot::Sender<Result<Option<PathBuf>>>,
) -> Result<()> {
    let audio_wav_path = audio_output.as_ref().map(|output| output.wav_path.clone());
    let startup = build_recorder_stream(audio_wav_path, channel_probe);
    match startup {
        Ok((stream, audio_rx, wav, src_rate)) => {
            let mut processor = match PcmProcessor::new(src_rate, backend, pcm_tx, wav) {
                Ok(processor) => processor,
                Err(error) => {
                    let msg = format!("{error:#}");
                    drop(stream);
                    if let Some(output) = audio_output {
                        output.discard();
                    }
                    let _ = audio_result_tx.send(Err(anyhow!(msg.clone())));
                    let _ = ready_tx.send(Err(anyhow!(msg)));
                    return Err(error);
                }
            };
            let _ = ready_tx.send(Ok(()));
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

fn build_recorder_stream(
    audio_wav_path: Option<PathBuf>,
    channel_probe: bool,
) -> Result<RecorderStreamSetup> {
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
        SampleFormat::F32 => {
            build_input_stream::<f32>(&device, config, channels, channel_probe, audio_tx)
        }
        SampleFormat::I16 => {
            build_input_stream::<i16>(&device, config, channels, channel_probe, audio_tx)
        }
        SampleFormat::U16 => {
            build_input_stream::<u16>(&device, config, channels, channel_probe, audio_tx)
        }
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
    channel_probe: bool,
    audio_tx: std::sync::mpsc::Sender<Vec<f32>>,
) -> Result<cpal::Stream>
where
    T: InputSample + SizedSample,
{
    #[cfg(feature = "dev")]
    let mut probe = (channel_probe && channels > 1).then(|| ChannelProbe::new(channels));
    #[cfg(not(feature = "dev"))]
    let _ = channel_probe;
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            #[cfg(feature = "dev")]
            if let Some(probe) = probe.as_mut() {
                probe.observe(data);
            }
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

/// 多通道 → mono：所有通道求平均，而不是只取 `frame[0]`。
///
/// 单通道走快路径。多通道取均值是最稳妥的设备无关策略：普通立体声 mic 正常混音；
/// 遇到 default input 变成多通道 aggregate（例如残留的 voice-processing aggregate，
/// 通道 0 可能是近静音的参考通道）时，真正有语音的通道仍会进入 mono，不会像
/// 只取 `frame[0]` 那样把整段人声丢成静音。均值会让信号幅度按通道数缩小，但
/// VAD/ASR 对电平不敏感，远好过丢信号。
fn input_to_mono<T: InputSample>(data: &[T], channels: usize) -> Vec<f32> {
    if channels == 0 || data.is_empty() {
        return Vec::new();
    }
    if channels == 1 {
        return data.iter().map(|sample| sample.to_f32()).collect();
    }
    let inv = 1.0 / channels as f32;
    data.chunks_exact(channels)
        .map(|frame| frame.iter().map(|sample| sample.to_f32()).sum::<f32>() * inv)
        .collect()
}

/// dev-only 多通道探针（`dev.apple_backend_trace`）：`channels > 1` 时统计启动
/// 前若干帧的 per-channel RMS/peak，一次性打日志。用来确认哪个通道有信号——
/// 例如 VP aggregate 残留导致 default input 变成 3ch、mono 取的 `frame[0]` 恰好
/// 是近静音的参考通道时，这里能立刻看出真正有语音的通道。
#[cfg(feature = "dev")]
struct ChannelProbe {
    channels: usize,
    sum_sq: Vec<f64>,
    peak: Vec<f32>,
    frames: usize,
    logged: bool,
}

#[cfg(feature = "dev")]
impl ChannelProbe {
    const PROBE_FRAMES: usize = 8_192;

    fn new(channels: usize) -> Self {
        Self {
            channels,
            sum_sq: vec![0.0; channels],
            peak: vec![0.0; channels],
            frames: 0,
            logged: false,
        }
    }

    fn observe<T: InputSample>(&mut self, data: &[T]) {
        if self.logged {
            return;
        }
        for frame in data.chunks_exact(self.channels) {
            for (ch, sample) in frame.iter().enumerate() {
                let value = sample.to_f32();
                self.sum_sq[ch] += (value as f64) * (value as f64);
                let magnitude = value.abs();
                if magnitude > self.peak[ch] {
                    self.peak[ch] = magnitude;
                }
            }
            self.frames += 1;
        }
        if self.frames >= Self::PROBE_FRAMES {
            let rms: Vec<f32> = self
                .sum_sq
                .iter()
                .map(|sum| (sum / self.frames as f64).sqrt() as f32)
                .collect();
            tracing::info!(
                channels = self.channels,
                frames = self.frames,
                per_channel_rms = ?rms,
                per_channel_peak = ?self.peak,
                "cpal multi-channel input probe"
            );
            self.logged = true;
        }
    }
}

struct PcmProcessor {
    resampler: Resampler16k,
    preprocess: PreprocessStage,
    pcm_tx: mpsc::UnboundedSender<Vec<i16>>,
    wav: Option<WavWriter>,
    preprocess_error: Option<anyhow::Error>,
    resample_error: Option<anyhow::Error>,
    audio_error: Option<anyhow::Error>,
}

impl PcmProcessor {
    fn new(
        src_rate: u32,
        backend: VoicePreprocessBackend,
        pcm_tx: mpsc::UnboundedSender<Vec<i16>>,
        wav: Option<WavWriter>,
    ) -> Result<Self> {
        let preprocess = PreprocessStage::new(backend, src_rate)?;
        let resample_src_rate = preprocess.output_sample_rate(src_rate);
        Ok(Self {
            resampler: Resampler16k::new(resample_src_rate),
            preprocess,
            pcm_tx,
            wav,
            preprocess_error: None,
            resample_error: None,
            audio_error: None,
        })
    }

    fn process_mono_chunk(&mut self, mono: &[f32]) {
        if self.preprocess_error.is_some() || self.resample_error.is_some() {
            return;
        }
        let processed = match self.preprocess.process_mono_chunk(mono) {
            Ok(processed) => processed,
            Err(error) => {
                self.preprocess_error = Some(error);
                return;
            }
        };
        match self.resampler.process(&processed) {
            Ok(pcm) => self.publish_pcm(&pcm),
            Err(error) => self.resample_error = Some(error),
        }
    }

    fn finish(&mut self, finish_mode: FinishMode) -> Result<()> {
        if finish_mode == FinishMode::Discard {
            let _ = self.wav.take();
            return Ok(());
        }

        if self.preprocess_error.is_none() && self.resample_error.is_none() {
            match self.preprocess.finish() {
                Ok(processed) => match self.resampler.process(&processed) {
                    Ok(pcm) => self.publish_pcm(&pcm),
                    Err(error) => self.resample_error = Some(error),
                },
                Err(error) => self.preprocess_error = Some(error),
            }
        }

        if self.preprocess_error.is_none() && self.resample_error.is_none() {
            match self.resampler.finish() {
                Ok(pcm) => self.publish_pcm(&pcm),
                Err(error) => self.resample_error = Some(error),
            }
        }

        if let Some(wav) = self.wav.take() {
            wav.finalize().context("finalize wav")?;
        }

        if let Some(error) = self.preprocess_error.take() {
            return Err(error);
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

enum PreprocessStage {
    Off,
    WebRtc(Box<crate::voice::webrtc_apm::WebRtcPreprocessor>),
}

impl PreprocessStage {
    fn new(backend: VoicePreprocessBackend, src_rate: u32) -> Result<Self> {
        match backend {
            VoicePreprocessBackend::Off => Ok(Self::Off),
            VoicePreprocessBackend::WebRtc => {
                crate::voice::webrtc_apm::WebRtcPreprocessor::new(src_rate)
                    .map(Box::new)
                    .map(Self::WebRtc)
            }
            VoicePreprocessBackend::Apple => {
                bail!("voice preprocess backend \"apple\" is not available in cpal recorder")
            }
        }
    }

    fn output_sample_rate(&self, src_rate: u32) -> u32 {
        match self {
            Self::Off => src_rate,
            Self::WebRtc(processor) => processor.output_sample_rate(),
        }
    }

    fn process_mono_chunk(&mut self, mono: &[f32]) -> Result<Vec<f32>> {
        match self {
            Self::Off => Ok(mono.to_vec()),
            Self::WebRtc(processor) => processor.process_mono_chunk(mono),
        }
    }

    fn finish(&mut self) -> Result<Vec<f32>> {
        match self {
            Self::Off => Ok(Vec::new()),
            Self::WebRtc(processor) => processor.finish(),
        }
    }
}

pub(crate) struct ResamplerForPreprocess {
    inner: ResamplerF32,
}

impl ResamplerForPreprocess {
    pub(crate) fn new(src_rate: u32, dst_rate: u32) -> Self {
        Self {
            inner: ResamplerF32::new(src_rate, dst_rate),
        }
    }

    pub(crate) fn process(&mut self, mono: &[f32]) -> Result<Vec<f32>> {
        self.inner.process(mono)
    }

    pub(crate) fn finish(&mut self) -> Result<Vec<f32>> {
        self.inner.finish()
    }
}

enum Resampler16k {
    Passthrough,
    Rubato(Box<ResamplerF32>),
}

impl Resampler16k {
    fn new(src_rate: u32) -> Self {
        if src_rate == DST_RATE_HZ {
            Self::Passthrough
        } else {
            Self::Rubato(Box::new(ResamplerF32::new(src_rate, DST_RATE_HZ)))
        }
    }

    fn process(&mut self, mono: &[f32]) -> Result<Vec<i16>> {
        match self {
            Self::Passthrough => Ok(mono_to_i16(mono)),
            Self::Rubato(resampler) => Ok(mono_to_i16(&resampler.process(mono)?)),
        }
    }

    fn finish(&mut self) -> Result<Vec<i16>> {
        match self {
            Self::Passthrough => Ok(Vec::new()),
            Self::Rubato(resampler) => Ok(mono_to_i16(&resampler.finish()?)),
        }
    }
}

struct ResamplerF32 {
    src_rate: u32,
    dst_rate: u32,
    inner: Fft<f32>,
    pending: Vec<f32>,
    output_buf: Vec<f32>,
    input_frames: usize,
    output_frames: usize,
    trim_output_frames: usize,
}

impl ResamplerF32 {
    fn new(src_rate: u32, dst_rate: u32) -> Self {
        let inner = Fft::<f32>::new_custom(
            src_rate as usize,
            dst_rate as usize,
            1024,
            2,
            1,
            WindowFunction::BlackmanHarris2,
            FixedSync::Input,
        )
        .expect("valid recorder sample rates");
        let output_buf = vec![0.0; inner.output_frames_max()];
        let trim_output_frames = inner.output_delay();
        Self {
            src_rate,
            dst_rate,
            inner,
            pending: Vec::new(),
            output_buf,
            input_frames: 0,
            output_frames: 0,
            trim_output_frames,
        }
    }

    fn process(&mut self, mono: &[f32]) -> Result<Vec<f32>> {
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

    fn finish(&mut self) -> Result<Vec<f32>> {
        let mut output = Vec::new();
        let mut pending_output = None;
        if !self.pending.is_empty() {
            let partial_len = self.pending.len();
            self.pending.resize(self.inner.input_frames_next(), 0.0);
            let (used, produced) = self.process_next(Some(partial_len))?;
            self.pending.drain(..used.min(self.pending.len()));
            pending_output = Some(produced);
        }

        let target_frames = ((self.input_frames as f64 * self.dst_rate as f64)
            / self.src_rate as f64)
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

    fn append_output(&mut self, output: &mut Vec<f32>, produced: usize, cap: Option<usize>) {
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

        output.extend_from_slice(&self.output_buf[start..end]);
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
    let mono = input_to_mono(data, channels);
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
    fn resample_averages_channels_for_stereo() {
        // 2ch interleaved：帧内两通道求均值。L=0.5,R=0.3 → 0.4
        let data = vec![0.5, 0.3, 0.5, 0.3];
        let out = resample_to_16k_mono_i16(&data, 2, 16_000);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], out[1]);
        assert_eq!(out[0], (0.4_f32 * 32767.0).round() as i16);
    }

    #[test]
    fn downmix_preserves_signal_when_only_non_first_channel_is_loud() {
        // 3ch aggregate 场景：通道 0（参考通道）静音，语音在通道 2。
        // 取 frame[0] 会得到静音；均值保留信号（1/3 幅度）。
        let data = vec![0.0, 0.0, 0.9, 0.0, 0.0, 0.9];
        let out = resample_to_16k_mono_i16(&data, 3, 16_000);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], (0.3_f32 * 32767.0).round() as i16);
        assert!(
            out[0] > 8,
            "downmixed signal must clear the VAD noise floor"
        );
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
        let dir = std::env::temp_dir().join(format!("shuohua-recorder-{}", ulid::Ulid::generate()));
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
}
