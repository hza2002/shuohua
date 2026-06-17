use std::path::Path;

use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorLaunch {
    Command { program: String, args: Vec<String> },
    MacOpen { path: String },
}

pub fn editor_launch_for(path: &Path, visual: Option<&str>, editor: Option<&str>) -> EditorLaunch {
    let path = path.display().to_string();
    if let Some(program) = non_empty(visual).or_else(|| non_empty(editor)) {
        return EditorLaunch::Command {
            program: program.to_string(),
            args: vec![path],
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
        EditorLaunch::Command { program, args } => {
            std::process::Command::new(&program)
                .args(&args)
                .spawn()
                .with_context(|| format!("launch editor {program}"))?;
        }
        EditorLaunch::MacOpen { path } => {
            std::process::Command::new("open")
                .arg(&path)
                .spawn()
                .with_context(|| format!("open {path}"))?;
        }
    }
    Ok(())
}

pub fn reveal_in_finder(path: &Path) -> Result<()> {
    let mut command = std::process::Command::new("open");
    if path.is_file() {
        command.arg("-R").arg(path);
    } else {
        command.arg(path);
    }
    command
        .spawn()
        .with_context(|| format!("reveal config path {}", path.display()))?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn editor_launch_prefers_visual_then_editor() {
        let path = Path::new("/tmp/config.toml");

        assert_eq!(
            editor_launch_for(path, Some("code"), Some("vim")),
            EditorLaunch::Command {
                program: "code".to_string(),
                args: vec!["/tmp/config.toml".to_string()],
            }
        );
        assert_eq!(
            editor_launch_for(path, Some(" "), Some("vim")),
            EditorLaunch::Command {
                program: "vim".to_string(),
                args: vec!["/tmp/config.toml".to_string()],
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
}
