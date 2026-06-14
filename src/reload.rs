//! 配置热重载。M5 设计稿 §2.13 + DESIGN.md:177 / 不变量 #4。
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
//!   - `[asr].provider` 切换（要重建 trait object，跟 ASR session 生命周期耦合）
//!   - UDS `{"op":"reload_config"}` 手动触发（依赖 M4 的 UDS server）
//!   - `shuo doctor` / launchd 自启（M5 同包的另两个 feature，跟 reload 无关）

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio::sync::watch;

use crate::config::{self, Config};
use crate::overlay::{OverlayCmd, OverlayHandle};

pub type Cfg = Arc<Config>;
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
/// 初值 = `config::load_from(path)`。
pub fn watch_with_handle(path: PathBuf) -> Result<(Rx, Handle)> {
    let initial = Arc::new(config::load_from(&path).context("initial config load")?);
    let (tx, rx) = watch::channel(initial);

    let dir = path
        .parent()
        .context("config path has no parent dir")?
        .to_path_buf();
    let file_name = path
        .file_name()
        .context("config path has no file name")?
        .to_os_string();

    let handle = Handle {
        path: path.clone(),
        tx: tx.clone(),
    };

    std::thread::Builder::new()
        .name("config-watcher".into())
        .spawn(move || {
            if let Err(e) = run_watcher(dir, file_name, path, tx) {
                eprintln!("[reload] watcher exited: {e:#}");
            }
        })
        .context("spawn config-watcher thread")?;

    Ok((rx, handle))
}

fn run_watcher(
    dir: PathBuf,
    file_name: OsString,
    path: PathBuf,
    tx: watch::Sender<Cfg>,
) -> Result<()> {
    use notify::{RecursiveMode, Watcher};

    let (event_tx, event_rx) = std_mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = event_tx.send(res);
    })
    .context("create notify watcher")?;
    watcher
        .watch(&dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("watch {}", dir.display()))?;

    let debounce = Duration::from_millis(150);
    loop {
        let event = match event_rx.recv() {
            Ok(Ok(e)) => e,
            Ok(Err(e)) => {
                eprintln!("[reload] notify error: {e}");
                continue;
            }
            Err(_) => return Ok(()),
        };
        if !event
            .paths
            .iter()
            .any(|p| p.file_name() == Some(file_name.as_os_str()))
        {
            continue;
        }
        while event_rx.recv_timeout(debounce).is_ok() {}

        match load_and_broadcast(&path, &tx) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("[reload] parse failed, keeping previous: {e:#}");
            }
        }
    }
}

fn load_and_broadcast(path: &PathBuf, tx: &watch::Sender<Cfg>) -> Result<()> {
    let cfg = config::load_from(path)?;
    tx.send(Arc::new(cfg)).context("broadcast config reload")?;
    crate::debug_println!("[reload] config reloaded from {}", path.display());
    Ok(())
}

/// Overlay subscriber：`[overlay]` 段变化 → `OverlayCmd::ReloadConfig`。
pub fn spawn_overlay(mut rx: Rx, handle: OverlayHandle) {
    tokio::spawn(async move {
        let mut prev = rx.borrow().overlay.clone();
        while rx.changed().await.is_ok() {
            let next = rx.borrow().overlay.clone();
            if next != prev {
                handle.send(OverlayCmd::ReloadConfig { cfg: next.clone() });
                prev = next;
            }
        }
    });
}

/// i18n subscriber：`ui.language` 变化 → 重置字典 + 推 Relabel 让 overlay 刷新当前 state label。
pub fn spawn_i18n(mut rx: Rx, handle: OverlayHandle) {
    tokio::spawn(async move {
        let mut prev = rx.borrow().ui.language.clone();
        while rx.changed().await.is_ok() {
            let next = rx.borrow().ui.language.clone();
            if next != prev {
                crate::i18n::init(&next);
                handle.send(OverlayCmd::Relabel);
                crate::debug_println!("[reload] language → {next}");
                prev = next;
            }
        }
    });
}

/// Hotkey subscriber：`[hotkey].trigger` 变化 → 重新 parse，成功则发新 `Combo` 到 daemon
/// 主循环（主循环用 tokio::select 在 `RawEvent` 和这个 channel 之间多路复用，swap Tracker
/// 与 Suppressor）。parse 失败保留旧 trigger，只打日志。
pub fn spawn_hotkey(mut rx: Rx, combo_tx: mpsc::UnboundedSender<crate::hotkey::Combo>) {
    tokio::spawn(async move {
        let mut prev = rx.borrow().hotkey.trigger.clone();
        while rx.changed().await.is_ok() {
            let next = rx.borrow().hotkey.trigger.clone();
            if next == prev {
                continue;
            }
            match crate::hotkey::parse::parse(&next) {
                Ok(combo) => {
                    let printed = combo.to_string();
                    if combo_tx.send(combo).is_err() {
                        return;
                    }
                    crate::debug_println!("[reload] hotkey trigger → {next} (parsed={printed})");
                }
                Err(e) => {
                    eprintln!("[reload] invalid hotkey {next:?}, keeping previous: {e:#}");
                }
            }
            prev = next;
        }
    });
}
