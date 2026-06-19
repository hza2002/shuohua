use anyhow::{Context, Result};
use std::thread;

use crate::daemon::lock::DaemonLock;
use crate::daemon::runtime;
use crate::hotkey::{Bindings, HotkeyAction};
use crate::overlay::OverlayHandle;
use crate::state::StateStore;

pub fn run_daemon_process() -> Result<()> {
    let _lock = DaemonLock::acquire()?;
    let _log_guard = crate::log::init_daemon().context("initialize daemon logger")?;
    let cfg_path = crate::config::default_path();
    let (overlay, overlay_rx) = OverlayHandle::channel();
    let (cfg_rx, reload_handle) =
        crate::reload::watch_with_handle(cfg_path.clone(), Some(overlay.clone()))
            .context("start config watcher")?;
    let runtime_cfg = cfg_rx.borrow().clone();
    let cfg = &runtime_cfg.config;
    crate::i18n::init(&cfg.ui.language);
    let hotkeys = Bindings::parse(&cfg.hotkey.trigger, &cfg.hotkey.cancel)?;
    let parsed_trigger = hotkeys
        .combo_for(HotkeyAction::ToggleRecord)
        .context("missing toggle-record hotkey binding")?;
    let parsed_cancel = hotkeys
        .combo_for(HotkeyAction::CancelRecord)
        .context("missing cancel-record hotkey binding")?;

    tracing::info!(
        config_path = %cfg_path.display(),
        trigger = %cfg.hotkey.trigger,
        cancel = %cfg.hotkey.cancel,
        parsed_trigger = %parsed_trigger,
        parsed_cancel = %parsed_cancel,
        post_timeout_ms = cfg.post.timeout_ms,
        auto_paste = cfg.voice.auto_paste,
        record_audio = %cfg.voice.record_audio,
        stop_delay_ms = cfg.voice.stop_delay_ms,
        vad_trace = cfg.dev.vad_trace,
        language = %cfg.ui.language,
        "daemon config loaded"
    );
    let state_store = StateStore::new();

    let cfg_rx_for_daemon = cfg_rx.clone();
    let reload_for_daemon = reload_handle.clone();
    let overlay_for_daemon = overlay.clone();
    let state_for_daemon = state_store.clone();
    thread::Builder::new()
        .name("tokio-daemon".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("create tokio runtime");
            if let Err(e) = rt.block_on(runtime::run_daemon(
                cfg_rx_for_daemon,
                reload_for_daemon,
                hotkeys,
                overlay_for_daemon,
                state_for_daemon,
            )) {
                tracing::error!(error = ?e, "daemon exited");
                std::process::exit(2);
            }
        })
        .context("spawn tokio daemon thread")?;

    crate::overlay::run(overlay_rx, runtime_cfg.theme.overlay.clone());
    Ok(())
}
