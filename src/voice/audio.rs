use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::RecordAudioMode;

const RETAINED_AUDIO_TARGET_RMS: f32 = 0.12;
const RETAINED_AUDIO_TARGET_PEAK: f32 = 0.89;
const RETAINED_AUDIO_MIN_PEAK_TO_NORMALIZE: u16 = 32;
const RETAINED_AUDIO_ACTIVE_SAMPLE_FLOOR: u16 = 16;
const RETAINED_AUDIO_MAX_GAIN: f32 = 64.0;

#[derive(Debug)]
pub(crate) struct AudioOutput {
    pub(crate) wav_path: PathBuf,
    mode: RecordAudioMode,
    temp_path: PathBuf,
    final_path: PathBuf,
}

pub(crate) fn prepare(recording_id: &str, mode: RecordAudioMode) -> Result<Option<AudioOutput>> {
    prepare_in_dir(
        &crate::paths::StateDirs::discover().audio(),
        recording_id,
        mode,
    )
}

pub(crate) fn prepare_in_dir(
    base: &Path,
    recording_id: &str,
    mode: RecordAudioMode,
) -> Result<Option<AudioOutput>> {
    if mode == RecordAudioMode::Off {
        return Ok(None);
    }
    std::fs::create_dir_all(base)
        .with_context(|| format!("create retained audio dir {}", base.display()))?;
    let extension = match mode {
        RecordAudioMode::Off => unreachable!(),
        RecordAudioMode::Lossless => "flac",
        RecordAudioMode::Compact => "m4a",
    };
    Ok(Some(AudioOutput {
        wav_path: base.join(format!("{recording_id}.tmp.wav")),
        mode,
        temp_path: base.join(format!("{recording_id}.tmp.{extension}")),
        final_path: base.join(format!("{recording_id}.{extension}")),
    }))
}

impl AudioOutput {
    pub(crate) fn finish(self) -> Result<PathBuf> {
        self.finish_with(crate::platform::audio_convert::convert_retained_audio)
    }

    fn finish_with(
        self,
        convert: impl FnOnce(RecordAudioMode, &Path, &Path) -> Result<()>,
    ) -> Result<PathBuf> {
        let mut published = false;
        let result = (|| -> Result<PathBuf> {
            normalize_retained_wav_loudness(&self.wav_path)?;
            convert(self.mode, &self.wav_path, &self.temp_path)?;
            std::fs::rename(&self.temp_path, &self.final_path).with_context(|| {
                format!(
                    "publish retained audio {} -> {}",
                    self.temp_path.display(),
                    self.final_path.display()
                )
            })?;
            published = true;
            std::fs::remove_file(&self.wav_path)
                .with_context(|| format!("remove temporary wav {}", self.wav_path.display()))?;
            Ok(self.final_path.clone())
        })();

        if result.is_err() {
            remove_if_exists(&self.temp_path);
            remove_if_exists(&self.wav_path);
            if published {
                remove_if_exists(&self.final_path);
            }
        }
        result
    }

    pub(crate) fn discard(self) {
        remove_if_exists(&self.temp_path);
        remove_if_exists(&self.wav_path);
        remove_if_exists(&self.final_path);
    }
}

fn normalize_retained_wav_loudness(path: &Path) -> Result<()> {
    let (spec, samples) = read_pcm_i16_wav(path)?;
    let metrics = AudioMetrics::from_samples(&samples);
    if metrics.peak < RETAINED_AUDIO_MIN_PEAK_TO_NORMALIZE
        || metrics.rms >= RETAINED_AUDIO_TARGET_RMS
    {
        return Ok(());
    }

    let rms_gain = RETAINED_AUDIO_TARGET_RMS / metrics.rms;
    let peak_gain = RETAINED_AUDIO_TARGET_PEAK / metrics.peak_normalized();
    let gain = rms_gain.min(peak_gain).min(RETAINED_AUDIO_MAX_GAIN);
    if gain <= 1.0 {
        return Ok(());
    }
    let normalized = samples
        .into_iter()
        .map(|sample| apply_gain(sample, gain))
        .collect::<Vec<_>>();
    write_pcm_i16_wav(path, spec, &normalized)
}

#[derive(Debug, Clone, Copy)]
struct AudioMetrics {
    peak: u16,
    rms: f32,
}

impl AudioMetrics {
    fn from_samples(samples: &[i16]) -> Self {
        let mut peak = 0u16;
        let mut active_count = 0usize;
        let mut square_sum = 0.0f32;

        for sample in samples {
            let abs = sample.unsigned_abs();
            peak = peak.max(abs);
            if abs >= RETAINED_AUDIO_ACTIVE_SAMPLE_FLOOR {
                let normalized = *sample as f32 / i16::MAX as f32;
                square_sum += normalized * normalized;
                active_count += 1;
            }
        }

        let rms = if active_count == 0 {
            0.0
        } else {
            (square_sum / active_count as f32).sqrt()
        };
        Self { peak, rms }
    }

    fn peak_normalized(self) -> f32 {
        self.peak as f32 / i16::MAX as f32
    }
}

fn read_pcm_i16_wav(path: &Path) -> Result<(hound::WavSpec, Vec<i16>)> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("open WAV {}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
        anyhow::bail!(
            "retained audio normalization only supports 16-bit PCM WAV input, got {:?}/{}bit",
            spec.sample_format,
            spec.bits_per_sample
        );
    }
    let samples = reader
        .samples::<i16>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("read retained WAV PCM samples")?;
    Ok((spec, samples))
}

fn write_pcm_i16_wav(path: &Path, spec: hound::WavSpec, samples: &[i16]) -> Result<()> {
    let temp_path = path.with_extension("normalized.tmp.wav");
    let mut writer = hound::WavWriter::create(&temp_path, spec)
        .with_context(|| format!("create normalized WAV {}", temp_path.display()))?;
    for &sample in samples {
        writer
            .write_sample(sample)
            .with_context(|| format!("write normalized WAV {}", temp_path.display()))?;
    }
    writer
        .finalize()
        .with_context(|| format!("finalize normalized WAV {}", temp_path.display()))?;
    std::fs::rename(&temp_path, path).with_context(|| {
        format!(
            "replace retained WAV {} -> {}",
            temp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn apply_gain(sample: i16, gain: f32) -> i16 {
    (sample as f32 * gain)
        .round()
        .clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

fn remove_if_exists(path: &Path) {
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(path = %path.display(), error = %error, "remove temporary audio failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_wav(path: &Path) {
        write_test_wav_with_samples(path, &[0i16; 1_600]);
    }

    fn write_test_wav_with_samples(path: &Path, samples: &[i16]) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for &sample in samples {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn read_test_wav_samples(path: &Path) -> Vec<i16> {
        hound::WavReader::open(path)
            .unwrap()
            .samples::<i16>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn retained_wav_normalization_amplifies_quiet_recordings_before_conversion() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("quiet.wav");
        write_test_wav_with_samples(&wav, &[1000, -1000, 500, -500]);

        normalize_retained_wav_loudness(&wav).unwrap();

        let samples = read_test_wav_samples(&wav);
        let metrics = AudioMetrics::from_samples(&samples);
        assert!(metrics.rms > 0.10, "rms={}", metrics.rms);
        assert!(metrics.peak_normalized() <= RETAINED_AUDIO_TARGET_PEAK + 0.001);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn retained_wav_normalization_amplifies_very_low_mic_level() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("low-mic.wav");
        write_test_wav_with_samples(&wav, &[120, -120, 80, -80, 0, 64, -64]);

        normalize_retained_wav_loudness(&wav).unwrap();

        let samples = read_test_wav_samples(&wav);
        let peak = samples
            .iter()
            .map(|sample| sample.unsigned_abs())
            .max()
            .unwrap();
        assert!(peak > 5000, "peak={peak}");
        assert!(AudioMetrics::from_samples(&samples).rms > 0.10);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn retained_wav_normalization_leaves_tiny_noise_unchanged() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("noise.wav");
        write_test_wav_with_samples(&wav, &[12, -12, 8, -8]);

        normalize_retained_wav_loudness(&wav).unwrap();

        assert_eq!(read_test_wav_samples(&wav), vec![12, -12, 8, -8]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn finish_normalizes_retained_wav_before_conversion() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-{}", ulid::Ulid::new()));
        let output = prepare_in_dir(&dir, "01HXYZ", RecordAudioMode::Lossless)
            .unwrap()
            .unwrap();
        write_test_wav_with_samples(&output.wav_path, &[1000, -1000, 500, -500]);
        let final_path = output.final_path.clone();

        let finished = output
            .finish_with(|_, input, output| {
                let samples = read_test_wav_samples(input);
                assert!(
                    AudioMetrics::from_samples(&samples).peak_normalized()
                        <= RETAINED_AUDIO_TARGET_PEAK + 0.001
                );
                std::fs::write(output, b"converted").unwrap();
                Ok(())
            })
            .unwrap();

        assert_eq!(finished, final_path);
        assert_eq!(std::fs::read(&finished).unwrap(), b"converted");
        assert!(!dir.join("01HXYZ.tmp.wav").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn off_mode_prepares_no_output() {
        let output = prepare_in_dir(
            Path::new("/tmp/shuohua-audio"),
            "01HXYZ",
            RecordAudioMode::Off,
        )
        .unwrap();

        assert!(output.is_none());
    }

    #[test]
    fn lossless_paths_use_flac_final_and_temporary_wav() {
        let output = prepare_in_dir(
            Path::new("/tmp/shuohua-audio"),
            "01HXYZ",
            RecordAudioMode::Lossless,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            output.wav_path,
            PathBuf::from("/tmp/shuohua-audio/01HXYZ.tmp.wav")
        );
        assert_eq!(
            output.temp_path,
            PathBuf::from("/tmp/shuohua-audio/01HXYZ.tmp.flac")
        );
        assert_eq!(
            output.final_path,
            PathBuf::from("/tmp/shuohua-audio/01HXYZ.flac")
        );
    }

    #[test]
    fn compact_paths_use_m4a_final_and_temporary_wav() {
        let output = prepare_in_dir(
            Path::new("/tmp/shuohua-audio"),
            "01HXYZ",
            RecordAudioMode::Compact,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            output.temp_path,
            PathBuf::from("/tmp/shuohua-audio/01HXYZ.tmp.m4a")
        );
        assert_eq!(
            output.final_path,
            PathBuf::from("/tmp/shuohua-audio/01HXYZ.m4a")
        );
    }

    #[test]
    fn failed_conversion_removes_temporary_files() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-{}", ulid::Ulid::new()));
        let output = prepare_in_dir(&dir, "01HXYZ", RecordAudioMode::Lossless)
            .unwrap()
            .unwrap();
        write_test_wav(&output.wav_path);
        std::fs::write(&output.temp_path, b"partial").unwrap();
        let wav = output.wav_path.clone();
        let temp = output.temp_path.clone();

        assert!(output
            .finish_with(|mode, input, output| {
                crate::platform::audio_convert::convert_retained_audio_with_program(
                    "/usr/bin/false",
                    mode,
                    input,
                    output,
                )
            })
            .is_err());
        assert!(!wav.exists());
        assert!(!temp.exists());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn afconvert_creates_flac_and_removes_temporary_wav() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-{}", ulid::Ulid::new()));
        let output = prepare_in_dir(&dir, "01HXYZ", RecordAudioMode::Lossless)
            .unwrap()
            .unwrap();
        write_test_wav(&output.wav_path);
        let wav = output.wav_path.clone();

        let final_path = output.finish().unwrap();

        assert_eq!(final_path, dir.join("01HXYZ.flac"));
        assert!(final_path.is_file());
        assert!(!wav.exists());
        assert!(!dir.join("01HXYZ.tmp.flac").exists());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn afconvert_creates_compact_m4a_and_removes_temporary_wav() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-{}", ulid::Ulid::new()));
        let output = prepare_in_dir(&dir, "01HXYZ", RecordAudioMode::Compact)
            .unwrap()
            .unwrap();
        write_test_wav(&output.wav_path);
        let wav = output.wav_path.clone();

        let final_path = output.finish().unwrap();

        assert_eq!(final_path, dir.join("01HXYZ.m4a"));
        assert!(final_path.is_file());
        assert!(!wav.exists());
        assert!(!dir.join("01HXYZ.tmp.m4a").exists());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "windows")]
    #[ignore = "uses Windows Media Foundation; run only during Windows retained-audio runtime smoke"]
    #[test]
    fn native_compact_finish_creates_retained_audio_and_removes_temporary_wav() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-finish-{}", ulid::Ulid::new()));
        let output = prepare_in_dir(&dir, "01HXYZ", RecordAudioMode::Compact)
            .unwrap()
            .unwrap();
        write_test_wav(&output.wav_path);
        let wav = output.wav_path.clone();
        let temp = output.temp_path.clone();

        let final_path = output.finish().unwrap();

        assert_eq!(final_path, dir.join("01HXYZ.m4a"));
        assert!(final_path.is_file());
        assert!(std::fs::metadata(&final_path).unwrap().len() > 0);
        assert!(!wav.exists());
        assert!(!temp.exists());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "windows")]
    #[ignore = "uses pure Rust FLAC; run only during Windows retained-audio runtime smoke"]
    #[test]
    fn native_lossless_finish_creates_retained_audio_and_removes_temporary_wav() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-finish-{}", ulid::Ulid::new()));
        let output = prepare_in_dir(&dir, "01HXYZ", RecordAudioMode::Lossless)
            .unwrap()
            .unwrap();
        write_test_wav(&output.wav_path);
        let wav = output.wav_path.clone();
        let temp = output.temp_path.clone();

        let final_path = output.finish().unwrap();

        assert_eq!(final_path, dir.join("01HXYZ.flac"));
        assert!(final_path.is_file());
        assert!(std::fs::metadata(&final_path).unwrap().len() > 0);
        assert!(!wav.exists());
        assert!(!temp.exists());

        let _ = std::fs::remove_dir_all(dir);
    }
}
