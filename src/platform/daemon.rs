use anyhow::Result;
use std::sync::{Arc, Mutex};

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

    fn spawn_hotkey_event_tap(
        &self,
        writer: os_pipe::PipeWriter,
        suppressor: Arc<Mutex<Suppressor>>,
    ) -> Result<()> {
        crate::platform::hotkey::spawn_event_tap(writer, suppressor)
    }
}
