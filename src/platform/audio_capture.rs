use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{SampleFormat, SupportedStreamConfig};

pub(crate) const DIAGNOSTIC_REASON: &str = "diagnostic_probe_only";

#[derive(Debug, Clone)]
pub(crate) struct InputDeviceInfo {
    pub(crate) name: Option<String>,
    pub(crate) sample_rate: u32,
    pub(crate) channels: u16,
    pub(crate) sample_format: SampleFormat,
}

#[derive(Debug, Clone)]
pub(crate) struct InputDiagnostics {
    pub(crate) backend: &'static str,
    pub(crate) default_input: Option<InputDeviceInfo>,
    pub(crate) default_input_error: Option<String>,
    pub(crate) input_device_count: Option<usize>,
    pub(crate) device_count_error: Option<String>,
}

pub(crate) fn probe_default_input() -> Result<InputDeviceInfo> {
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

pub(crate) fn diagnose_input() -> Result<InputDiagnostics> {
    let host = cpal::default_host();
    let (default_input, default_input_error) = match host.default_input_device() {
        Some(device) => {
            let name = Some(device.to_string());
            match device
                .default_input_config()
                .context("query default input config")
                .and_then(|supported| {
                    validate_supported_config(&supported)?;
                    Ok(InputDeviceInfo {
                        name,
                        sample_rate: supported.sample_rate(),
                        channels: supported.channels(),
                        sample_format: supported.sample_format(),
                    })
                }) {
                Ok(info) => (Some(info), None),
                Err(error) => (None, Some(format!("{error:#}"))),
            }
        }
        None => (None, Some("no default input device".to_string())),
    };

    let (input_device_count, device_count_error) = match host.input_devices() {
        Ok(devices) => (Some(devices.count()), None),
        Err(error) => (None, Some(error.to_string())),
    };

    Ok(InputDiagnostics {
        backend: backend_name(),
        default_input,
        default_input_error,
        input_device_count,
        device_count_error,
    })
}

fn validate_supported_config(supported: &SupportedStreamConfig) -> Result<()> {
    let sample_format = supported.sample_format();
    if !is_supported_input_format(sample_format) {
        anyhow::bail!("recorder requires F32/I16/U16 input, got {sample_format:?}");
    }
    if supported.channels() == 0 {
        anyhow::bail!("default input device reports 0 channels");
    }
    Ok(())
}

fn is_supported_input_format(format: SampleFormat) -> bool {
    matches!(
        format,
        SampleFormat::F32 | SampleFormat::I16 | SampleFormat::U16
    )
}

const fn backend_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "cpal_wasapi"
    }
    #[cfg(target_os = "linux")]
    {
        "cpal_alsa"
    }
    #[cfg(target_os = "macos")]
    {
        "cpal_coreaudio"
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        "cpal"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_input_formats_match_recorder_stream_dispatch() {
        assert!(is_supported_input_format(SampleFormat::F32));
        assert!(is_supported_input_format(SampleFormat::I16));
        assert!(is_supported_input_format(SampleFormat::U16));
        assert!(!is_supported_input_format(SampleFormat::F64));
    }

    #[test]
    fn diagnostic_reason_is_stable() {
        assert_eq!(DIAGNOSTIC_REASON, "diagnostic_probe_only");
    }
}
