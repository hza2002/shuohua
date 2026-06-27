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
    use std::os::windows::ffi::OsStrExt;
    use std::process::Command;

    use anyhow::{bail, Context, Result};
    use windows::core::PCWSTR;
    use windows::Win32::Media::MediaFoundation::{
        MFAudioFormat_AAC, MFAudioFormat_PCM, MFCreateMediaType, MFCreateMemoryBuffer,
        MFCreateSample, MFCreateSinkWriterFromURL, MFMediaType_Audio, MFStartup, MFSTARTUP_FULL,
        MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT,
        MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE,
        MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_VERSION,
    };

    use super::*;

    const AAC_BITRATE: u32 = 32_000;
    const FFMPEG: &str = "ffmpeg";
    const HNS_PER_SECOND: u64 = 10_000_000;

    pub(super) fn convert_retained_audio(
        mode: RecordAudioMode,
        input: &Path,
        output: &Path,
    ) -> Result<()> {
        match mode {
            RecordAudioMode::Off => unreachable!(),
            RecordAudioMode::Compact => convert_wav_to_m4a_media_foundation(input, output)
                .with_context(|| "convert compact retained audio with Windows Media Foundation"),
            RecordAudioMode::Lossless => {
                convert_retained_audio_with_program(FFMPEG, mode, input, output)
            }
        }
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

    fn convert_wav_to_m4a_media_foundation(input: &Path, output: &Path) -> Result<()> {
        let mut reader = hound::WavReader::open(input)
            .with_context(|| format!("open WAV {}", input.display()))?;
        let spec = reader.spec();
        if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
            bail!(
                "Windows Media Foundation compact conversion only supports 16-bit PCM WAV input, got {:?}/{}bit",
                spec.sample_format,
                spec.bits_per_sample
            );
        }
        if spec.channels == 0 || spec.sample_rate == 0 {
            bail!(
                "invalid WAV format for Windows Media Foundation conversion: {} channels at {} Hz",
                spec.channels,
                spec.sample_rate
            );
        }

        let samples = reader
            .samples::<i16>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("read WAV PCM samples")?;
        encode_pcm_i16_to_m4a_media_foundation(&samples, spec.channels, spec.sample_rate, output)
    }

    fn encode_pcm_i16_to_m4a_media_foundation(
        samples: &[i16],
        channels: u16,
        sample_rate: u32,
        output: &Path,
    ) -> Result<()> {
        let path = wide_path(output);
        let bytes_per_sample = 2u32;
        let block_align = channels as u32 * bytes_per_sample;
        let avg_bytes_per_second = sample_rate * block_align;
        let frames_per_sample = samples.len() as u64 / channels as u64;
        let sample_duration_hns = (frames_per_sample * HNS_PER_SECOND / sample_rate as u64) as i64;

        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_FULL).context("start Windows Media Foundation")?;

            let output_type = MFCreateMediaType().context("create MF output media type")?;
            output_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            output_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
            output_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels as u32)?;
            output_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate)?;
            output_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            output_type.SetUINT32(&MF_MT_AVG_BITRATE, AAC_BITRATE)?;

            let input_type = MFCreateMediaType().context("create MF input media type")?;
            input_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            input_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
            input_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels as u32)?;
            input_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate)?;
            input_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            input_type.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_align)?;
            input_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, avg_bytes_per_second)?;

            let writer = MFCreateSinkWriterFromURL(PCWSTR(path.as_ptr()), None, None)
                .with_context(|| format!("create MF sink writer {}", output.display()))?;
            let stream_index = writer
                .AddStream(&output_type)
                .context("add MF AAC output stream")?;
            writer
                .SetInputMediaType(stream_index, &input_type, None)
                .context("set MF PCM input media type")?;
            writer.BeginWriting().context("begin MF sink writing")?;

            if !samples.is_empty() {
                let byte_len = std::mem::size_of_val(samples) as u32;
                let buffer = MFCreateMemoryBuffer(byte_len).context("create MF PCM buffer")?;
                let mut dest = std::ptr::null_mut();
                buffer
                    .Lock(&mut dest, None, None)
                    .context("lock MF PCM buffer")?;
                std::ptr::copy_nonoverlapping(
                    samples.as_ptr() as *const u8,
                    dest,
                    byte_len as usize,
                );
                buffer.Unlock().context("unlock MF PCM buffer")?;
                buffer.SetCurrentLength(byte_len)?;

                let sample = MFCreateSample().context("create MF sample")?;
                sample.AddBuffer(&buffer)?;
                sample.SetSampleTime(0)?;
                sample.SetSampleDuration(sample_duration_hns)?;
                writer
                    .WriteSample(stream_index, &sample)
                    .context("write MF PCM sample")?;
            }

            writer.Finalize().context("finalize MF sink writer")?;
        }

        Ok(())
    }

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain([0]).collect()
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

        #[ignore = "uses Windows Media Foundation and ffmpeg; run only during Windows retained-audio runtime smoke"]
        #[test]
        fn runtime_smoke_creates_native_m4a_and_ffmpeg_flac() {
            let dir =
                std::env::temp_dir().join(format!("shuohua-audio-convert-{}", ulid::Ulid::new()));
            std::fs::create_dir_all(&dir).unwrap();
            let input = dir.join("input.wav");
            write_test_wav(&input);

            let m4a = dir.join("output.m4a");
            convert_retained_audio(RecordAudioMode::Compact, &input, &m4a).unwrap();
            assert!(m4a.is_file(), "missing {}", m4a.display());
            assert!(std::fs::metadata(&m4a).unwrap().len() > 0);

            let flac = dir.join("output.flac");
            convert_retained_audio(RecordAudioMode::Lossless, &input, &flac).unwrap();
            assert!(flac.is_file(), "missing {}", flac.display());
            assert!(std::fs::metadata(&flac).unwrap().len() > 0);

            let _ = std::fs::remove_dir_all(dir);
        }

        #[ignore = "uses Windows Media Foundation; run only during Windows retained-audio runtime smoke"]
        #[test]
        fn media_foundation_runtime_smoke_creates_m4a_without_ffmpeg() {
            let dir = std::env::temp_dir()
                .join(format!("shuohua-audio-convert-mf-{}", ulid::Ulid::new()));
            std::fs::create_dir_all(&dir).unwrap();
            let input = dir.join("input.wav");
            let output = dir.join("output.m4a");
            write_test_wav(&input);

            convert_wav_to_m4a_media_foundation(&input, &output).unwrap();

            assert!(output.is_file(), "missing {}", output.display());
            assert!(std::fs::metadata(&output).unwrap().len() > 0);
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
