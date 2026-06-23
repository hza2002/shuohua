use std::path::Path;

use anyhow::Result;

pub(crate) fn open_path(path: &Path) -> Result<()> {
    imp::open_path(path)
}

pub(crate) fn reveal_path(path: &Path) -> Result<()> {
    imp::reveal_path(path)
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use anyhow::Context;
    use std::process::Command;

    pub(super) fn open_path(path: &Path) -> Result<()> {
        Command::new("open")
            .arg(path)
            .spawn()
            .with_context(|| format!("open {}", path.display()))?;
        Ok(())
    }

    pub(super) fn reveal_path(path: &Path) -> Result<()> {
        Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .with_context(|| format!("reveal {}", path.display()))?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use anyhow::Context;
    use std::process::Command;

    pub(super) fn open_path(path: &Path) -> Result<()> {
        xdg_open(path)
    }

    pub(super) fn reveal_path(path: &Path) -> Result<()> {
        let target = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        xdg_open(target)
    }

    fn xdg_open(path: &Path) -> Result<()> {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .with_context(|| format!("xdg-open {}", path.display()))?;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::*;
    use anyhow::Context;
    use std::process::Command;

    pub(super) fn open_path(path: &Path) -> Result<()> {
        Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .with_context(|| format!("explorer.exe {}", path.display()))?;
        Ok(())
    }

    pub(super) fn reveal_path(path: &Path) -> Result<()> {
        Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .with_context(|| format!("explorer.exe /select,{}", path.display()))?;
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
mod imp {
    use super::*;

    pub(super) fn open_path(path: &Path) -> Result<()> {
        anyhow::bail!(
            "path open is not implemented on this platform: {}",
            path.display()
        )
    }

    pub(super) fn reveal_path(path: &Path) -> Result<()> {
        anyhow::bail!(
            "path reveal is not implemented on this platform: {}",
            path.display()
        )
    }
}
