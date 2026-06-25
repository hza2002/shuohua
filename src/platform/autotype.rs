use anyhow::Result;

#[cfg(target_os = "macos")]
pub fn paste() -> Result<()> {
    crate::platform::macos::autotype::paste()
}

#[cfg(target_os = "windows")]
pub fn paste() -> Result<()> {
    crate::platform::windows::autotype::paste()
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn paste() -> Result<()> {
    anyhow::bail!("autotype paste is not implemented on this platform")
}
