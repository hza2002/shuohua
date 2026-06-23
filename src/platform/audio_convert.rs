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

#[cfg(not(target_os = "macos"))]
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
