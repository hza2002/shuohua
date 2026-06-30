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
    /// 把准备好的 binary 原子替换到 `target`（preferred install path）。
    /// 只更新 per-user 可写路径，不做 sudo 提权。
    fn install_executable(&self, new_exe: &Path, target: &Path) -> Result<()>;
}

#[cfg(target_os = "macos")]
pub fn current() -> impl UpdatePlatform {
    macos::MacosUpdatePlatform
}

#[cfg(not(target_os = "macos"))]
pub fn current() -> impl UpdatePlatform {
    unsupported::UnsupportedUpdatePlatform
}
