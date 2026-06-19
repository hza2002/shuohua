use anyhow::{Context, Result};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::hotkey::{Bindings, HotkeyAction, RawEvent, Suppressor, TrackerSet};
use crate::overlay::{OverlayCmd, OverlayHandle, TextKind};
use crate::state::StateStore;
use crate::voice::finish::SessionParams;
use crate::voice::SessionControl;

pub fn run_smart_fallback() -> Result<()> {
    let socket = crate::ipc::server::default_socket_path();
    if !socket_accepts_connections(&socket) {
        let stderr = smart_fallback_log("smart.stderr.log")?;
        let stdout = smart_fallback_log("smart.stdout.log")?;
        let child = Command::new(std::env::current_exe().context("resolve current exe")?)
            .arg("--daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .context("spawn shuo --daemon")?;
        drop(child);
        wait_for_socket(&socket, Duration::from_secs(2))?;
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create TUI runtime")?;
    rt.block_on(crate::tui::run())
}

fn smart_fallback_log(name: &str) -> Result<std::fs::File> {
    let dir = crate::state::history::state_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("create state dir {}", dir.display()))?;
    let path = dir.join(name);
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))
}

fn socket_accepts_connections(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

fn wait_for_socket(path: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if socket_accepts_connections(path) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("daemon did not accept UDS connections within {:?}", timeout)
}

struct DaemonLock(std::fs::File);

impl DaemonLock {
    fn acquire() -> Result<Self> {
        let dir = crate::state::history::state_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create state dir {}", dir.display()))?;
        let path = dir.join("daemon.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("open daemon lock {}", path.display()))?;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            anyhow::bail!("another shuo daemon is already starting or running");
        }
        Ok(Self(file))
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

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
            if let Err(e) = rt.block_on(run_daemon(
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

async fn run_daemon(
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
        },
    ));

    crate::reload::spawn_overlay(cfg_rx.clone(), overlay.clone());
    crate::reload::spawn_i18n(cfg_rx.clone(), overlay.clone());
    let (hotkey_tx, mut hotkey_rx) = tokio::sync::mpsc::unbounded_channel::<Bindings>();
    crate::reload::spawn_hotkey(cfg_rx.clone(), hotkey_tx);

    let (pipe_reader, pipe_writer) = os_pipe::pipe().context("create hotkey pipe")?;
    let suppressor = build_suppressor(&initial_hotkeys)?;
    let suppressor_for_tap = suppressor.clone();

    thread::Builder::new()
        .name("hotkey-eventtap".into())
        .spawn(move || {
            if let Err(e) = crate::hotkey::provider_darwin::run(pipe_writer, suppressor_for_tap) {
                tracing::error!(error = ?e, "hotkey event tap exited");
                std::process::exit(2);
            }
        })
        .context("spawn hotkey thread")?;

    let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel::<RawEvent>();
    thread::Builder::new()
        .name("hotkey-pipe-bridge".into())
        .spawn(move || pipe_to_mpsc(pipe_reader, raw_tx))
        .context("spawn hotkey bridge thread")?;

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
                update_suppressor_bindings(&suppressor, &new_hotkeys)?;
                continue;
            }
            maybe_raw = raw_rx.recv() => {
                let Some(ev) = maybe_raw else {
                    anyhow::bail!("hotkey bridge channel closed");
                };
                match hotkey_trackers.on_event(ev, Instant::now()) {
                    Some(HotkeyAction::CancelRecord) => {
                        clear_finished_session(&mut active, &suppressor);
                        if let Some(session) = active.as_ref() {
                            let _ = session.control.send(SessionControl::Cancel);
                        }
                        overlay.send(OverlayCmd::Dismiss);
                        continue;
                    }
                    Some(HotkeyAction::ToggleRecord) => {}
                    None => continue,
                }
                clear_finished_session(&mut active, &suppressor);
                match active.as_ref() {
                    None => {
                        match prepare_session_start(
                            &cfg_rx.borrow(),
                            post_app_context(),
                            state_store.clone(),
                            overlay.clone(),
                            crate::asr::providers::build,
                        ) {
                            Ok(start) => {
                                let suppressor_for_task = suppressor.clone();
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
                                active = Some(ActiveSession {
                                    control: start.control_tx,
                                    join,
                                });
                                if let Ok(mut s) = suppressor.lock() {
                                    s.set_cancel_active(true);
                                }
                            }
                            Err(error) => {
                                tracing::warn!(error = ?error, "session start failed");
                                state_store.set_error(None);
                                send_start_error_overlay(&overlay, error);
                                continue;
                            }
                        }
                    }
                    Some(session) => {
                        let _ = session.control.send(SessionControl::Stop);
                    }
                }
            }
        }
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

fn update_suppressor_bindings(
    suppressor: &Arc<Mutex<Suppressor>>,
    hotkeys: &Bindings,
) -> Result<()> {
    let new_trigger = hotkeys
        .combo_for(HotkeyAction::ToggleRecord)
        .context("missing toggle-record hotkey binding")?
        .clone();
    let new_cancel = hotkeys
        .combo_for(HotkeyAction::CancelRecord)
        .context("missing cancel-record hotkey binding")?
        .clone();
    if let Ok(mut s) = suppressor.lock() {
        s.set_trigger(new_trigger);
        s.set_cancel(new_cancel);
    }
    Ok(())
}

struct ActiveSession {
    control: tokio::sync::watch::Sender<SessionControl>,
    join: tokio::task::JoinHandle<()>,
}

fn clear_finished_session(active: &mut Option<ActiveSession>, suppressor: &Arc<Mutex<Suppressor>>) {
    if active
        .as_ref()
        .is_some_and(|session| session.join.is_finished())
    {
        *active = None;
        if let Ok(mut s) = suppressor.lock() {
            s.set_cancel_active(false);
        }
    }
}

fn post_app_context() -> crate::post::AppContext {
    crate::post::app_context::frontmost_app()
}

struct SessionStart {
    provider: Arc<dyn crate::asr::AsrProvider>,
    params: SessionParams,
    control_tx: tokio::sync::watch::Sender<SessionControl>,
    control_rx: tokio::sync::watch::Receiver<SessionControl>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionStartError {
    Profile,
    PostChainLoad,
    PostChainBuild,
    AsrProvider,
}

impl SessionStartError {
    fn i18n_key(self) -> &'static str {
        match self {
            Self::Profile => "error.profile_load",
            Self::PostChainLoad => "error.post_chain_load",
            Self::PostChainBuild => "error.post_chain_build",
            Self::AsrProvider => "error.asr_provider_init",
        }
    }
}

fn prepare_session_start(
    runtime_cfg: &crate::reload::Cfg,
    start_app_context: crate::post::AppContext,
    state_store: StateStore,
    overlay: OverlayHandle,
    build_provider: impl Fn(&str, &toml::value::Table) -> Result<crate::asr::providers::ProviderRuntime>,
) -> std::result::Result<SessionStart, SessionStartError> {
    let cfg = &runtime_cfg.config;
    let profile = crate::config::profile::load_for_app(
        &crate::config::profile::default_dir(),
        &cfg.profile,
        start_app_context.bundle_id.as_deref(),
    )
    .map_err(|error| {
        tracing::warn!(error = ?error, "profile load failed");
        SessionStartError::Profile
    })?;

    let post_dir = crate::config::post::default_dir();
    let post_chain_config = crate::config::post::load_components(
        &profile.post.chain,
        &crate::config::post::PostDirs {
            rule: post_dir.join("rule"),
            llm: post_dir.join("llm"),
        },
        &profile.post.llm,
    )
    .map_err(|error| {
        tracing::warn!(error = ?error, "post chain load failed");
        SessionStartError::PostChainLoad
    })?;

    let post_chain = crate::post::build_chain(post_chain_config).map_err(|error| {
        tracing::warn!(error = ?error, "post chain build failed");
        SessionStartError::PostChainBuild
    })?;

    let runtime =
        build_provider(&profile.asr.provider, &profile.asr.overrides).map_err(|error| {
            tracing::error!(error = ?error, "ASR provider init failed");
            SessionStartError::AsrProvider
        })?;

    let (control_tx, control_rx) = tokio::sync::watch::channel(SessionControl::Idle);
    Ok(SessionStart {
        provider: runtime.provider,
        params: SessionParams {
            auto_paste: cfg.voice.auto_paste,
            record_audio: cfg.voice.record_audio,
            vad_trace: cfg.dev.vad_trace,
            idle_pause: runtime.options.idle_pause,
            finalize_timeout_ms: runtime.options.finalize_timeout_ms,
            vad: cfg.voice.vad.clone(),
            stop_delay_ms: cfg.voice.stop_delay_ms,
            hotwords: profile.asr.hotwords,
            start_app_context,
            post_chain,
            post_timeout_ms: cfg.post.timeout_ms,
            overlay: Some(overlay),
            state: state_store,
        },
        control_tx,
        control_rx,
    })
}

fn send_start_error_overlay(overlay: &OverlayHandle, error: SessionStartError) {
    overlay.send(OverlayCmd::SetText {
        text: crate::i18n::tr(error.i18n_key(), &[]),
        kind: TextKind::Error,
    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Mutex as StdMutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| StdMutex::new(())).lock().unwrap()
    }

    fn temp_config_home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shuohua-daemon-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_minimal_config(root: &Path, profile_body: &str) {
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::write(
            root.join("config.toml"),
            r#"
[hotkey]
trigger = "f16"

[profile]
default = "default"
"#,
        )
        .unwrap();
        fs::write(root.join("profile/default.toml"), profile_body).unwrap();
    }

    fn fake_runtime(
        provider: Arc<dyn crate::asr::AsrProvider>,
    ) -> crate::asr::providers::ProviderRuntime {
        crate::asr::providers::ProviderRuntime {
            provider,
            options: crate::asr::providers::ProviderOptions {
                idle_pause: true,
                finalize_timeout_ms: 1234,
            },
        }
    }

    #[test]
    fn session_start_error_maps_to_i18n_keys() {
        assert_eq!(SessionStartError::Profile.i18n_key(), "error.profile_load");
        assert_eq!(
            SessionStartError::PostChainLoad.i18n_key(),
            "error.post_chain_load"
        );
        assert_eq!(
            SessionStartError::PostChainBuild.i18n_key(),
            "error.post_chain_build"
        );
        assert_eq!(
            SessionStartError::AsrProvider.i18n_key(),
            "error.asr_provider_init"
        );
    }

    #[test]
    fn start_error_overlay_sends_localized_error_text() {
        crate::i18n::init("en-US");
        let (overlay, mut rx) = OverlayHandle::channel();

        send_start_error_overlay(&overlay, SessionStartError::Profile);

        match rx.try_recv().unwrap() {
            OverlayCmd::SetText { text, kind } => {
                assert_eq!(kind, TextKind::Error);
                assert_eq!(text, "Profile could not be loaded");
            }
            other => panic!("unexpected overlay command: {other:?}"),
        }
    }

    #[test]
    fn prepare_session_start_builds_params_from_profile_and_runtime_options() {
        let _guard = env_lock();
        let config_home = temp_config_home();
        let root = config_home.join("shuohua");
        write_minimal_config(
            &root,
            r#"
name = "default"

[asr]
provider = "fake"
hotwords = ["Rust", "macOS"]

[post]
chain = []
"#,
        );
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        let cfg = Arc::new(crate::reload::RuntimeConfig {
            config: crate::config::load_from(&root.join("config.toml")).unwrap(),
            theme: crate::config::theme::EffectiveTheme::default(),
            theme_warning: None,
        });
        let (overlay, _rx) = OverlayHandle::channel();

        let start = prepare_session_start(
            &cfg,
            crate::post::AppContext {
                bundle_id: Some("com.example.App".to_string()),
                app_name: Some("Example".to_string()),
            },
            StateStore::new(),
            overlay,
            |name, overrides| {
                assert_eq!(name, "fake");
                assert!(overrides.is_empty());
                Ok(fake_runtime(
                    Arc::new(crate::asr::fake::FakeProvider::new()),
                ))
            },
        )
        .unwrap();

        assert_eq!(start.params.hotwords, ["Rust", "macOS"]);
        assert!(start.params.idle_pause);
        assert_eq!(start.params.finalize_timeout_ms, 1234);
        assert_eq!(start.params.post_timeout_ms, 10_000);
        assert_eq!(
            start.params.start_app_context.app_name.as_deref(),
            Some("Example")
        );

        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = fs::remove_dir_all(config_home);
    }

    #[test]
    fn prepare_session_start_classifies_provider_build_failure() {
        let _guard = env_lock();
        let config_home = temp_config_home();
        let root = config_home.join("shuohua");
        write_minimal_config(
            &root,
            r#"
name = "default"

[asr]
provider = "fake"

[post]
chain = []
"#,
        );
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        let cfg = Arc::new(crate::reload::RuntimeConfig {
            config: crate::config::load_from(&root.join("config.toml")).unwrap(),
            theme: crate::config::theme::EffectiveTheme::default(),
            theme_warning: None,
        });
        let (overlay, _rx) = OverlayHandle::channel();

        let result = prepare_session_start(
            &cfg,
            crate::post::AppContext::default(),
            StateStore::new(),
            overlay,
            |_name, _overrides| anyhow::bail!("provider unavailable"),
        );
        let Err(error) = result else {
            panic!("provider build failure should reject session start");
        };

        assert_eq!(error, SessionStartError::AsrProvider);

        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = fs::remove_dir_all(config_home);
    }
}
