use anyhow::Result;
use std::sync::{Arc, Mutex};

use crate::hotkey::Suppressor;

#[cfg(target_os = "macos")]
pub(crate) fn spawn_event_tap(
    writer: os_pipe::PipeWriter,
    suppressor: Arc<Mutex<Suppressor>>,
) -> Result<()> {
    std::thread::Builder::new()
        .name("hotkey-eventtap".into())
        .spawn(move || {
            if let Err(e) = crate::hotkey::provider_darwin::run(writer, suppressor) {
                tracing::error!(error = ?e, "hotkey event tap exited");
                std::process::exit(2);
            }
        })?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn spawn_event_tap(
    _writer: os_pipe::PipeWriter,
    _suppressor: Arc<Mutex<Suppressor>>,
) -> Result<()> {
    anyhow::bail!("daemon hotkey event tap is not implemented on this platform")
}
