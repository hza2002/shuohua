use anyhow::{anyhow, Result};
use std::time::{Duration, Instant};

use crate::daemon::active_session::{ActiveSession, ShutdownStopResult};
use crate::daemon::hotkey_input::HotkeyInput;
use crate::daemon::session_start;
use crate::hotkey::{Bindings, HotkeyAction, TrackerSet};
use crate::overlay::{OverlayCmd, OverlayHandle};
use crate::platform::daemon::{DaemonPlatform, SystemDaemonPlatform};
use crate::state::StateStore;

const SHUTDOWN_ACTIVE_SESSION_TIMEOUT: Duration = Duration::from_secs(15);

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
    let history = crate::history::HistoryService::new();
    let _history_watcher = match history.watch() {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            tracing::warn!(error = ?error, "history watcher unavailable");
            None
        }
    };
    let listener = crate::ipc::server::bind_default().await?;
    let socket_path = crate::ipc::server::default_socket_path();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let mut ipc_task = tokio::spawn(crate::ipc::server::run(
        listener,
        state_store.clone(),
        history.clone(),
        crate::ipc::server::ServerControl {
            reload: reload_handle,
            started_at: Instant::now(),
            shutdown: shutdown_tx,
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
            biased;
            ipc_result = &mut ipc_task => {
                return Err(classify_ipc_exit(ipc_result));
            }
            changed = shutdown_rx.changed() => {
                if changed.is_ok() && *shutdown_rx.borrow_and_update() {
                    tracing::info!("shutdown requested over IPC");
                    let _ = shutdown_active_session(
                        &mut active,
                        SHUTDOWN_ACTIVE_SESSION_TIMEOUT,
                    ).await;
                    hotkey_input.set_cancel_active(false);
                    overlay.send(OverlayCmd::Quit);
                    return Ok(());
                }
            }
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
                                let history_for_task = history.clone();
                                let join = tokio::spawn(async move {
                                    crate::voice::finish::run_recording(
                                        start.provider.as_ref(),
                                        start.params,
                                        start.control_rx,
                                        history_for_task,
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

fn classify_ipc_exit(
    result: std::result::Result<Result<()>, tokio::task::JoinError>,
) -> anyhow::Error {
    match result {
        Ok(Ok(())) => anyhow!("IPC server exited unexpectedly"),
        Ok(Err(error)) => error.context("IPC server failed"),
        Err(error) if error.is_panic() => anyhow!("IPC server task panicked: {error}"),
        Err(error) => anyhow!("IPC server task failed: {error}"),
    }
}

fn clear_finished_session(active: &mut Option<ActiveSession>, hotkey_input: &HotkeyInput) {
    if active.as_ref().is_some_and(ActiveSession::is_finished) {
        *active = None;
        hotkey_input.set_cancel_active(false);
    }
}

async fn shutdown_active_session(active: &mut Option<ActiveSession>, timeout: Duration) -> bool {
    let Some(session) = active.take() else {
        return true;
    };
    let mut session = session;
    match session.stop_and_join(timeout).await {
        ShutdownStopResult::Stopped => true,
        ShutdownStopResult::JoinError(error) => {
            tracing::warn!(error = ?error, "active recording task failed during shutdown");
            true
        }
        ShutdownStopResult::TimedOut => {
            tracing::warn!(
                timeout_ms = timeout.as_millis(),
                "active recording did not stop before shutdown timeout"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::SessionControl;
    use std::time::Duration;

    #[test]
    fn ipc_server_unexpected_ok_is_fatal() {
        let error = classify_ipc_exit(Ok(Ok(())));

        assert!(
            error.to_string().contains("IPC server exited unexpectedly"),
            "{error:#}"
        );
    }

    #[test]
    fn ipc_server_error_keeps_context() {
        let error = classify_ipc_exit(Ok(Err(anyhow::anyhow!("accept failed"))));

        assert!(error.to_string().contains("IPC server failed"), "{error:#}");
        assert!(format!("{error:#}").contains("accept failed"), "{error:#}");
    }

    #[tokio::test]
    async fn ipc_server_panic_is_fatal() {
        let join = tokio::spawn(async {
            panic!("boom");
            #[allow(unreachable_code)]
            Ok(())
        });

        let error = classify_ipc_exit(join.await);

        assert!(error.to_string().contains("panicked"), "{error:#}");
    }

    #[tokio::test]
    async fn ipc_ready_wins_over_shutdown_ready() {
        let mut shutdown = std::future::ready(());
        let mut ipc = std::future::ready(());

        let winner = tokio::select! {
            biased;
            _ = &mut ipc => "ipc",
            _ = &mut shutdown => "shutdown",
        };

        assert_eq!(winner, "ipc");
    }

    #[tokio::test]
    async fn shutdown_active_session_sends_stop_and_waits_for_completion() {
        let (control_tx, mut control_rx) = tokio::sync::watch::channel(SessionControl::Idle);
        let join = tokio::spawn(async move {
            control_rx.changed().await.unwrap();
            assert_eq!(*control_rx.borrow_and_update(), SessionControl::Stop);
        });
        let mut active = Some(ActiveSession::new(control_tx, join));

        let stopped = shutdown_active_session(&mut active, Duration::from_millis(100)).await;

        assert!(stopped);
        assert!(active.is_none());
    }
}
