//! 流式 cpal 录音：default input → linear resample → 16k mono s16le 帧 → mpsc。
//!
//! 一次录音一个 [`RecordingStream`]，背后是一个专用 std 线程跑 cpal stream
//! (cpal::Stream 是 !Send，不能跨线程移动)。callback 把 PCM 帧 push 到 tokio
//! unbounded mpsc，async 端按需 recv。可选 wav 留存在同一线程里写。
//!
//! Stop 协议：voice 端调 stop()（drop oneshot sender 等价语义），cpal 线程收到
//! 信号后 drop stream 并 finalize wav。drop stream 时 cpal 自动 drain
//! callback in-flight 的 buffer。
//!
//! 故意保留 M1 的 linear resample：质量对识别足够好，DESIGN §2.9 的 rubato
//! 升级留给"识别质量真出问题"那天。

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SupportedStreamConfig};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

const DST_RATE_HZ: u32 = 16_000;

pub struct RecordingStream {
    pcm_rx: mpsc::UnboundedReceiver<Vec<i16>>,
    /// drop 这边就告诉 cpal 线程退出。
    stop: Option<std::sync::mpsc::Sender<()>>,
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
        let _ = self.stop.take();
    }

    /// 非 await 的 try_recv（finishing 阶段一次性吸干残余帧用）。
    pub fn try_recv(&mut self) -> Option<Vec<i16>> {
        match self.pcm_rx.try_recv() {
            Ok(v) => Some(v),
            Err(_) => None,
        }
    }
}

impl Drop for RecordingStream {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 启动一路录音。`audio_wav_path = Some(path)` 时把同一份 PCM 留存到 wav；
/// path 父目录必须已存在（caller 负责 mkdir）。
pub fn start(audio_wav_path: Option<PathBuf>) -> Result<RecordingStream> {
    let (pcm_tx, pcm_rx) = mpsc::unbounded_channel::<Vec<i16>>();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();

    std::thread::Builder::new()
        .name("cpal-recorder".into())
        .spawn(move || {
            if let Err(e) = run_recorder(pcm_tx, stop_rx, audio_wav_path, ready_tx) {
                eprintln!("[recorder] {e:#}");
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
    })
}

fn run_recorder(
    pcm_tx: mpsc::UnboundedSender<Vec<i16>>,
    stop_rx: std::sync::mpsc::Receiver<()>,
    audio_wav_path: Option<PathBuf>,
    ready_tx: std::sync::mpsc::Sender<Result<()>>,
) -> Result<()> {
    let startup = build_recorder_stream(pcm_tx, audio_wav_path);
    match startup {
        Ok((stream, wav)) => {
            let _ = ready_tx.send(Ok(()));
            // 阻塞等 stop_tx 发信号或被 drop。cpal callback 同时在另一个 cpal 内部线程跑。
            let _ = stop_rx.recv();
            drop(stream); // cpal drain & close

            if let Ok(mut guard) = wav.lock() {
                if let Some(w) = guard.take() {
                    w.finalize().context("finalize wav")?;
                }
            }
            Ok(())
        }
        Err(e) => {
            let msg = format!("{e:#}");
            let _ = ready_tx.send(Err(anyhow!(msg)));
            Err(e)
        }
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
    crate::debug_println!("[recorder] {src_rate}Hz × {channels}ch F32 → 16k mono s16le");

    let wav = Arc::new(Mutex::new(open_wav(audio_wav_path.as_deref())?));
    let wav_cb = wav.clone();

    let config = supported.into();
    let stream = device
        .build_input_stream(
            config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let pcm = resample_to_16k_mono_i16(data, channels, src_rate);
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
            |err| eprintln!("[recorder] stream error: {err}"),
            None,
        )
        .context("build input stream")?;

    stream.play().context("start input stream")?;
    Ok((stream, wav))
}

fn validate_supported_config(supported: &SupportedStreamConfig) -> Result<()> {
    let sample_format = supported.sample_format();
    if sample_format != SampleFormat::F32 {
        bail!("recorder requires F32 input, got {sample_format:?}");
    }
    if supported.channels() == 0 {
        bail!("default input device reports 0 channels");
    }
    Ok(())
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
    fn resample_clamps_overshoot() {
        let data = vec![2.0f32, -2.0]; // 超出 [-1, 1]
        let out = resample_to_16k_mono_i16(&data, 1, 16_000);
        assert_eq!(out[0], 32767);
        assert_eq!(out[1], -32767);
    }
}
