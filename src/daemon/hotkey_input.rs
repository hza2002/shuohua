use anyhow::{Context, Result};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::hotkey::{Bindings, HotkeyAction, RawEvent, Suppressor};
use crate::platform::daemon::DaemonPlatform;

pub(super) struct HotkeyInput {
    pub(super) raw_rx: tokio::sync::mpsc::UnboundedReceiver<RawEvent>,
    suppressor: Arc<Mutex<Suppressor>>,
}

impl HotkeyInput {
    pub(super) fn spawn(
        platform: &impl DaemonPlatform,
        initial_hotkeys: &Bindings,
    ) -> Result<Self> {
        let (pipe_reader, pipe_writer) = os_pipe::pipe().context("create hotkey pipe")?;
        let suppressor = build_suppressor(initial_hotkeys)?;
        platform.spawn_hotkey_event_tap(pipe_writer, suppressor.clone())?;

        let (raw_tx, raw_rx) = tokio::sync::mpsc::unbounded_channel::<RawEvent>();
        thread::Builder::new()
            .name("hotkey-pipe-bridge".into())
            .spawn(move || pipe_to_mpsc(pipe_reader, raw_tx))
            .context("spawn hotkey bridge thread")?;

        Ok(Self { raw_rx, suppressor })
    }

    pub(super) fn update_bindings(&self, hotkeys: &Bindings) -> Result<()> {
        let new_trigger = hotkeys
            .combo_for(HotkeyAction::ToggleRecord)
            .context("missing toggle-record hotkey binding")?
            .clone();
        let new_cancel = hotkeys
            .combo_for(HotkeyAction::CancelRecord)
            .context("missing cancel-record hotkey binding")?
            .clone();
        if let Ok(mut s) = self.suppressor.lock() {
            s.set_trigger(new_trigger);
            s.set_cancel(new_cancel);
        }
        Ok(())
    }

    pub(super) fn set_cancel_active(&self, active: bool) {
        if let Ok(mut s) = self.suppressor.lock() {
            s.set_cancel_active(active);
        }
    }

    pub(super) fn suppressor(&self) -> Arc<Mutex<Suppressor>> {
        self.suppressor.clone()
    }
}

fn build_suppressor(hotkeys: &Bindings) -> Result<Arc<Mutex<Suppressor>>> {
    let initial_trigger = hotkeys
        .combo_for(HotkeyAction::ToggleRecord)
        .context("missing toggle-record hotkey binding")?
        .clone();
    let initial_cancel = hotkeys
        .combo_for(HotkeyAction::CancelRecord)
        .context("missing cancel-record hotkey binding")?
        .clone();
    let mut initial_suppressor = Suppressor::new(initial_trigger);
    initial_suppressor.set_cancel(initial_cancel);
    Ok(Arc::new(Mutex::new(initial_suppressor)))
}

fn pipe_to_mpsc(mut reader: os_pipe::PipeReader, tx: tokio::sync::mpsc::UnboundedSender<RawEvent>) {
    let mut buf = [0u8; 4];
    loop {
        if let Err(e) = reader.read_exact(&mut buf) {
            tracing::error!(error = %e, "hotkey pipe read failed");
            return;
        }
        let Some(ev) = RawEvent::decode(buf) else {
            tracing::warn!(frame = ?buf, "dropped unknown hotkey frame");
            continue;
        };
        if tx.send(ev).is_err() {
            return;
        }
    }
}
