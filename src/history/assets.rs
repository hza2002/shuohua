use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context, Result};

use crate::history::{AudioDeleteResult, CleanupIssue};
use crate::trash::FileDeleter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioAssetState {
    Missing,
    Present,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioAssetInfo {
    pub path: PathBuf,
    pub size_bytes: Option<u64>,
    pub modified: Option<SystemTime>,
    pub state: AudioAssetState,
}

pub(crate) fn audio_dir_for_history_dir(history_dir: &Path) -> PathBuf {
    history_dir
        .parent()
        .map(|state_dir| state_dir.join("audio"))
        .unwrap_or_else(|| crate::paths::StateDirs::discover().audio())
}

pub(crate) fn audio_info_in_dir(audio_dir: &Path, id: &str) -> Result<AudioAssetInfo> {
    validate_recording_id(id)?;
    let flac = audio_dir.join(format!("{id}.flac"));
    let m4a = audio_dir.join(format!("{id}.m4a"));
    let flac_state = inspect_candidate(&flac)?;
    let m4a_state = inspect_candidate(&m4a)?;

    match (flac_state, m4a_state) {
        (Candidate::Present(flac_meta), Candidate::Missing) => Ok(present_info(flac, flac_meta)),
        (Candidate::Missing, Candidate::Present(m4a_meta)) => Ok(present_info(m4a, m4a_meta)),
        (Candidate::Present(_), Candidate::Present(_)) => Ok(AudioAssetInfo {
            path: flac,
            size_bytes: None,
            modified: None,
            state: AudioAssetState::Conflict,
        }),
        (Candidate::Missing, Candidate::Missing) => Ok(AudioAssetInfo {
            path: flac,
            size_bytes: None,
            modified: None,
            state: AudioAssetState::Missing,
        }),
    }
}

pub(crate) fn delete_audio_in_dir(
    audio_dir: &Path,
    id: &str,
    deleter: &FileDeleter,
) -> Result<AudioDeleteResult> {
    let info = audio_info_in_dir(audio_dir, id)?;
    match info.state {
        AudioAssetState::Missing => Ok(AudioDeleteResult {
            id: id.to_string(),
            deleted: false,
        }),
        AudioAssetState::Conflict => {
            bail!("audio asset conflict for {id}: both .flac and .m4a exist")
        }
        AudioAssetState::Present => {
            deleter
                .delete(&info.path)
                .with_context(|| format!("delete audio {}", info.path.display()))?;
            Ok(AudioDeleteResult {
                id: id.to_string(),
                deleted: true,
            })
        }
    }
}

pub(crate) fn preflight_history_delete_audio_in_dir(audio_dir: &Path, id: &str) -> Result<()> {
    let info = audio_info_in_dir(audio_dir, id)?;
    match info.state {
        AudioAssetState::Missing | AudioAssetState::Present => Ok(()),
        AudioAssetState::Conflict => {
            bail!("audio asset conflict for {id}: both .flac and .m4a exist")
        }
    }
}

/// 批量清理视角下的单条 audio 分类。与 `audio_info_in_dir` 不同：symlink /
/// non-regular / conflict 不 bail，而是归类为 `Unsafe`，让 preview/execute 把它当
/// 一条可跳过的告警而非中断整批。真正的 IO 错误仍以 `Err` 上报。
pub(crate) enum CleanupAudioClass {
    Missing,
    Present {
        path: PathBuf,
        size_bytes: u64,
        identity: CleanupAudioIdentity,
    },
    Unsafe(CleanupIssue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanupAudioIdentity {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(unix)]
    ctime: i64,
    #[cfg(unix)]
    ctime_nsec: i64,
}

fn cleanup_audio_identity(meta: &fs::Metadata) -> CleanupAudioIdentity {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    CleanupAudioIdentity {
        len: meta.len(),
        modified: meta.modified().ok(),
        #[cfg(unix)]
        dev: meta.dev(),
        #[cfg(unix)]
        ino: meta.ino(),
        #[cfg(unix)]
        ctime: meta.ctime(),
        #[cfg(unix)]
        ctime_nsec: meta.ctime_nsec(),
    }
}

pub(crate) fn classify_cleanup_audio(audio_dir: &Path, id: &str) -> Result<CleanupAudioClass> {
    // 非法 ULID 不可能有按约定命名的音频文件；当作无音频跳过。
    if validate_recording_id(id).is_err() {
        return Ok(CleanupAudioClass::Missing);
    }
    let flac = audio_dir.join(format!("{id}.flac"));
    let m4a = audio_dir.join(format!("{id}.m4a"));
    let flac_c = classify_candidate(&flac)?;
    let m4a_c = classify_candidate(&m4a)?;

    if let Some(issue) = candidate_issue(&flac_c).or_else(|| candidate_issue(&m4a_c)) {
        return Ok(CleanupAudioClass::Unsafe(issue));
    }
    match (flac_c, m4a_c) {
        (CandidateClass::Present(meta), CandidateClass::Missing) => {
            Ok(CleanupAudioClass::Present {
                path: flac,
                size_bytes: meta.len(),
                identity: cleanup_audio_identity(&meta),
            })
        }
        (CandidateClass::Missing, CandidateClass::Present(meta)) => {
            Ok(CleanupAudioClass::Present {
                path: m4a,
                size_bytes: meta.len(),
                identity: cleanup_audio_identity(&meta),
            })
        }
        (CandidateClass::Present(_), CandidateClass::Present(_)) => {
            Ok(CleanupAudioClass::Unsafe(CleanupIssue::Conflict))
        }
        (CandidateClass::Missing, CandidateClass::Missing) => Ok(CleanupAudioClass::Missing),
        // Unsafe 组合已在上面短路返回。
        _ => unreachable!("unsafe candidates handled above"),
    }
}

pub(crate) fn validate_recording_id(id: &str) -> Result<ulid::Ulid> {
    ulid::Ulid::from_string(id).map_err(|_| anyhow!("invalid recording id: {id}"))
}

enum Candidate {
    Missing,
    Present(fs::Metadata),
}

fn inspect_candidate(path: &Path) -> Result<Candidate> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("audio file must not be a symlink: {}", path.display())
        }
        Ok(metadata) if metadata.file_type().is_file() => Ok(Candidate::Present(metadata)),
        Ok(_) => bail!("audio path must be a regular file: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Candidate::Missing),
        Err(error) => Err(error).with_context(|| format!("inspect audio {}", path.display())),
    }
}

enum CandidateClass {
    Missing,
    Present(fs::Metadata),
    Symlink,
    NonRegular,
}

fn classify_candidate(path: &Path) -> Result<CandidateClass> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(CandidateClass::Symlink),
        Ok(metadata) if metadata.file_type().is_file() => Ok(CandidateClass::Present(metadata)),
        Ok(_) => Ok(CandidateClass::NonRegular),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(CandidateClass::Missing),
        Err(error) => Err(error).with_context(|| format!("inspect audio {}", path.display())),
    }
}

fn candidate_issue(candidate: &CandidateClass) -> Option<CleanupIssue> {
    match candidate {
        CandidateClass::Symlink => Some(CleanupIssue::Symlink),
        CandidateClass::NonRegular => Some(CleanupIssue::NonRegular),
        CandidateClass::Missing | CandidateClass::Present(_) => None,
    }
}

fn present_info(path: PathBuf, metadata: fs::Metadata) -> AudioAssetInfo {
    AudioAssetInfo {
        path,
        size_bytes: Some(metadata.len()),
        modified: metadata.modified().ok(),
        state: AudioAssetState::Present,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::AudioAssetState;
    use crate::history::HistoryService;

    #[test]
    fn resolves_exactly_one_flac_or_m4a() {
        let history_dir = temp_history_dir("resolve-one");
        let audio_dir = audio_dir_for_history(&history_dir);
        fs::create_dir_all(&audio_dir).unwrap();
        let flac_id = ulid::Ulid::generate().to_string();
        let m4a_id = ulid::Ulid::generate().to_string();
        let flac = audio_dir.join(format!("{flac_id}.flac"));
        let m4a = audio_dir.join(format!("{m4a_id}.m4a"));
        fs::write(&flac, [1, 2, 3]).unwrap();
        fs::write(&m4a, [4, 5]).unwrap();
        let service = HistoryService::with_dir(history_dir.clone());

        let flac_info = service.audio(&flac_id).unwrap();
        let m4a_info = service.audio(&m4a_id).unwrap();

        assert_eq!(flac_info.state, AudioAssetState::Present);
        assert_eq!(flac_info.path, flac);
        assert_eq!(flac_info.size_bytes, Some(3));
        assert!(flac_info.modified.is_some());
        assert_eq!(m4a_info.state, AudioAssetState::Present);
        assert_eq!(m4a_info.path, m4a);
        assert_eq!(m4a_info.size_bytes, Some(2));
        let _ = fs::remove_dir_all(state_dir_for_history(&history_dir));
    }

    #[test]
    fn reports_conflict_when_both_formats_exist() {
        let history_dir = temp_history_dir("conflict");
        let audio_dir = audio_dir_for_history(&history_dir);
        fs::create_dir_all(&audio_dir).unwrap();
        let id = ulid::Ulid::generate().to_string();
        fs::write(audio_dir.join(format!("{id}.flac")), [1]).unwrap();
        fs::write(audio_dir.join(format!("{id}.m4a")), [2]).unwrap();
        let service = HistoryService::with_dir(history_dir.clone());

        let info = service.audio(&id).unwrap();
        let error = service.delete_audio(&id).unwrap_err();

        assert_eq!(info.state, AudioAssetState::Conflict);
        assert!(error.to_string().contains("conflict"), "{error:#}");
        assert!(audio_dir.join(format!("{id}.flac")).exists());
        assert!(audio_dir.join(format!("{id}.m4a")).exists());
        let _ = fs::remove_dir_all(state_dir_for_history(&history_dir));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_and_invalid_ulid_paths() {
        use std::os::unix::fs::symlink;

        let history_dir = temp_history_dir("rejects");
        let audio_dir = audio_dir_for_history(&history_dir);
        fs::create_dir_all(&audio_dir).unwrap();
        let id = ulid::Ulid::generate().to_string();
        let target = audio_dir.join("target.flac");
        let link = audio_dir.join(format!("{id}.flac"));
        fs::write(&target, [1]).unwrap();
        symlink(&target, &link).unwrap();
        let service = HistoryService::with_dir(history_dir.clone());

        let invalid = service.audio("not-a-ulid").unwrap_err();
        let symlink_error = service.audio(&id).unwrap_err();
        let delete_error = service.delete_audio(&id).unwrap_err();

        assert!(invalid.to_string().contains("invalid"), "{invalid:#}");
        assert!(
            symlink_error.to_string().contains("symlink"),
            "{symlink_error:#}"
        );
        assert!(
            delete_error.to_string().contains("symlink"),
            "{delete_error:#}"
        );
        assert!(link.exists());
        let _ = fs::remove_dir_all(state_dir_for_history(&history_dir));
    }

    fn temp_history_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!(
                "shuohua-history-assets-{name}-{}",
                ulid::Ulid::generate()
            ))
            .join("history")
    }

    fn state_dir_for_history(history_dir: &std::path::Path) -> std::path::PathBuf {
        history_dir.parent().unwrap().to_path_buf()
    }

    fn audio_dir_for_history(history_dir: &std::path::Path) -> std::path::PathBuf {
        state_dir_for_history(history_dir).join("audio")
    }
}
