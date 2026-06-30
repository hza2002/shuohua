use anyhow::Result;
use std::path::{Path, PathBuf};

pub struct UnsupportedUpdatePlatform;

impl super::UpdatePlatform for UnsupportedUpdatePlatform {
    fn artifact_target(&self) -> Result<&'static str> {
        unsupported()
    }

    fn current_exe(&self) -> Result<PathBuf> {
        unsupported()
    }

    fn prepare_executable(&self, _path: &Path) -> Result<()> {
        unsupported()
    }

    fn install_executable(&self, _new_exe: &Path, _target: &Path) -> Result<()> {
        unsupported()
    }
}

fn unsupported<T>() -> Result<T> {
    anyhow::bail!("self-update is not supported on this platform yet")
}
