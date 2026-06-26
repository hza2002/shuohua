use std::path::Path;

use anyhow::Result;

use crate::config::RecordAudioMode;

pub(crate) fn convert_retained_audio(
    mode: RecordAudioMode,
    input: &Path,
    output: &Path,
) -> Result<()> {
    imp::convert_retained_audio(mode, input, output)
}

#[cfg(test)]
pub(crate) fn convert_retained_audio_with_program(
    program: &str,
    mode: RecordAudioMode,
    input: &Path,
    output: &Path,
) -> Result<()> {
    imp::convert_retained_audio_with_program(program, mode, input, output)
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::OsString;
    use std::process::Command;

    use anyhow::{bail, Context, Result};

    use super::*;

    const AFCONVERT: &str = "/usr/bin/afconvert";

    pub(super) fn convert_retained_audio(
        mode: RecordAudioMode,
        input: &Path,
        output: &Path,
    ) -> Result<()> {
        convert_retained_audio_with_program(AFCONVERT, mode, input, output)
    }

    pub(super) fn convert_retained_audio_with_program(
        program: &str,
        mode: RecordAudioMode,
        input: &Path,
        output: &Path,
    ) -> Result<()> {
        let args = afconvert_args(mode, input, output);
        let output = Command::new(program)
            .args(&args)
            .output()
            .with_context(|| format!("run {program}"))?;
        if !output.status.success() {
            bail!(
                "afconvert failed with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    fn afconvert_args(mode: RecordAudioMode, input: &Path, output: &Path) -> Vec<OsString> {
        let mut args = vec![input.as_os_str().to_owned(), output.as_os_str().to_owned()];
        match mode {
            RecordAudioMode::Off => unreachable!(),
            RecordAudioMode::Lossless => {
                args.extend(["-f", "flac", "-d", "flac"].map(OsString::from));
            }
            RecordAudioMode::Compact => {
                args.extend(["-f", "m4af", "-d", "aac", "-b", "32000"].map(OsString::from));
            }
        }
        args
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn compact_afconvert_args_use_aac_32_kbps() {
            let args = afconvert_args(
                RecordAudioMode::Compact,
                Path::new("input.wav"),
                Path::new("output.m4a"),
            );
            let args = args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>();

            assert_eq!(
                args,
                [
                    "input.wav",
                    "output.m4a",
                    "-f",
                    "m4af",
                    "-d",
                    "aac",
                    "-b",
                    "32000"
                ]
            );
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::OsString;
    use std::process::Command;

    use anyhow::{bail, Context, Result};

    use super::*;

    const FFMPEG: &str = "ffmpeg";

    pub(super) fn convert_retained_audio(
        mode: RecordAudioMode,
        input: &Path,
        output: &Path,
    ) -> Result<()> {
        convert_retained_audio_with_program(FFMPEG, mode, input, output)
    }

    pub(super) fn convert_retained_audio_with_program(
        program: &str,
        mode: RecordAudioMode,
        input: &Path,
        output: &Path,
    ) -> Result<()> {
        let args = ffmpeg_args(mode, input, output);
        let output = Command::new(program)
            .args(&args)
            .output()
            .with_context(|| {
                format!(
                    "run {program}; install ffmpeg and ensure it is on PATH to retain audio on Windows"
                )
            })?;
        if !output.status.success() {
            bail!(
                "ffmpeg failed with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    fn ffmpeg_args(mode: RecordAudioMode, input: &Path, output: &Path) -> Vec<OsString> {
        let mut args = vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-y".into(),
            "-i".into(),
            input.as_os_str().to_owned(),
            "-vn".into(),
        ];
        match mode {
            RecordAudioMode::Off => unreachable!(),
            RecordAudioMode::Lossless => {
                args.extend(["-c:a", "flac"].map(OsString::from));
            }
            RecordAudioMode::Compact => {
                args.extend(["-c:a", "aac", "-b:a", "32k"].map(OsString::from));
            }
        }
        args.push(output.as_os_str().to_owned());
        args
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn arg_strings(args: Vec<OsString>) -> Vec<String> {
            args.into_iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect()
        }

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
        fn lossless_ffmpeg_args_use_flac_encoder() {
            let args = arg_strings(ffmpeg_args(
                RecordAudioMode::Lossless,
                Path::new("input.wav"),
                Path::new("output.flac"),
            ));

            assert_eq!(
                args,
                [
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-i",
                    "input.wav",
                    "-vn",
                    "-c:a",
                    "flac",
                    "output.flac"
                ]
            );
        }

        #[test]
        fn compact_ffmpeg_args_use_aac_32_kbps() {
            let args = arg_strings(ffmpeg_args(
                RecordAudioMode::Compact,
                Path::new("input.wav"),
                Path::new("output.m4a"),
            ));

            assert_eq!(
                args,
                [
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-i",
                    "input.wav",
                    "-vn",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "32k",
                    "output.m4a"
                ]
            );
        }

        #[test]
        fn missing_ffmpeg_reports_windows_retained_audio_hint() {
            let err = convert_retained_audio_with_program(
                "__shuohua_missing_ffmpeg__",
                RecordAudioMode::Lossless,
                Path::new("input.wav"),
                Path::new("output.flac"),
            )
            .unwrap_err();

            assert!(format!("{err:#}").contains("retain audio on Windows"));
        }

        #[ignore = "requires ffmpeg on PATH; run only during Windows retained-audio runtime smoke"]
        #[test]
        fn ffmpeg_runtime_smoke_creates_flac_and_m4a() {
            let dir =
                std::env::temp_dir().join(format!("shuohua-audio-convert-{}", ulid::Ulid::new()));
            std::fs::create_dir_all(&dir).unwrap();
            let input = dir.join("input.wav");
            write_test_wav(&input);

            for (mode, name) in [
                (RecordAudioMode::Lossless, "output.flac"),
                (RecordAudioMode::Compact, "output.m4a"),
            ] {
                let output = dir.join(name);
                convert_retained_audio(mode, &input, &output).unwrap();
                assert!(output.is_file(), "missing {}", output.display());
                assert!(std::fs::metadata(&output).unwrap().len() > 0);
            }

            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
mod imp {
    use anyhow::Result;

    use super::*;

    pub(super) fn convert_retained_audio(
        _mode: RecordAudioMode,
        _input: &Path,
        _output: &Path,
    ) -> Result<()> {
        anyhow::bail!("retained audio conversion is not implemented on this platform")
    }

    #[cfg(test)]
    pub(super) fn convert_retained_audio_with_program(
        _program: &str,
        mode: RecordAudioMode,
        input: &Path,
        output: &Path,
    ) -> Result<()> {
        convert_retained_audio(mode, input, output)
    }
}
