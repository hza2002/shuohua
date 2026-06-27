use std::path::Path;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HANDLE, HWND,
};
use windows_sys::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::post::AppContext;

const PROCESS_IMAGE_BUFFER_LEN: usize = 32_768;

pub(crate) fn frontmost_app() -> AppContext {
    let Some(identity) = frontmost_identity() else {
        return AppContext::default();
    };
    AppContext {
        app_name: identity.exe_name.as_deref().map(app_name_from_exe_name),
        windows_app_user_model_id: identity.app_user_model_id,
        windows_exe_name: identity.exe_name,
        ..AppContext::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowsAppIdentity {
    app_user_model_id: Option<String>,
    exe_name: Option<String>,
}

fn frontmost_identity() -> Option<WindowsAppIdentity> {
    let hwnd = unsafe { GetForegroundWindow() };
    let pid = foreground_window_pid(hwnd)?;
    let handle = ProcessHandle::open(pid)?;
    Some(identity_from_process(&handle))
}

fn identity_from_process(handle: &ProcessHandle) -> WindowsAppIdentity {
    WindowsAppIdentity {
        app_user_model_id: process_app_user_model_id(handle),
        exe_name: process_image_path(handle).and_then(|path| exe_name_from_path(&path)),
    }
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

fn process_image_path(handle: &ProcessHandle) -> Option<String> {
    let mut buffer = vec![0u16; PROCESS_IMAGE_BUFFER_LEN];
    let mut len = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle.raw(), 0, buffer.as_mut_ptr(), &mut len) };
    if ok == 0 || len == 0 {
        return None;
    }
    buffer.truncate(len as usize);
    Some(String::from_utf16_lossy(&buffer))
}

fn process_app_user_model_id(handle: &ProcessHandle) -> Option<String> {
    let mut len = 0u32;
    let first = unsafe { GetApplicationUserModelId(handle.raw(), &mut len, std::ptr::null_mut()) };
    if first != ERROR_INSUFFICIENT_BUFFER || len == 0 {
        return None;
    }

    let mut buffer = vec![0u16; len as usize];
    let second = unsafe { GetApplicationUserModelId(handle.raw(), &mut len, buffer.as_mut_ptr()) };
    if second != ERROR_SUCCESS || len == 0 {
        return None;
    }
    utf16_nul_terminated_to_string(buffer.get(..len as usize).unwrap_or(&buffer))
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

fn utf16_nul_terminated_to_string(buffer: &[u16]) -> Option<String> {
    let without_nul = buffer
        .split_last()
        .map(|(last, rest)| if *last == 0 { rest } else { buffer })
        .unwrap_or(&[]);
    (!without_nul.is_empty()).then(|| String::from_utf16_lossy(without_nul))
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
    use anyhow::{anyhow, Result};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, PeekMessageW, SetForegroundWindow,
        ShowWindow, TranslateMessage, CW_USEDEFAULT, MSG, PM_REMOVE, SW_SHOW, WS_OVERLAPPEDWINDOW,
        WS_VISIBLE,
    };

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

    #[test]
    fn app_user_model_id_utf16_trims_terminal_nul() {
        let encoded = [
            'M' as u16, 'i' as u16, 'c' as u16, 'r' as u16, 'o' as u16, 's' as u16, 'o' as u16,
            'f' as u16, 't' as u16, '.' as u16, 'A' as u16, 'p' as u16, 'p' as u16, 0,
        ];

        assert_eq!(
            utf16_nul_terminated_to_string(&encoded).as_deref(),
            Some("Microsoft.App")
        );
    }

    #[test]
    #[ignore = "creates a foreground Win32 window; run only during Windows active-app runtime smoke"]
    fn foreground_self_window_runtime_smoke() -> Result<()> {
        let expected_exe = std::env::current_exe()?
            .file_name()
            .and_then(|name| name.to_str())
            .map(ToOwned::to_owned)
            .ok_or_else(|| anyhow!("current test executable has no UTF-8 file name"))?;

        let window = TestWindow::create()?;
        window.focus()?;
        pump_messages(Duration::from_millis(150));

        let app = super::frontmost_app();
        assert_eq!(app.windows_exe_name.as_deref(), Some(expected_exe.as_str()));
        assert_eq!(
            app.app_name.as_deref(),
            Some(app_name_from_exe_name(&expected_exe).as_str())
        );

        let routes = crate::config::ProfileRouteCfg {
            default: "default".to_string(),
            routes: crate::config::ProfileRoutes::from_iter([(
                "foreground".to_string(),
                crate::config::ProfileRouteMatchers {
                    windows: crate::config::WindowsProfileMatchers {
                        exe_name: vec![expected_exe.to_ascii_uppercase()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )]),
        };
        assert_eq!(
            routes.matching_profiles(&crate::config::AppIdentity::current_from_app_context(&app)),
            vec!["foreground"],
            "foreground Windows exe identity should drive profile route matching"
        );
        Ok(())
    }

    struct TestWindow {
        hwnd: HWND,
    }

    impl TestWindow {
        fn create() -> Result<Self> {
            let class = wide_null("STATIC");
            let title = wide_null("shuohua active app smoke");
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    title.as_ptr(),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    480,
                    180,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if hwnd.is_null() {
                return Err(last_error("CreateWindowExW"));
            }
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
            }
            Ok(Self { hwnd })
        }

        fn focus(&self) -> Result<()> {
            unsafe {
                if SetForegroundWindow(self.hwnd) == 0 {
                    return Err(last_error("SetForegroundWindow"));
                }
            }
            Ok(())
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.hwnd);
            }
        }
    }

    fn pump_messages(duration: Duration) {
        let until = Instant::now() + duration;
        while Instant::now() < until {
            unsafe {
                let mut msg = std::mem::zeroed::<MSG>();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn last_error(operation: &'static str) -> anyhow::Error {
        unsafe {
            anyhow!(
                "{operation}: {}",
                std::io::Error::from_raw_os_error(GetLastError() as i32)
            )
        }
    }
}
