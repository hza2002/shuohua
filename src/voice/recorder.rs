//! 流式 cpal 录音：default input → linear resample → 16k mono s16le 帧 → mpsc。
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
//! 这里保留简单的 linear resample：质量对识别足够好，后续只有在真实识别质量
//! 出问题时才需要升级重采样算法。

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, SupportedStreamConfig};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

use crate::voice::audio::AudioOutput;

const DST_RATE_HZ: u32 = 16_000;

pub struct RecordingStream {
    pcm_rx: mpsc::UnboundedReceiver<Vec<i16>>,
    stop: Option<std::sync::mpsc::Sender<FinishMode>>,
    audio_result: Option<oneshot::Receiver<Result<Option<PathBuf>>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinishMode {
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

    fn request_stop(&mut self, mode: FinishMode) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(mode);
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
    let startup = build_recorder_stream(pcm_tx, audio_wav_path);
    match startup {
        Ok((stream, wav)) => {
            let _ = ready_tx.send(Ok(()));
            // 阻塞等 stop 信号。sender 意外被 drop 时按丢弃处理，避免发布半成品。
            let finish_mode = stop_rx.recv().unwrap_or(FinishMode::Discard);
            drop(stream); // cpal drain & close

            let finalize_result = if let Ok(mut guard) = wav.lock() {
                match (finish_mode, guard.take()) {
                    (FinishMode::Publish, Some(w)) => w.finalize().context("finalize wav"),
                    (_, Some(w)) => {
                        drop(w);
                        Ok(())
                    }
                    (_, None) => Ok(()),
                }
            } else {
                Err(anyhow!("retained audio writer lock poisoned"))
            };
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
type SharedWav = Arc<Mutex<Option<WavWriter>>>;

fn build_recorder_stream(
    pcm_tx: mpsc::UnboundedSender<Vec<i16>>,
    audio_wav_path: Option<PathBuf>,
) -> Result<(cpal::Stream, SharedWav)> {
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

    let wav = Arc::new(Mutex::new(open_wav(audio_wav_path.as_deref())?));
    let wav_cb = wav.clone();

    let sample_format = supported.sample_format();
    let config = supported.into();
    let stream = match sample_format {
        SampleFormat::F32 => {
            build_input_stream::<f32>(&device, config, channels, src_rate, pcm_tx, wav_cb)
        }
        SampleFormat::I16 => {
            build_input_stream::<i16>(&device, config, channels, src_rate, pcm_tx, wav_cb)
        }
        SampleFormat::U16 => {
            build_input_stream::<u16>(&device, config, channels, src_rate, pcm_tx, wav_cb)
        }
        other => bail!("recorder requires F32/I16/U16 input, got {other:?}"),
    }
    .context("build input stream")?;

    stream.play().context("start input stream")?;
    Ok((stream, wav))
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
    src_rate: u32,
    pcm_tx: mpsc::UnboundedSender<Vec<i16>>,
    wav_cb: SharedWav,
) -> Result<cpal::Stream>
where
    T: InputSample + SizedSample,
{
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let pcm = resample_input_to_16k_mono_i16(data, channels, src_rate);
            if pcm.is_empty() {
                return;
            }
            if let Ok(mut guard) = wav_cb.lock() {
                if let Some(w) = guard.as_mut() {
                    for &s in &pcm {
                        let _ = w.write_sample(s);
                    }
                }
            }
            // unbounded send 不会阻塞 cpal callback。voice 消费滞后只是
            // 增加内存占用，不会丢帧。
            let _ = pcm_tx.send(pcm);
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

fn resample_input_to_16k_mono_i16<T: InputSample>(
    data: &[T],
    channels: usize,
    src_rate: u32,
) -> Vec<i16> {
    if channels == 0 || data.is_empty() {
        return Vec::new();
    }
    let mono: Vec<f32> = data
        .chunks_exact(channels)
        .map(|frame| frame[0].to_f32())
        .collect();
    resample_to_16k_mono_i16(&mono, 1, src_rate)
}

fn resample_to_16k_mono_i16(data: &[f32], channels: usize, src_rate: u32) -> Vec<i16> {
    if channels == 0 || data.is_empty() {
        return Vec::new();
    }
    // 单声道：取第一通道（多通道求平均纯属感官清晰度提升，对识别无收益）
    let mono: Vec<f32> = data.chunks_exact(channels).map(|f| f[0]).collect();
    if mono.is_empty() {
        return Vec::new();
    }
    if src_rate == DST_RATE_HZ {
        return mono
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16)
            .collect();
    }
    // 线性插值。每 cpal callback 内重置位置，相邻 callback 间有可忽略相位跳动。
    let ratio = src_rate as f64 / DST_RATE_HZ as f64;
    let out_len = (mono.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = (src_pos - idx as f64) as f32;
        let s = if idx + 1 < mono.len() {
            mono[idx] * (1.0 - frac) + mono[idx + 1] * frac
        } else {
            mono[idx]
        };
        out.push((s.clamp(-1.0, 1.0) * 32767.0).round() as i16);
    }
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
    fn resample_takes_first_channel_for_stereo() {
        // 2ch interleaved；左声道 = 0.5，右声道 = -0.5
        let data = vec![0.5, -0.5, 0.5, -0.5];
        let out = resample_to_16k_mono_i16(&data, 2, 16_000);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], out[1]);
        assert!(out[0] > 0); // 左声道是正值
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
}
