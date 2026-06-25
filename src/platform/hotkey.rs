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

#[cfg(target_os = "windows")]
pub(crate) fn spawn_event_tap(
    writer: os_pipe::PipeWriter,
    suppressor: Arc<Mutex<Suppressor>>,
) -> Result<()> {
    std::thread::Builder::new()
        .name("hotkey-wh-keyboard-ll".into())
        .spawn(move || {
            if let Err(e) = crate::hotkey::provider_windows::run(writer, suppressor) {
                tracing::error!(error = ?e, "windows hotkey hook exited");
                std::process::exit(2);
            }
        })?;
    Ok(())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub(crate) fn spawn_event_tap(
    writer: os_pipe::PipeWriter,
    _suppressor: Arc<Mutex<Suppressor>>,
) -> Result<()> {
    std::thread::Builder::new()
        .name("hotkey-unimplemented-idle".into())
        .spawn(move || {
            tracing::warn!(
                "daemon hotkey event tap is unsupported on this platform; keeping IPC runtime alive"
            );
            let _writer = writer;
            loop {
                std::thread::park();
            }
        })?;
    Ok(())
}
