use anyhow::{anyhow, Result};
use windows_sys::Win32::Foundation::{GetLastError, GlobalFree, HGLOBAL};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT,
};

const CF_UNICODETEXT_FORMAT: u32 = 13;

pub(crate) fn write_string(text: &str) -> Result<()> {
    let utf16 = clipboard_utf16(text);
    let byte_len = clipboard_utf16_byte_len(&utf16);

    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, byte_len);
        if handle.is_null() {
            return Err(last_error("GlobalAlloc"));
        }

        let mut handle = GlobalMem::new(handle);
        let ptr = GlobalLock(handle.raw()) as *mut u16;
        if ptr.is_null() {
            return Err(last_error("GlobalLock"));
        }

        ptr.copy_from_nonoverlapping(utf16.as_ptr(), utf16.len());
        if GlobalUnlock(handle.raw()) == 0 {
            let error = GetLastError();
            if error != 0 {
                return Err(last_error_code("GlobalUnlock", error));
            }
        }

        let _clipboard = ClipboardGuard::open()?;
        if EmptyClipboard() == 0 {
            return Err(last_error("EmptyClipboard"));
        }
        if SetClipboardData(CF_UNICODETEXT_FORMAT, handle.raw()).is_null() {
            return Err(last_error("SetClipboardData"));
        }
        handle.disown();
    }

    Ok(())
}

fn clipboard_utf16(text: &str) -> Vec<u16> {
    let mut encoded: Vec<u16> = text.encode_utf16().collect();
    encoded.push(0);
    encoded
}

fn clipboard_utf16_byte_len(encoded: &[u16]) -> usize {
    std::mem::size_of_val(encoded)
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

struct GlobalMem {
    handle: HGLOBAL,
}

impl GlobalMem {
    fn new(handle: HGLOBAL) -> Self {
        Self { handle }
    }

    fn raw(&self) -> HGLOBAL {
        self.handle
    }

    fn disown(&mut self) {
        self.handle = std::ptr::null_mut();
    }
}

impl Drop for GlobalMem {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                GlobalFree(self.handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn clipboard_utf16_is_null_terminated() {
        let encoded = super::clipboard_utf16("hello");

        assert_eq!(encoded, vec![104, 101, 108, 108, 111, 0]);
    }

    #[test]
    fn clipboard_utf16_preserves_non_bmp_text() {
        let encoded = super::clipboard_utf16("a🙂");

        assert_eq!(encoded, vec![97, 0xD83D, 0xDE42, 0]);
    }

    #[test]
    fn clipboard_utf16_byte_len_counts_terminal_nul() {
        let encoded = super::clipboard_utf16("ab");

        assert_eq!(super::clipboard_utf16_byte_len(&encoded), 6);
    }

    #[test]
    #[ignore = "writes to the user clipboard; run only during Windows runtime smoke"]
    fn clipboard_write_runtime_smoke() {
        let text = std::env::var("SHUOHUA_WINDOWS_CLIPBOARD_SMOKE_TEXT")
            .unwrap_or_else(|_| "shuohua clipboard smoke".to_string());

        super::write_string(&text).expect("write clipboard text");
    }
}
