//! shuohua daemon entry.
//!
//!   * tokio multi-thread runtime
//!   * hotkey CGEventTap CFRunLoop 专用 OS 线程 → os_pipe → 桥到 tokio mpsc
//!   * Tracker 纯函数状态机消化 RawEvent → HotkeyEvent；trigger/cancel 可热替换
//!   * trigger 首次命中 = 起录音；再次命中 = 发 stop signal 让 task 收尾
//!   * Session 起来时从 `cfg_rx.borrow()` 取**最新** voice/asr 配置，做到
//!     "下次录音用新值"。
//!   * 配置热重载靠 `reload` 模块（独立）：watcher 在 `~/.config/shuohua/`
//!     上跑 notify；各 subscriber 自取所需。

mod asr;
mod cli;
mod config;
mod daemon;
pub mod history;
mod hotkey;
mod i18n;
mod install;
mod ipc;
mod log;
mod overlay;
pub mod paths;
mod platform;
mod post;
mod reload;
mod state;
mod text_stats;
mod trash;
mod tui;
mod voice;

use anyhow::Result;

fn main() -> Result<()> {
    let args = cli::parse();
    if args.daemon {
        return daemon::run_daemon_process();
    }
    if let Some(command) = args.command {
        return cli::run_command(command);
    }
    daemon::run_smart_fallback()
}
