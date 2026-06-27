use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::RecordAudioMode;

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
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for sample in [0i16; 1_600] {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
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
