use anyhow::{Context, Result};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub struct MacosUpdatePlatform;

impl super::UpdatePlatform for MacosUpdatePlatform {
    fn artifact_target(&self) -> Result<&'static str> {
        match std::env::consts::ARCH {
            "aarch64" => Ok("aarch64-apple-darwin"),
            other => {
                anyhow::bail!("no release artifact is available for macOS architecture {other}")
            }
        }
    }

    fn current_exe(&self) -> Result<PathBuf> {
        std::env::current_exe().context("resolve current shuo executable path")
    }

    fn prepare_executable(&self, path: &Path) -> Result<()> {
        let mut permissions = fs::metadata(path)
            .with_context(|| format!("read metadata for {}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("mark {} executable", path.display()))
    }

    fn install_executable(&self, new_exe: &Path, target: &Path) -> Result<()> {
        replace_path(new_exe, target)
    }
}

/// 原子替换 `target`：写 sibling temp → chmod → rename。target 必须 per-user 可写
/// （preferred install path），失败直接报错，不 sudo 提权。
pub fn replace_path(new_exe: &Path, target: &Path) -> Result<()> {
    replace_path_with(
        new_exe,
        target,
        |from, to| fs::copy(from, to).map(|_| ()),
        |path, mode| {
            fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
            Ok(())
        },
        |from, to| fs::rename(from, to),
    )
}

fn replace_path_with(
    new_exe: &Path,
    target: &Path,
    copy: impl Fn(&Path, &Path) -> io::Result<()>,
    set_mode: impl Fn(&Path, u32) -> io::Result<()>,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> Result<()> {
    let parent = target
        .parent()
        .with_context(|| format!("{} has no parent directory", target.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create install directory {}", parent.display()))?;
    let temp = parent.join(format!(
        ".{}.update-{}",
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("shuo"),
        ulid::Ulid::new()
    ));
    let mode = fs::metadata(new_exe)
        .with_context(|| format!("read metadata for {}", new_exe.display()))?
        .permissions()
        .mode();
    copy(new_exe, &temp).with_context(|| format!("copy update to {}", temp.display()))?;
    let result = (|| {
        set_mode(&temp, mode).with_context(|| format!("set permissions on {}", temp.display()))?;
        rename(&temp, target).with_context(|| format!("replace {}", target.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_path_overwrites_target() {
        let dir = std::env::temp_dir().join(format!("shuohua-replace-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let current = dir.join("shuo");
        let new_exe = dir.join("new-shuo");
        fs::write(&current, b"old").unwrap();
        fs::write(&new_exe, b"new").unwrap();

        replace_path(&new_exe, &current).unwrap();

        assert_eq!(fs::read(&current).unwrap(), b"new");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_path_cleans_temp_file_when_replace_fails() {
        let dir = std::env::temp_dir().join(format!("shuohua-replace-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let current = dir.join("shuo");
        let new_exe = dir.join("new-shuo");
        fs::create_dir_all(&current).unwrap();
        fs::write(&new_exe, b"new").unwrap();

        let err = replace_path(&new_exe, &current).unwrap_err();

        assert!(err.to_string().contains("replace"), "{err:#}");
        let leftovers = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".shuo.update-")
            })
            .count();
        assert_eq!(leftovers, 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_path_propagates_copy_error_without_sudo() {
        let dir = std::env::temp_dir().join(format!("shuohua-replace-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("shuo");
        let new_exe = dir.join("new-shuo");
        fs::write(&new_exe, b"new").unwrap();
        fs::set_permissions(&new_exe, fs::Permissions::from_mode(0o755)).unwrap();

        let err = replace_path_with(
            &new_exe,
            &target,
            |_, _| Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")),
            |_, _| unreachable!("copy failed before chmod"),
            |_, _| unreachable!("copy failed before rename"),
        )
        .unwrap_err();

        assert!(err.to_string().contains("copy update to"), "{err:#}");
        let leftovers = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".shuo.update-")
            })
            .count();
        assert_eq!(leftovers, 0);
        let _ = fs::remove_dir_all(dir);
    }
}
