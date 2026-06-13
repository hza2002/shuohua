//! 把识别文本送达用户。
//!
//! M2: 写剪贴板 + 可选 Cmd+V 上屏。`voice.auto_paste = true`（默认）= 完整
//! 上屏链路；`= false` = 只进剪贴板，用户自己 Cmd+V。
//!
//! M3+ 会加 dispatcher trait 让 history.jsonl 也能算作一种"dispatch"（写
//! 不仅是粘贴）。M2 keep it simple。

use crate::autotype_darwin;
use crate::clipboard_darwin;
use anyhow::{Context, Result};

pub fn dispatch(text: &str, auto_paste: bool) -> Result<()> {
    if text.is_empty() {
        // 没识别出文本就别污染剪贴板。voice 层应在调用前就拦掉，这里多一道防线。
        return Ok(());
    }
    clipboard_darwin::write_string(text).context("write clipboard")?;
    if auto_paste {
        autotype_darwin::paste().context("Cmd+V")?;
    }
    Ok(())
}
