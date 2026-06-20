use anyhow::Result;

#[cfg(target_os = "macos")]
pub fn write_string(text: &str) -> Result<()> {
    crate::platform::macos::clipboard::write_string(text)
}

#[cfg(not(target_os = "macos"))]
pub fn write_string(_text: &str) -> Result<()> {
    anyhow::bail!("clipboard is not implemented on this platform")
}
