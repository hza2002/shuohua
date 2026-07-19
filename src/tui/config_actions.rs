use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Open a config file with the OS default application.
///
/// The TUI is the primary way to edit config; this is the escape hatch for
/// users who want to edit the file directly. It uses macOS `open` (default
/// app) rather than `$EDITOR`, because spawning a terminal editor (e.g. vim)
/// from inside the TUI would fight the alternate screen.
pub fn open_path(path: &Path) -> Result<()> {
    std::process::Command::new("open")
        .arg(path)
        .spawn()
        .with_context(|| format!("open {}", path.display()))?;
    Ok(())
}

/// Reveal a config file in Finder, or open its containing folder.
pub fn reveal_in_finder(path: &Path) -> Result<()> {
    match reveal_launch_for(path) {
        Some(RevealLaunch::RevealFile(path)) => {
            std::process::Command::new("open")
                .arg("-R")
                .arg(&path)
                .spawn()
                .with_context(|| format!("reveal config file {}", path.display()))?;
        }
        Some(RevealLaunch::OpenDir(path)) => {
            std::process::Command::new("open")
                .arg(&path)
                .spawn()
                .with_context(|| format!("open config directory {}", path.display()))?;
        }
        None => anyhow::bail!("config path and parent do not exist: {}", path.display()),
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RevealLaunch {
    RevealFile(PathBuf),
    OpenDir(PathBuf),
}

fn reveal_launch_for(path: &Path) -> Option<RevealLaunch> {
    if path.is_file() {
        return Some(RevealLaunch::RevealFile(path.to_path_buf()));
    }
    if path.is_dir() {
        return Some(RevealLaunch::OpenDir(path.to_path_buf()));
    }
    path.parent()
        .filter(|parent| parent.exists())
        .map(|parent| RevealLaunch::OpenDir(parent.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_launch_handles_file_dir_and_missing_child() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-reveal-test-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.toml");
        std::fs::write(&file, "").unwrap();
        let missing = dir.join("missing.toml");

        assert_eq!(
            reveal_launch_for(&file),
            Some(RevealLaunch::RevealFile(file.clone()))
        );
        assert_eq!(
            reveal_launch_for(&dir),
            Some(RevealLaunch::OpenDir(dir.clone()))
        );
        assert_eq!(
            reveal_launch_for(&missing),
            Some(RevealLaunch::OpenDir(dir.clone()))
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
