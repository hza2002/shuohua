//! 把识别文本送达用户。
//!
//! 两步链路：先写剪贴板（必成功才算 dispatch 成功）→ 再可选 Cmd+V 上屏。
//! Cmd+V 失败不算 dispatch 失败：文本已进剪贴板，用户手动 Cmd+V 即可恢复。
//! 这样 Accessibility 权限被撤、目标 App 拒绝注入等罕见路径上，用户体验
//! 不至于"看着像啥都没干"。
//!
//! 各步骤的日志由本模块自己负责，调用方只看 Result 决定是否记 history.

use crate::autotype_darwin;
use crate::clipboard_darwin;
use anyhow::{Context, Result};

pub fn dispatch(text: &str, auto_paste: bool) -> Result<()> {
    if text.is_empty() {
        // 没识别出文本就别污染剪贴板。voice 层应在调用前就拦掉，这里多一道防线。
        return Ok(());
    }
    clipboard_darwin::write_string(text).context("write clipboard")?;
    eprintln!("[shuo] ✓ 剪贴板已写入");

    if auto_paste {
        match autotype_darwin::paste() {
            Ok(()) => eprintln!("[shuo] ✓ Cmd+V 已上屏"),
            Err(e) => eprintln!("[shuo] ⚠ Cmd+V 失败，文本仍在剪贴板，可手动粘贴: {e:#}"),
        }
    }
    Ok(())
}
