//! M1 recorder: cpal default input → linear resample to 16k mono → hound WAV.
//!
//! Canonical internal PCM format is 16kHz s16le mono (docs/DESIGN.md §2.9).
//! Conversion lives here in one place; ASR providers see canonical input.
//!
//! Intentional M1 limitations (will revisit at M2):
//!   * Linear interpolation with no anti-alias prefilter. Aliased highs are
//!     audible but harmless for QuickTime playback and likely fine for ASR;
//!     swap in a windowed-sinc / `rubato` if recognition quality suffers.
//!   * F32 input only — bail loudly on rare devices that return I16/U16.
//!     Apple Silicon + Intel macs deliver F32 from the default mic on
//!     current macOS, so no real-world coverage gap.

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DST_RATE_HZ: u32 = 16_000;

pub fn record_to_wav(out_path: &Path, secs: f64) -> Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("no default input device"))?;
    let device_name = device
        .description()
        .ok()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|| "<unknown>".into());

    let supported = device
        .default_input_config()
        .context("query default input config")?;
    let src_rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();

    eprintln!(
        "[recorder] device={device_name} rate={src_rate}Hz ch={channels} fmt={sample_format:?}"
    );

    if sample_format != SampleFormat::F32 {
        bail!(
            "M1 recorder only handles F32 input, got {:?}. Add I16/U16 branches \
             in voice/recorder.rs when needed.",
            sample_format
        );
    }

    let config: StreamConfig = supported.into();
    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::with_capacity(
        (src_rate as f64 * secs * channels as f64) as usize,
    )));
    let samples_cb = samples.clone();

    let stream = device
        .build_input_stream(
            config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let mut buf = samples_cb.lock().unwrap();
                // mono = first channel; multi-channel summing is overkill at M1
                for frame in data.chunks_exact(channels) {
                    buf.push(frame[0]);
                }
            },
            |err| eprintln!("[recorder] stream error: {err}"),
            None,
        )
        .context("build input stream")?;

    stream.play().context("start input stream")?;
    std::thread::sleep(Duration::from_secs_f64(secs));
    drop(stream);

    let mono = std::mem::take(&mut *samples.lock().unwrap());
    let resampled = linear_resample(&mono, src_rate, DST_RATE_HZ);
    write_wav_s16le(out_path, &resampled, DST_RATE_HZ)?;
    Ok(())
}

fn linear_resample(input: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if src_rate == dst_rate || input.is_empty() {
        return input.to_vec();
    }
    let ratio = src_rate as f64 / dst_rate as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = (src_pos - idx as f64) as f32;
        let s = if idx + 1 < input.len() {
            input[idx] * (1.0 - frac) + input[idx + 1] * frac
        } else {
            input[idx]
        };
        out.push(s);
    }
    out
}

fn write_wav_s16le(path: &Path, samples: &[f32], rate: u32) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav = hound::WavWriter::create(path, spec).context("create wav")?;
    for &s in samples {
        let clipped = s.clamp(-1.0, 1.0);
        let q = (clipped * 32767.0).round() as i16;
        wav.write_sample(q)?;
    }
    wav.finalize().context("finalize wav")?;
    Ok(())
}
