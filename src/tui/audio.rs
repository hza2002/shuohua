use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::SystemTime;

use anyhow::{bail, Context, Result};

use crate::state::history::{state_dir, HistoryRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInfo {
    pub path: PathBuf,
    pub size_bytes: Option<u64>,
    pub modified: Option<SystemTime>,
}

impl AudioInfo {
    pub fn exists(&self) -> bool {
        self.size_bytes.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteAudioResult {
    Deleted,
    Missing,
}

pub fn audio_path_for_record(record: &HistoryRecord) -> PathBuf {
    audio_path_for_record_in_state_dir(&state_dir(), &record.id)
}

pub fn audio_path_for_record_in_state_dir(state_dir: &Path, recording_id: &str) -> PathBuf {
    state_dir.join("audio").join(format!("{recording_id}.wav"))
}

pub fn audio_info_for_record(record: &HistoryRecord) -> AudioInfo {
    audio_info_for_path(audio_path_for_record(record))
}

pub fn missing_audio_info_for_record(record: &HistoryRecord) -> AudioInfo {
    AudioInfo {
        path: audio_path_for_record(record),
        size_bytes: None,
        modified: None,
    }
}

pub fn audio_info_for_path(path: PathBuf) -> AudioInfo {
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => AudioInfo {
            path,
            size_bytes: Some(metadata.len()),
            modified: metadata.modified().ok(),
        },
        _ => AudioInfo {
            path,
            size_bytes: None,
            modified: None,
        },
    }
}

pub fn delete_audio_path(path: &Path) -> Result<DeleteAudioResult> {
    ensure_wav_path(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(DeleteAudioResult::Deleted),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DeleteAudioResult::Missing),
        Err(e) => Err(e).with_context(|| format!("delete audio {}", path.display())),
    }
}

pub fn open_audio(path: &Path) -> Result<()> {
    ensure_existing_wav(path)?;
    open_with_args(&[path.as_os_str()])
}

pub fn reveal_audio(path: &Path) -> Result<()> {
    ensure_existing_wav(path)?;
    open_with_args(&[std::ffi::OsStr::new("-R"), path.as_os_str()])
}

fn open_with_args(args: &[&std::ffi::OsStr]) -> Result<()> {
    ProcessCommand::new("/usr/bin/open")
        .args(args)
        .spawn()
        .context("launch open")?;
    Ok(())
}

fn ensure_existing_wav(path: &Path) -> Result<()> {
    ensure_wav_path(path)?;
    if !path.is_file() {
        bail!("audio file is missing: {}", path.display());
    }
    Ok(())
}

fn ensure_wav_path(path: &Path) -> Result<()> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("wav") {
        bail!("refusing to operate on non-wav path: {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_path_is_derived_from_state_dir_and_recording_id() {
        let path = audio_path_for_record_in_state_dir(Path::new("/tmp/shuohua-state"), "01HXYZ");

        assert_eq!(path, PathBuf::from("/tmp/shuohua-state/audio/01HXYZ.wav"));
    }

    #[test]
    fn audio_info_reports_existing_file_size() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("01HXYZ.wav");
        fs::write(&path, [0u8; 12]).unwrap();

        let info = audio_info_for_path(path.clone());

        assert_eq!(info.path, path);
        assert_eq!(info.size_bytes, Some(12));
        assert!(info.modified.is_some());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn audio_info_reports_missing_file() {
        let path =
            std::env::temp_dir().join(format!("shuohua-audio-missing-{}.wav", ulid::Ulid::new()));

        let info = audio_info_for_path(path.clone());

        assert_eq!(info.path, path);
        assert!(!info.exists());
        assert_eq!(info.size_bytes, None);
        assert_eq!(info.modified, None);
    }

    #[test]
    fn delete_audio_path_only_removes_wav_file() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-delete-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("01HXYZ.wav");
        let jsonl = dir.join("2026-06.jsonl");
        fs::write(&wav, [0u8; 4]).unwrap();
        fs::write(&jsonl, "{}\n").unwrap();

        assert_eq!(delete_audio_path(&wav).unwrap(), DeleteAudioResult::Deleted);

        assert!(!wav.exists());
        assert!(jsonl.exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn delete_audio_path_refuses_non_wav() {
        let path = std::env::temp_dir().join(format!("shuohua-audio-{}.jsonl", ulid::Ulid::new()));

        let err = delete_audio_path(&path).unwrap_err();

        assert!(err.to_string().contains("non-wav"));
    }
}
