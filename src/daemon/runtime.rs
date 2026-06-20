use anyhow::Result;
use std::time::Instant;

use crate::daemon::active_session::ActiveSession;
use crate::daemon::hotkey_input::HotkeyInput;
use crate::daemon::session_start;
use crate::hotkey::{Bindings, HotkeyAction, TrackerSet};
use crate::overlay::{OverlayCmd, OverlayHandle};
use crate::platform::daemon::{DaemonPlatform, SystemDaemonPlatform};
use crate::state::StateStore;

pub(super) async fn run_daemon(
    cfg_rx: crate::reload::Rx,
    reload_handle: crate::reload::Handle,
    initial_hotkeys: Bindings,
    overlay: OverlayHandle,
    state_store: StateStore,
) -> Result<()> {
    run_daemon_with_platform(
        SystemDaemonPlatform,
        cfg_rx,
        reload_handle,
        initial_hotkeys,
        overlay,
        state_store,
    )
    .await
}

async fn run_daemon_with_platform(
    platform: impl DaemonPlatform,
    cfg_rx: crate::reload::Rx,
    reload_handle: crate::reload::Handle,
    initial_hotkeys: Bindings,
    overlay: OverlayHandle,
    state_store: StateStore,
) -> Result<()> {
    let listener = crate::ipc::server::bind_default().await?;
    let socket_path = crate::ipc::server::default_socket_path();
    tokio::spawn(crate::ipc::server::run(
        listener,
        state_store.clone(),
        crate::ipc::server::ServerControl {
            reload: reload_handle,
            started_at: Instant::now(),
            shutdown: {
                let overlay = overlay.clone();
                std::sync::Arc::new(move || {
                    tracing::info!("shutdown requested over IPC");
                    overlay.send(OverlayCmd::Quit);
                })
            },
        },
    ));

    crate::reload::spawn_overlay(cfg_rx.clone(), overlay.clone());
    crate::reload::spawn_i18n(cfg_rx.clone(), overlay.clone());
    let (hotkey_tx, mut hotkey_rx) = tokio::sync::mpsc::unbounded_channel::<Bindings>();
    crate::reload::spawn_hotkey(cfg_rx.clone(), hotkey_tx);

    let mut hotkey_input = HotkeyInput::spawn(&platform, &initial_hotkeys)?;

    tracing::info!(
        uds = %socket_path.display(),
        trigger = %cfg_rx.borrow().config.hotkey.trigger,
        "daemon ready"
    );

    let mut hotkey_trackers = TrackerSet::new(&initial_hotkeys);
    let mut active: Option<ActiveSession> = None;

    loop {
        tokio::select! {
            Some(new_hotkeys) = hotkey_rx.recv() => {
                hotkey_trackers.set_bindings(&new_hotkeys);
                hotkey_input.update_bindings(&new_hotkeys)?;
                continue;
            }
            maybe_raw = hotkey_input.raw_rx.recv() => {
                let Some(ev) = maybe_raw else {
                    anyhow::bail!("hotkey bridge channel closed");
                };
                match hotkey_trackers.on_event(ev, Instant::now()) {
                    Some(HotkeyAction::CancelRecord) => {
                        clear_finished_session(&mut active, &hotkey_input);
                        if let Some(session) = active.as_ref() {
                            session.cancel();
                        }
                        overlay.send(OverlayCmd::Dismiss);
                        continue;
                    }
                    Some(HotkeyAction::ToggleRecord) => {}
                    None => continue,
                }
                clear_finished_session(&mut active, &hotkey_input);
                match active.as_ref() {
                    None => {
                        match session_start::prepare(
                            &cfg_rx.borrow(),
                            platform.frontmost_app(),
                            state_store.clone(),
                            overlay.clone(),
                            crate::asr::providers::build,
                        ) {
                            Ok(start) => {
                                let suppressor_for_task = hotkey_input.suppressor();
                                let join = tokio::spawn(async move {
                                    crate::voice::finish::run_recording(
                                        start.provider.as_ref(),
                                        start.params,
                                        start.control_rx,
                                    )
                                    .await;
                                    if let Ok(mut s) = suppressor_for_task.lock() {
                                        s.set_cancel_active(false);
                                    }
                                });
                                active = Some(ActiveSession::new(start.control_tx, join));
                                hotkey_input.set_cancel_active(true);
                            }
                            Err(error) => {
                                tracing::warn!(error = ?error, "session start failed");
                                state_store.set_error(None);
                                session_start::send_error_overlay(&overlay, error);
                                continue;
                            }
                        }
                    }
                    Some(session) => {
                        session.stop();
                    }
                }
            }
        }
    }
}

fn clear_finished_session(active: &mut Option<ActiveSession>, hotkey_input: &HotkeyInput) {
    if active.as_ref().is_some_and(ActiveSession::is_finished) {
        *active = None;
        hotkey_input.set_cancel_active(false);
    }
}
