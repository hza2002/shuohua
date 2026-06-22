use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::hotkey::Suppressor;
use crate::post::AppContext;

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
    fn frontmost_app(&self) -> AppContext {
        crate::platform::desktop::frontmost_app()
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
