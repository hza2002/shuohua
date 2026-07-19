use anyhow::Result;
use parking_lot::Mutex;
use std::sync::Arc;
use std::thread;

use super::AppContext;
use crate::hotkey::Suppressor;

pub(crate) trait DaemonPlatform {
    fn frontmost_app(&self) -> AppContext;
    fn spawn_hotkey_event_tap(
        &self,
        writer: os_pipe::PipeWriter,
        suppressor: Arc<Mutex<Suppressor>>,
    ) -> Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemDaemonPlatform;

impl DaemonPlatform for SystemDaemonPlatform {
    #[cfg(target_os = "macos")]
    fn frontmost_app(&self) -> AppContext {
        crate::platform::macos::app_context::frontmost_app()
    }

    #[cfg(not(target_os = "macos"))]
    fn frontmost_app(&self) -> AppContext {
        AppContext::default()
    }

    #[cfg(target_os = "macos")]
    fn spawn_hotkey_event_tap(
        &self,
        writer: os_pipe::PipeWriter,
        suppressor: Arc<Mutex<Suppressor>>,
    ) -> Result<()> {
        thread::Builder::new()
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
    fn spawn_hotkey_event_tap(
        &self,
        _writer: os_pipe::PipeWriter,
        _suppressor: Arc<Mutex<Suppressor>>,
    ) -> Result<()> {
        anyhow::bail!("daemon hotkey event tap is not implemented on this platform")
    }
}
