use anyhow::Result;

#[cfg(target_os = "macos")]
pub fn paste() -> Result<()> {
    crate::platform::macos::autotype::paste()
}

#[cfg(not(target_os = "macos"))]
pub fn paste() -> Result<()> {
    anyhow::bail!("autotype paste is not implemented on this platform")
}
