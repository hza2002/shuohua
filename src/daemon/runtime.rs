use anyhow::{anyhow, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::daemon::active_session::{ActiveSession, ShutdownStopResult};
use crate::daemon::hotkey_input::HotkeyInput;
use crate::daemon::resume::ResumeDecision;
use crate::daemon::session_start;
use crate::history::HistoryService;
use crate::hotkey::{Bindings, HotkeyAction, TrackerSet};
use crate::overlay::{OverlayAction, OverlayActionReceiver, OverlayCmd, OverlayHandle};
use crate::platform::daemon::{DaemonPlatform, SystemDaemonPlatform};
use crate::state::StateStore;

const SHUTDOWN_ACTIVE_SESSION_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) async fn run_daemon(
    cfg_rx: crate::reload::Rx,
    reload_handle: crate::reload::Handle,
    initial_hotkeys: Bindings,
    overlay: OverlayHandle,
    overlay_actions: OverlayActionReceiver,
    state_store: StateStore,
    overlay_on_screen: Arc<AtomicBool>,
) -> Result<()> {
    run_daemon_with_platform(
        SystemDaemonPlatform,
        cfg_rx,
        reload_handle,
        initial_hotkeys,
        overlay,
        overlay_actions,
        state_store,
        overlay_on_screen,
    )
    .await
}

// Daemon setup seam: wires together config/reload/overlay/state/hotkey plus the
// ESC-suppression visibility flag. Plain dependency injection, not business logic.
#[allow(clippy::too_many_arguments)]
async fn run_daemon_with_platform(
    platform: impl DaemonPlatform,
    cfg_rx: crate::reload::Rx,
    reload_handle: crate::reload::Handle,
    initial_hotkeys: Bindings,
    overlay: OverlayHandle,
    mut overlay_actions: OverlayActionReceiver,
    state_store: StateStore,
    overlay_on_screen: Arc<AtomicBool>,
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
    let reload_for_overlay_actions = reload_handle.clone();
    tokio::spawn(async move {
        while let Some(action) = overlay_actions.recv().await {
            handle_overlay_action(action, &reload_for_overlay_actions).await;
        }
    });
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

    let mut hotkey_input =
        HotkeyInput::spawn(&platform, &initial_hotkeys, overlay_on_screen.clone())?;

    tracing::info!(
        uds = %socket_path.display(),
        trigger = %cfg_rx.borrow().config.hotkey.trigger,
        "daemon ready"
    );

    let mut current_hotkeys = initial_hotkeys.clone();
    let mut hotkey_trackers = TrackerSet::new(&current_hotkeys);
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
                current_hotkeys = new_hotkeys;
                continue;
            }
            maybe_raw = hotkey_input.raw_rx.recv() => {
                let Some(ev) = maybe_raw else {
                    anyhow::bail!("hotkey bridge channel closed");
                };
                match hotkey_trackers.on_event(ev, Instant::now()) {
                    Some(HotkeyAction::Cancel) => {
                        clear_finished_session(&mut active, &hotkey_input);
                        // Cancel a live session if any; dismiss the overlay if
                        // anything is on screen (session OR a lingering error
                        // panel). Idle (neither) → no-op so ESC passes through.
                        let cancelled = handle_cancel_hotkey(active.as_ref());
                        if cancelled || overlay_on_screen.load(Ordering::Relaxed) {
                            tracing::debug!(
                                action = "cancel_record",
                                combo = %hotkey_combo(&current_hotkeys, HotkeyAction::Cancel),
                                "hotkey action triggered"
                            );
                            overlay.send(OverlayCmd::Dismiss);
                        }
                        continue;
                    }
                    Some(HotkeyAction::Toggle) => {
                        tracing::debug!(
                            action = "toggle_record",
                            combo = %hotkey_combo(&current_hotkeys, HotkeyAction::Toggle),
                            "hotkey action triggered"
                        );
                    }
                    Some(HotkeyAction::Resume) => {
                        clear_finished_session(&mut active, &hotkey_input);
                        if active.is_some() {
                            tracing::debug!(
                                action = "resume_record",
                                combo = %hotkey_combo(&current_hotkeys, HotkeyAction::Resume),
                                "resume_ignored_active_session"
                            );
                            continue;
                        }
                        let decision = match crate::daemon::resume::latest_decision(history.clone()).await {
                            Ok(decision) => decision,
                            Err(error) => {
                                tracing::warn!(error = ?error, "resume history lookup failed; starting new recording");
                                ResumeDecision::New
                            }
                        };
                        // resume notice + seed 文本回显由录音引擎在 SetState
                        // (Connecting) 清屏之后统一发（见 engine::apply_start_notice），
                        // 这里再发 overlay Notice 会被随后的 Connecting 清掉。
                        let start = match decision {
                            ResumeDecision::ResumeSeed { source_id, text } => {
                                tracing::debug!(
                                    action = "resume_record",
                                    combo = %hotkey_combo(&current_hotkeys, HotkeyAction::Resume),
                                    source_id,
                                    "hotkey action triggered"
                                );
                                crate::voice::resume::RecordingStart::Seed(
                                    crate::voice::resume::ResumeSeed { text },
                                )
                            }
                            ResumeDecision::New => {
                                tracing::debug!(
                                    action = "resume_record",
                                    combo = %hotkey_combo(&current_hotkeys, HotkeyAction::Resume),
                                    "resume hotkey starting new recording"
                                );
                                crate::voice::resume::RecordingStart::NewFromResume
                            }
                        };
                        match start_session(
                            &cfg_rx,
                            &platform,
                            state_store.clone(),
                            overlay.clone(),
                            history.clone(),
                            &hotkey_input,
                            start,
                        ) {
                            Ok(session) => active = Some(session),
                            Err(error) => {
                                tracing::warn!(error = ?error, "session start failed");
                                state_store.set_error(None);
                                session_start::send_error_overlay(&overlay, error);
                            }
                        }
                        continue;
                    }
                    None => continue,
                }
                clear_finished_session(&mut active, &hotkey_input);
                match active.as_ref() {
                    None => {
                        match start_session(
                            &cfg_rx,
                            &platform,
                            state_store.clone(),
                            overlay.clone(),
                            history.clone(),
                            &hotkey_input,
                            crate::voice::resume::RecordingStart::Fresh,
                        ) {
                            Ok(session) => active = Some(session),
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

fn start_session(
    cfg_rx: &crate::reload::Rx,
    platform: &impl DaemonPlatform,
    state_store: StateStore,
    overlay: OverlayHandle,
    history: HistoryService,
    hotkey_input: &HotkeyInput,
    start: crate::voice::resume::RecordingStart,
) -> std::result::Result<ActiveSession, session_start::SessionStartError> {
    let prepared = session_start::prepare(
        &cfg_rx.borrow(),
        platform.frontmost_app(),
        state_store,
        overlay,
        start,
        crate::asr::providers::build_instance,
    )?;
    let suppressor_for_task = hotkey_input.suppressor();
    let control = prepared.control;
    let task_control = control.clone();
    let join = tokio::spawn(async move {
        crate::voice::finish::run_recording(
            prepared.provider.as_ref(),
            prepared.params,
            task_control,
            history,
        )
        .await;
        if let Ok(mut s) = suppressor_for_task.lock() {
            s.set_cancel_active(false);
        }
    });
    hotkey_input.set_cancel_active(true);
    Ok(ActiveSession::new(control, join))
}

async fn handle_overlay_action(action: OverlayAction, reload: &crate::reload::Handle) {
    match action {
        OverlayAction::BindProfile { bundle_id, profile } => {
            let path = crate::config::default_path();
            if let Err(error) =
                crate::config::profile_write::bind_profile_route(&path, &bundle_id, &profile)
            {
                tracing::warn!(
                    error = ?error,
                    bundle_id,
                    profile,
                    "bind profile route failed"
                );
                return;
            }
            if let Err(error) = reload.reload_now() {
                tracing::warn!(error = ?error, "reload after profile route bind failed");
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

fn handle_cancel_hotkey(active: Option<&ActiveSession>) -> bool {
    let Some(session) = active else {
        return false;
    };
    session.cancel();
    true
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

fn hotkey_combo(bindings: &Bindings, action: HotkeyAction) -> String {
    bindings
        .combo_for(action)
        .map(ToString::to_string)
        .unwrap_or_else(|| "<missing>".to_string())
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
        let control = SessionControl::new();
        let task_control = control.clone();
        let join = tokio::spawn(async move {
            task_control.stopped().await;
            assert!(task_control.is_stop_requested());
        });
        let mut active = Some(ActiveSession::new(control, join));

        let stopped = shutdown_active_session(&mut active, Duration::from_millis(100)).await;

        assert!(stopped);
        assert!(active.is_none());
    }

    #[test]
    fn cancel_hotkey_is_ignored_without_active_session() {
        assert!(!handle_cancel_hotkey(None));
    }

    #[tokio::test]
    async fn cancel_hotkey_cancels_active_session() {
        let control = SessionControl::new();
        let active = ActiveSession::new(control.clone(), tokio::spawn(std::future::pending()));

        assert!(handle_cancel_hotkey(Some(&active)));
        assert!(control.is_cancelled());
    }
}
