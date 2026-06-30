use anyhow::Result;

pub fn launchd_status() -> super::LaunchdStatus {
    super::LaunchdStatus::Unsupported
}

pub fn plist_program() -> Option<std::path::PathBuf> {
    None
}

pub async fn install() -> Result<()> {
    unsupported()
}

pub fn uninstall() -> Result<()> {
    unsupported()
}

pub async fn start() -> Result<()> {
    unsupported()
}

pub async fn stop() -> Result<()> {
    unsupported()
}

pub async fn restart() -> Result<()> {
    unsupported()
}

pub async fn status() -> Result<()> {
    unsupported()
}

fn unsupported<T>() -> Result<T> {
    anyhow::bail!("service management is not supported on this platform yet")
}
