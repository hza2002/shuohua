use anyhow::{Context, Result};
use std::fs;
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

    fn replace_current_exe(&self, new_exe: &Path) -> Result<PathBuf> {
        let current = self.current_exe()?;
        replace_path(new_exe, &current)?;
        Ok(current)
    }
}

pub fn replace_path(new_exe: &Path, current: &Path) -> Result<()> {
    let parent = current
        .parent()
        .with_context(|| format!("{} has no parent directory", current.display()))?;
    let temp = parent.join(format!(
        ".{}.update-{}",
        current
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("shuo"),
        ulid::Ulid::new()
    ));
    fs::copy(new_exe, &temp).with_context(|| format!("copy update to {}", temp.display()))?;
    let result = (|| {
        let mode = fs::metadata(new_exe)
            .with_context(|| format!("read metadata for {}", new_exe.display()))?
            .permissions()
            .mode();
        fs::set_permissions(&temp, fs::Permissions::from_mode(mode))
            .with_context(|| format!("set permissions on {}", temp.display()))?;
        fs::rename(&temp, current).with_context(|| format!("replace {}", current.display()))
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
}
