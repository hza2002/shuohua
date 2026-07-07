use anyhow::Result;

#[cfg(target_os = "macos")]
pub fn write_string(text: &str) -> Result<()> {
    crate::platform::macos::clipboard::write_string(text)
}

#[cfg(target_os = "macos")]
pub fn read_string() -> Result<String> {
    crate::platform::macos::clipboard::read_string()
}

#[cfg(not(target_os = "macos"))]
pub fn write_string(_text: &str) -> Result<()> {
    anyhow::bail!("clipboard is not implemented on this platform")
}

#[cfg(not(target_os = "macos"))]
pub fn read_string() -> Result<String> {
    anyhow::bail!("clipboard is not implemented on this platform")
}
