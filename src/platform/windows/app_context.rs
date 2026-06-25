use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::post::AppContext;

const PROCESS_IMAGE_BUFFER_LEN: usize = 32_768;

pub(crate) fn frontmost_app() -> AppContext {
    match frontmost_exe_name() {
        Some(exe_name) => AppContext {
            app_name: Some(app_name_from_exe_name(&exe_name)),
            windows_exe_name: Some(exe_name),
            ..AppContext::default()
        },
        None => AppContext::default(),
    }
}

fn frontmost_exe_name() -> Option<String> {
    let hwnd = unsafe { GetForegroundWindow() };
    let pid = foreground_window_pid(hwnd)?;
    process_image_path(pid).and_then(|path| exe_name_from_path(&path))
}

fn foreground_window_pid(hwnd: HWND) -> Option<u32> {
    if hwnd.is_null() {
        return None;
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    (pid != 0).then_some(pid)
}

fn process_image_path(pid: u32) -> Option<String> {
    let handle = ProcessHandle::open(pid)?;
    let mut buffer = vec![0u16; PROCESS_IMAGE_BUFFER_LEN];
    let mut len = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle.raw(), 0, buffer.as_mut_ptr(), &mut len) };
    if ok == 0 || len == 0 {
        return None;
    }
    buffer.truncate(len as usize);
    Some(String::from_utf16_lossy(&buffer))
}

fn exe_name_from_path(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn app_name_from_exe_name(exe_name: &str) -> String {
    exe_name
        .strip_suffix(".exe")
        .or_else(|| exe_name.strip_suffix(".EXE"))
        .unwrap_or(exe_name)
        .to_string()
}

struct ProcessHandle(HANDLE);

impl ProcessHandle {
    fn open(pid: u32) -> Option<Self> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        (!handle.is_null()).then_some(Self(handle))
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_exe_name_from_windows_path() {
        assert_eq!(
            exe_name_from_path(r"C:\Users\Alice\AppData\Local\Programs\Microsoft VS Code\Code.exe")
                .as_deref(),
            Some("Code.exe")
        );
    }

    #[test]
    fn app_name_drops_exe_suffix_only_for_display() {
        assert_eq!(app_name_from_exe_name("Code.exe"), "Code");
        assert_eq!(app_name_from_exe_name("POWERPNT.EXE"), "POWERPNT");
        assert_eq!(app_name_from_exe_name("wezterm-gui"), "wezterm-gui");
    }
}
