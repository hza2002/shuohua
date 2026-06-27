//! 把识别文本送达用户。
//!
//! 两步链路：先写剪贴板（必成功才算 dispatch 成功）→ 再可选 Cmd+V 上屏。
//! Cmd+V 失败不算 dispatch 失败：文本已进剪贴板，用户手动 Cmd+V 即可恢复。
//! 这样 Accessibility 权限被撤、目标 App 拒绝注入等罕见路径上，用户体验
//! 不至于"看着像啥都没干"。
//!
//! 各步骤的日志由本模块自己负责，调用方只看 Result 决定是否记 history.

use anyhow::{Context, Result};

pub fn dispatch(text: &str, auto_paste: bool) -> Result<()> {
    if text.is_empty() {
        // 没识别出文本就别污染剪贴板。voice 层应在调用前就拦掉，这里多一道防线。
        return Ok(());
    }
    crate::platform::desktop::write_clipboard_string(text).context("write clipboard")?;
    tracing::debug!("clipboard write succeeded");

    if auto_paste {
        match crate::platform::desktop::paste_text() {
            Ok(()) => tracing::debug!("auto paste succeeded"),
            Err(e) => tracing::warn!(error = ?e, "auto paste failed; text remains on clipboard"),
        }
    }
    Ok(())
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use anyhow::{anyhow, Result};
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    const CF_UNICODETEXT_FORMAT: u32 = 13;

    #[test]
    fn empty_dispatch_does_not_touch_clipboard_path() {
        super::dispatch("", true).unwrap();
    }

    #[test]
    #[ignore = "writes to the user clipboard; run only during Windows dispatch runtime smoke"]
    fn windows_dispatch_clipboard_runtime_smoke() {
        let text = std::env::var("SHUOHUA_WINDOWS_DISPATCH_SMOKE_TEXT")
            .unwrap_or_else(|_| format!("shuohua dispatch smoke {}", ulid::Ulid::new()));

        super::dispatch(&text, false).expect("dispatch text to clipboard");

        let actual = read_clipboard_string().expect("read clipboard text");
        assert_eq!(actual, text);
    }

    fn read_clipboard_string() -> Result<String> {
        unsafe {
            let _clipboard = ClipboardGuard::open()?;
            if IsClipboardFormatAvailable(CF_UNICODETEXT_FORMAT) == 0 {
                return Err(anyhow!("CF_UNICODETEXT is not available"));
            }
            let handle = GetClipboardData(CF_UNICODETEXT_FORMAT);
            if handle.is_null() {
                return Err(last_error("GetClipboardData"));
            }
            let ptr = GlobalLock(handle) as *const u16;
            if ptr.is_null() {
                return Err(last_error("GlobalLock"));
            }
            let result = read_null_terminated_utf16(ptr);
            if GlobalUnlock(handle) == 0 {
                let error = GetLastError();
                if error != 0 {
                    return Err(last_error_code("GlobalUnlock", error));
                }
            }
            result
        }
    }

    unsafe fn read_null_terminated_utf16(ptr: *const u16) -> Result<String> {
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        String::from_utf16(slice).map_err(Into::into)
    }

    fn last_error(operation: &'static str) -> anyhow::Error {
        unsafe { last_error_code(operation, GetLastError()) }
    }

    fn last_error_code(operation: &'static str, code: u32) -> anyhow::Error {
        anyhow!(
            "{operation}: {}",
            std::io::Error::from_raw_os_error(code as i32)
        )
    }

    struct ClipboardGuard;

    impl ClipboardGuard {
        unsafe fn open() -> Result<Self> {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return Err(last_error("OpenClipboard"));
            }
            Ok(Self)
        }
    }

    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }
}
