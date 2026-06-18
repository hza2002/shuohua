//! 配置热重载。
//!
//! 流程：
//!   notify 监听 config 目录（不监听文件本身，避免编辑器 inode 替换）
//!     → 150 ms 合并连发 → parse → broadcast Arc<Config>
//!     → 各子系统的 subscriber 自取所需字段
//!
//! 跟其它模块的关系：本模块只发"现在的最新配置"。各子系统的 subscriber
//! 函数都在这里集中维护（每个就是一个 tokio task），但只通过对方暴露的
//! handle / sender 跟人家通信，不反向 import 业务逻辑。
//!
//! 显式不在范围内：
//!   - profile 的 ASR/post 组合切换（下次录音开始时读取）
//!   - `shuo doctor` / launchd 自启（跟 reload 无关）

use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio::sync::watch;

use crate::config::theme::EffectiveTheme;
use crate::config::{self, Config};
use crate::overlay::{OverlayCmd, OverlayHandle};

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub config: Config,
    pub theme: EffectiveTheme,
}

pub type Cfg = Arc<RuntimeConfig>;
pub type Rx = watch::Receiver<Cfg>;

#[derive(Clone)]
pub struct Handle {
    path: PathBuf,
    tx: watch::Sender<Cfg>,
}

impl Handle {
    pub fn reload_now(&self) -> Result<()> {
        load_and_broadcast(&self.path, &self.tx)
    }
}

/// 起 watcher 线程，返回带初值的 `watch::Receiver` 和手动 reload handle。
/// 初值 = `config::load_from(path)` + active theme.
pub fn watch_with_handle(path: PathBuf) -> Result<(Rx, Handle)> {
    let initial = Arc::new(load_runtime_config(&path).context("initial config load")?);
    let (tx, rx) = watch::channel(initial);

    let dir = path
        .parent()
        .context("config path has no parent dir")?
        .to_path_buf();
    let handle = Handle {
        path: path.clone(),
        tx: tx.clone(),
    };

    std::thread::Builder::new()
        .name("config-watcher".into())
        .spawn(move || {
            if let Err(e) = run_watcher(dir, path, tx) {
                tracing::error!(error = ?e, "config watcher exited");
            }
        })
        .context("spawn config-watcher thread")?;

    Ok((rx, handle))
}

fn run_watcher(dir: PathBuf, path: PathBuf, tx: watch::Sender<Cfg>) -> Result<()> {
    use notify::{RecursiveMode, Watcher};

    let (event_tx, event_rx) = std_mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = event_tx.send(res);
    })
    .context("create notify watcher")?;
    watcher
        .watch(&dir, RecursiveMode::Recursive)
        .with_context(|| format!("watch {}", dir.display()))?;

    let debounce = Duration::from_millis(150);
    loop {
        let event = match event_rx.recv() {
            Ok(Ok(e)) => e,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "notify error");
                continue;
            }
            Err(_) => return Ok(()),
        };
        if !event.paths.iter().any(|p| is_reload_relevant(&dir, p)) {
            continue;
        }
        while event_rx.recv_timeout(debounce).is_ok() {}

        match load_and_broadcast(&path, &tx) {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!(error = ?e, "config reload failed; keeping previous config");
            }
        }
    }
}

fn load_and_broadcast(path: &Path, tx: &watch::Sender<Cfg>) -> Result<()> {
    let cfg = load_runtime_config(path)?;
    tx.send(Arc::new(cfg)).context("broadcast config reload")?;
    tracing::info!(path = %path.display(), "config reloaded");
    Ok(())
}

fn load_runtime_config(path: &Path) -> Result<RuntimeConfig> {
    let config = config::load_from(path)?;
    let theme = config::theme::load_effective(&config, path);
    Ok(RuntimeConfig { config, theme })
}

fn is_reload_relevant(root: &Path, path: &Path) -> bool {
    if path.file_name().and_then(|name| name.to_str()) == Some("config.toml") {
        return true;
    }
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let mut components = relative.components();
    matches!(
        components.next().and_then(|c| c.as_os_str().to_str()),
        Some("theme")
    ) && path.extension().is_some_and(|ext| ext == "toml")
}

/// Overlay subscriber：`[overlay]` 段变化 → `OverlayCmd::ReloadConfig`。
pub fn spawn_overlay(mut rx: Rx, handle: OverlayHandle) {
    tokio::spawn(async move {
        let initial = rx.borrow().clone();
        let mut prev = (
            initial.config.overlay.clone(),
            initial.theme.overlay.clone(),
        );
        while rx.changed().await.is_ok() {
            let current = rx.borrow().clone();
            let next = (
                current.config.overlay.clone(),
                current.theme.overlay.clone(),
            );
            if next != prev {
                handle.send(OverlayCmd::ReloadConfig {
                    cfg: next.1.clone(),
                });
                prev = next;
            }
        }
    });
}

/// i18n subscriber：`ui.language` 变化 → 重置字典 + 推 Relabel 让 overlay 刷新当前 state label。
pub fn spawn_i18n(mut rx: Rx, handle: OverlayHandle) {
    tokio::spawn(async move {
        let mut prev = rx.borrow().config.ui.language.clone();
        while rx.changed().await.is_ok() {
            let next = rx.borrow().config.ui.language.clone();
            if next != prev {
                crate::i18n::init(&next);
                handle.send(OverlayCmd::Relabel);
                tracing::debug!(language = %next, "language changed");
                prev = next;
            }
        }
    });
}

/// Hotkey subscriber：`[hotkey]` 变化 → 重新 parse，成功则发新 binding 到 daemon
/// 主循环（主循环用 tokio::select 在 `RawEvent` 和这个 channel 之间多路复用，swap Tracker
/// 与 Suppressor）。parse 失败保留旧 trigger，只打日志。
pub fn spawn_hotkey(mut rx: Rx, combo_tx: mpsc::UnboundedSender<crate::HotkeyBindings>) {
    tokio::spawn(async move {
        let mut prev = rx.borrow().config.hotkey.clone();
        while rx.changed().await.is_ok() {
            let next = rx.borrow().config.hotkey.clone();
            if next.trigger == prev.trigger && next.cancel == prev.cancel {
                continue;
            }
            match crate::HotkeyBindings::parse(&next) {
                Ok(bindings) => {
                    let printed_trigger = bindings.trigger.to_string();
                    let printed_cancel = bindings.cancel.to_string();
                    if combo_tx.send(bindings).is_err() {
                        return;
                    }
                    tracing::debug!(
                        trigger = %next.trigger,
                        cancel = %next.cancel,
                        parsed_trigger = %printed_trigger,
                        parsed_cancel = %printed_cancel,
                        "hotkey bindings changed"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        trigger = ?next.trigger,
                        cancel = ?next.cancel,
                        error = ?e,
                        "invalid hotkey; keeping previous bindings"
                    );
                }
            }
            prev = next;
        }
    });
}
