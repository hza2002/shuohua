use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::SystemTime;

use anyhow::{bail, Context, Result};

use crate::history::HistoryRecord;
use crate::paths::StateDirs;

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

pub fn audio_path_for_record_in_state_dir(state_dir: &Path, recording_id: &str) -> PathBuf {
    if !is_valid_recording_id(recording_id) {
        return state_dir.join("audio");
    }
    state_dir.join("audio").join(format!("{recording_id}.flac"))
}

pub fn audio_info_for_record(record: &HistoryRecord) -> AudioInfo {
    audio_info_for_recording_id_in_state_dir(StateDirs::discover().root(), &record.id)
}

pub fn audio_info_for_recording_id_in_state_dir(state_dir: &Path, recording_id: &str) -> AudioInfo {
    let audio_dir = state_dir.join("audio");
    if !is_valid_recording_id(recording_id) {
        tracing::warn!(recording_id, "invalid retained audio recording id");
        return missing_audio_info(audio_dir);
    }

    let flac = audio_dir.join(format!("{recording_id}.flac"));
    let m4a = audio_dir.join(format!("{recording_id}.m4a"));
    let flac_exists = flac.is_file();
    let m4a_exists = m4a.is_file();
    match (flac_exists, m4a_exists) {
        (true, false) => audio_info_for_path(flac),
        (false, true) => audio_info_for_path(m4a),
        (true, true) => {
            tracing::warn!(
                recording_id,
                flac = %flac.display(),
                m4a = %m4a.display(),
                "multiple retained audio files found"
            );
            missing_audio_info(flac)
        }
        (false, false) => missing_audio_info(flac),
    }
}

pub fn missing_audio_info_for_record(record: &HistoryRecord) -> AudioInfo {
    missing_audio_info(audio_path_for_record_in_state_dir(
        StateDirs::discover().root(),
        &record.id,
    ))
}

fn missing_audio_info(path: PathBuf) -> AudioInfo {
    AudioInfo {
        path,
        size_bytes: None,
        modified: None,
    }
}

pub fn audio_info_for_path(path: PathBuf) -> AudioInfo {
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => AudioInfo {
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
    ensure_audio_path(path, &StateDirs::discover().audio())?;
    ensure_regular_file_if_present(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(DeleteAudioResult::Deleted),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DeleteAudioResult::Missing),
        Err(e) => Err(e).with_context(|| format!("delete audio {}", path.display())),
    }
}

pub fn open_audio_path(path: &Path) -> Result<()> {
    ensure_existing_audio(path)?;
    open_with_args(&[path.as_os_str()])
}

pub fn reveal_audio_path(path: &Path) -> Result<()> {
    ensure_existing_audio(path)?;
    open_with_args(&[std::ffi::OsStr::new("-R"), path.as_os_str()])
}

fn open_with_args(args: &[&std::ffi::OsStr]) -> Result<()> {
    ProcessCommand::new("/usr/bin/open")
        .args(args)
        .spawn()
        .context("launch open")?;
    Ok(())
}

fn ensure_existing_audio(path: &Path) -> Result<()> {
    ensure_audio_path(path, &StateDirs::discover().audio())?;
    if !is_regular_file(path)? {
        bail!("audio file is missing: {}", path.display());
    }
    Ok(())
}

fn ensure_regular_file_if_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => bail!(
            "refusing to operate on unsupported audio path: {}",
            path.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("inspect audio {}", path.display())),
    }
}

fn is_regular_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_file()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("inspect audio {}", path.display())),
    }
}

fn ensure_audio_path(path: &Path, audio_dir: &Path) -> Result<()> {
    if !matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("flac" | "m4a")
    ) {
        bail!(
            "refusing to operate on unsupported audio path: {}",
            path.display()
        );
    }

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("unsupported audio path: {}", path.display()))?;
    if parent != audio_dir {
        bail!(
            "refusing to operate on unsupported audio path: {}",
            path.display()
        );
    }

    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        bail!(
            "refusing to operate on unsupported audio path: {}",
            path.display()
        );
    };
    if !is_valid_recording_id(stem) {
        bail!(
            "refusing to operate on unsupported audio path: {}",
            path.display()
        );
    }
    Ok(())
}

fn is_valid_recording_id(value: &str) -> bool {
    ulid::Ulid::from_string(value).is_ok()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn resolves_lossless_audio_by_recording_id() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-test-{}", ulid::Ulid::new()));
        let audio_dir = dir.join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        let id = ulid::Ulid::new().to_string();
        let path = audio_dir.join(format!("{id}.flac"));
        fs::write(&path, [0u8; 12]).unwrap();

        let info = audio_info_for_recording_id_in_state_dir(&dir, &id);

        assert_eq!(info.path, path);
        assert!(info.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resolves_compact_audio_by_recording_id() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-test-{}", ulid::Ulid::new()));
        let audio_dir = dir.join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        let id = ulid::Ulid::new().to_string();
        let path = audio_dir.join(format!("{id}.m4a"));
        fs::write(&path, [0u8; 12]).unwrap();

        let info = audio_info_for_recording_id_in_state_dir(&dir, &id);

        assert_eq!(info.path, path);
        assert!(info.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn duplicate_formats_are_reported_as_unavailable() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-test-{}", ulid::Ulid::new()));
        let audio_dir = dir.join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        let id = ulid::Ulid::new().to_string();
        fs::write(audio_dir.join(format!("{id}.flac")), [0u8; 12]).unwrap();
        fs::write(audio_dir.join(format!("{id}.m4a")), [0u8; 12]).unwrap();

        let info = audio_info_for_recording_id_in_state_dir(&dir, &id);

        assert!(!info.exists());
        let _ = fs::remove_dir_all(dir);
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

    #[cfg(unix)]
    #[test]
    fn audio_info_does_not_follow_symlinks() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("target.flac");
        let link = dir.join(format!("{}.flac", ulid::Ulid::new()));
        fs::write(&target, [0u8; 12]).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let info = audio_info_for_path(link.clone());

        assert_eq!(info.path, link);
        assert!(!info.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ensure_audio_path_accepts_supported_file_inside_audio_dir() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-delete-{}", ulid::Ulid::new()));
        let audio_dir = dir.join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        let id = ulid::Ulid::new().to_string();
        let audio = audio_dir.join(format!("{id}.flac"));
        fs::write(&audio, [0u8; 4]).unwrap();

        ensure_audio_path(&audio, &audio_dir).unwrap();

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ensure_audio_path_refuses_paths_outside_audio_dir() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-delete-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let audio_dir = dir.join("audio");
        let audio = dir.join(format!("{}.flac", ulid::Ulid::new()));
        fs::write(&audio, [0u8; 4]).unwrap();

        let err = ensure_audio_path(&audio, &audio_dir).unwrap_err();

        assert!(err.to_string().contains("unsupported audio"));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn delete_audio_path_refuses_symlinks() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-delete-{}", ulid::Ulid::new()));
        let audio_dir = dir.join("audio");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(&audio_dir).unwrap();
        let target = dir.join("target.flac");
        let link = audio_dir.join(format!("{}.flac", ulid::Ulid::new()));
        fs::write(&target, [0u8; 12]).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = ensure_regular_file_if_present(&link).unwrap_err();

        assert!(err.to_string().contains("unsupported audio"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_recording_id_has_no_audio_path() {
        let dir = std::env::temp_dir().join(format!("shuohua-audio-test-{}", ulid::Ulid::new()));
        let info = audio_info_for_recording_id_in_state_dir(&dir, "../escape");

        assert!(!info.exists());
        assert_eq!(info.path, dir.join("audio"));
    }

    #[test]
    fn delete_audio_path_refuses_unsupported_extension() {
        let path = std::env::temp_dir().join(format!("shuohua-audio-{}.wav", ulid::Ulid::new()));

        let err = delete_audio_path(&path).unwrap_err();

        assert!(err.to_string().contains("unsupported audio"));
    }
}
