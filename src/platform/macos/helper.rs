use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct PcmFrame {
    pub samples: Vec<i16>,
    pub is_last: bool,
}

pub fn encode_pcm_frame(pcm: &[i16], is_last: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 4 + pcm.len() * 2);
    out.push(if is_last { 1 } else { 0 });
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    for &sample in pcm {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[allow(dead_code)]
pub fn decode_pcm_frame(frame: &[u8]) -> Result<PcmFrame> {
    if frame.len() < 5 {
        anyhow::bail!("truncated PCM frame header");
    }
    let is_last = frame[0] & 1 == 1;
    let sample_count = u32::from_le_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;
    let expected_len = 5 + sample_count * 2;
    if frame.len() != expected_len {
        anyhow::bail!("truncated PCM frame payload");
    }

    let mut samples = Vec::with_capacity(sample_count);
    for chunk in frame[5..].chunks_exact(2) {
        samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
    Ok(PcmFrame { samples, is_last })
}

pub fn ensure_helper_binary_at(
    path: &Path,
    lock_path: &Path,
    helper_bytes: &[u8],
) -> Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create helper dir")?;
    }

    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(lock_path)
        .context("open helper lock")?;
    lock_exclusive(&lock).context("lock helper")?;
    let result = publish_helper_locked(path, helper_bytes);
    let _ = unlock(&lock);
    result
}

fn publish_helper_locked(path: &Path, helper_bytes: &[u8]) -> Result<PathBuf> {
    if file_contents_equal(path, helper_bytes).context("read helper")? {
        return Ok(path.to_path_buf());
    }

    let tmp_path = path.with_file_name(format!(
        "{}.tmp.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("helper"),
        std::process::id()
    ));
    let mut file = std::fs::File::create(&tmp_path).context("write helper")?;
    file.write_all(helper_bytes).context("write helper")?;
    file.sync_all().context("sync helper")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = file.metadata().context("stat helper")?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmp_path, perms).context("chmod helper")?;
    }
    drop(file);
    match std::fs::rename(&tmp_path, path) {
        Ok(()) => Ok(path.to_path_buf()),
        Err(error) => {
            let _ = std::fs::remove_file(&tmp_path);
            Err(error).context("publish helper")
        }
    }
}

fn file_contents_equal(path: &Path, expected: &[u8]) -> std::io::Result<bool> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if metadata.len() != expected.len() as u64 {
        return Ok(false);
    }
    let mut body = Vec::with_capacity(expected.len());
    file.read_to_end(&mut body)?;
    Ok(body == expected)
}

fn lock_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn unlock(file: &std::fs::File) -> std::io::Result<()> {
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn helper_publish_skips_rewrite_when_existing_bytes_match() {
        let dir = std::env::temp_dir().join(format!("shuohua-helper-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let helper = dir.join("apple_helper");
        let lock = dir.join("apple_helper.lock");
        let bytes = b"helper-bytes";
        std::fs::write(&helper, bytes).unwrap();
        let before = std::fs::metadata(&helper).unwrap().modified().unwrap();

        std::thread::sleep(Duration::from_millis(5));
        let path = ensure_helper_binary_at(&helper, &lock, bytes).unwrap();

        let after = std::fs::metadata(&helper).unwrap().modified().unwrap();
        assert_eq!(path, helper);
        assert_eq!(before, after, "matching helper should not be rewritten");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn encode_pcm_frame_is_flag_count_then_little_endian_samples() {
        let frame = encode_pcm_frame(&[-1, 0, 258], true);
        assert_eq!(frame[0], 1);
        assert_eq!(&frame[1..5], &3u32.to_le_bytes());
        assert_eq!(&frame[5..], &[0xff, 0xff, 0, 0, 2, 1]);
    }

    #[test]
    fn decode_pcm_frame_rejects_truncated_payload() {
        let err = decode_pcm_frame(&[0, 2, 0, 0, 0, 1]).unwrap_err();
        assert!(err.to_string().contains("truncated PCM frame"));
    }

    #[test]
    fn decode_pcm_frame_round_trips_encoded_samples() {
        let frame = decode_pcm_frame(&encode_pcm_frame(&[-7, 0, 42], false)).unwrap();
        assert_eq!(
            frame,
            PcmFrame {
                samples: vec![-7, 0, 42],
                is_last: false
            }
        );
    }
}
