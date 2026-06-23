use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorLaunch {
    ShellCommand { command: String },
    MacOpen { path: String },
}

pub fn editor_launch_for(path: &Path, visual: Option<&str>, editor: Option<&str>) -> EditorLaunch {
    let path = path.display().to_string();
    if let Some(command) = non_empty(visual).or_else(|| non_empty(editor)) {
        return EditorLaunch::ShellCommand {
            command: format!("{command} {}", shell_quote(&path)),
        };
    }
    EditorLaunch::MacOpen { path }
}

pub fn open_in_editor(path: &Path) -> Result<()> {
    match editor_launch_for(
        path,
        std::env::var("VISUAL").ok().as_deref(),
        std::env::var("EDITOR").ok().as_deref(),
    ) {
        EditorLaunch::ShellCommand { command } => {
            std::process::Command::new("/bin/sh")
                .arg("-lc")
                .arg(&command)
                .spawn()
                .with_context(|| format!("launch editor command {command:?}"))?;
        }
        EditorLaunch::MacOpen { path } => crate::platform::path::open_path(Path::new(&path))?,
    }
    Ok(())
}

pub fn reveal_in_finder(path: &Path) -> Result<()> {
    match reveal_launch_for(path) {
        Some(RevealLaunch::RevealFile(path)) => {
            crate::platform::path::reveal_path(&path)
                .with_context(|| format!("reveal config file {}", path.display()))?;
        }
        Some(RevealLaunch::OpenDir(path)) => {
            crate::platform::path::open_path(&path)
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

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let value = value.trim();
        if value.is_empty() {
            None
        } else {
            Some(value)
        }
    })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn editor_launch_prefers_visual_then_editor() {
        let path = Path::new("/tmp/config.toml");

        assert_eq!(
            editor_launch_for(path, Some("code"), Some("vim")),
            EditorLaunch::ShellCommand {
                command: "code '/tmp/config.toml'".to_string(),
            }
        );
        assert_eq!(
            editor_launch_for(path, Some(" "), Some("vim")),
            EditorLaunch::ShellCommand {
                command: "vim '/tmp/config.toml'".to_string(),
            }
        );
    }

    #[test]
    fn editor_launch_splits_program_and_args() {
        assert_eq!(
            editor_launch_for(Path::new("/tmp/config.toml"), Some("nvim -f"), None),
            EditorLaunch::ShellCommand {
                command: "nvim -f '/tmp/config.toml'".to_string(),
            }
        );
    }

    #[test]
    fn editor_launch_preserves_quoted_editor_command_and_quotes_path() {
        assert_eq!(
            editor_launch_for(
                Path::new("/tmp/config dir/config's.toml"),
                Some("'/Applications/My Editor.app/Contents/MacOS/edit' --wait"),
                None,
            ),
            EditorLaunch::ShellCommand {
                command:
                    "'/Applications/My Editor.app/Contents/MacOS/edit' --wait '/tmp/config dir/config'\\''s.toml'"
                        .to_string(),
            }
        );
    }

    #[test]
    fn editor_launch_falls_back_to_macos_open() {
        assert_eq!(
            editor_launch_for(Path::new("/tmp/config.toml"), None, None),
            EditorLaunch::MacOpen {
                path: "/tmp/config.toml".to_string(),
            }
        );
    }

    #[test]
    fn reveal_launch_handles_file_dir_and_missing_child() {
        let dir = std::env::temp_dir().join(format!("shuohua-reveal-test-{}", ulid::Ulid::new()));
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
