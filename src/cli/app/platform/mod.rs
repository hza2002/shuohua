use anyhow::Result;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

pub trait UpdatePlatform {
    fn artifact_target(&self) -> Result<&'static str>;
    fn current_exe(&self) -> Result<PathBuf>;
    fn prepare_executable(&self, path: &Path) -> Result<()>;
    fn replace_current_exe(&self, new_exe: &Path) -> Result<PathBuf>;
}

#[cfg(target_os = "macos")]
pub fn current() -> impl UpdatePlatform {
    macos::MacosUpdatePlatform
}

#[cfg(not(target_os = "macos"))]
pub fn current() -> impl UpdatePlatform {
    unsupported::UnsupportedUpdatePlatform
}
