use anyhow::{anyhow, Result};
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_V,
};

pub(crate) fn paste() -> Result<()> {
    let inputs = ctrl_v_inputs();
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        return Err(send_input_error(sent, inputs.len()));
    }
    Ok(())
}

fn ctrl_v_inputs() -> [INPUT; 4] {
    [
        keyboard_input(VK_CONTROL, 0),
        keyboard_input(VK_V, 0),
        keyboard_input(VK_V, KEYEVENTF_KEYUP),
        keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ]
}

fn keyboard_input(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_input_error(sent: u32, expected: usize) -> anyhow::Error {
    let code = unsafe { GetLastError() };
    anyhow!(
        "SendInput sent {sent}/{expected} keyboard events: {}",
        std::io::Error::from_raw_os_error(code as i32)
    )
}

#[cfg(test)]
mod tests {
    use anyhow::{anyhow, Result};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{GetLastError, HWND};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_CONTROL, VK_V};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, GetWindowTextLengthW, GetWindowTextW,
        PeekMessageW, SetForegroundWindow, ShowWindow, TranslateMessage, CW_USEDEFAULT, MSG,
        PM_REMOVE, SW_SHOW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    #[test]
    fn ctrl_v_inputs_press_and_release_in_order() {
        let inputs = super::ctrl_v_inputs();

        assert_eq!(inputs.len(), 4);
        unsafe {
            assert_eq!(inputs[0].Anonymous.ki.wVk, VK_CONTROL);
            assert_eq!(inputs[0].Anonymous.ki.dwFlags, 0);
            assert_eq!(inputs[1].Anonymous.ki.wVk, VK_V);
            assert_eq!(inputs[1].Anonymous.ki.dwFlags, 0);
            assert_eq!(inputs[2].Anonymous.ki.wVk, VK_V);
            assert_eq!(inputs[2].Anonymous.ki.dwFlags, KEYEVENTF_KEYUP);
            assert_eq!(inputs[3].Anonymous.ki.wVk, VK_CONTROL);
            assert_eq!(inputs[3].Anonymous.ki.dwFlags, KEYEVENTF_KEYUP);
        }
    }

    #[test]
    fn ctrl_v_inputs_are_keyboard_events() {
        let inputs = super::ctrl_v_inputs();

        for input in inputs {
            assert_eq!(
                input.r#type,
                windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_KEYBOARD
            );
        }
    }

    #[test]
    #[ignore = "injects Ctrl+V into the foreground app; run only during Windows runtime smoke"]
    fn paste_runtime_smoke() {
        super::paste().expect("inject Ctrl+V");
    }

    #[test]
    #[ignore = "creates a foreground Win32 edit control and injects Ctrl+V; run only during Windows runtime smoke"]
    fn paste_into_win32_edit_runtime_smoke() -> Result<()> {
        let text = std::env::var("SHUOHUA_WINDOWS_PASTE_TARGET_SMOKE_TEXT")
            .unwrap_or_else(|_| format!("shuohua paste target smoke {}", ulid::Ulid::new()));
        crate::platform::windows::clipboard::write_string(&text)?;

        let window = EditWindow::create()?;
        window.focus()?;
        pump_messages(Duration::from_millis(100));

        super::paste()?;
        pump_messages(Duration::from_millis(300));

        assert_eq!(window.text()?, text);
        Ok(())
    }

    struct EditWindow {
        hwnd: HWND,
    }

    impl EditWindow {
        fn create() -> Result<Self> {
            let class = wide_null("EDIT");
            let title = wide_null("");
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

        fn text(&self) -> Result<String> {
            let len = unsafe { GetWindowTextLengthW(self.hwnd) };
            if len < 0 {
                return Err(last_error("GetWindowTextLengthW"));
            }
            let mut buffer = vec![0u16; len as usize + 1];
            let copied =
                unsafe { GetWindowTextW(self.hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
            if copied == 0 && len > 0 {
                return Err(last_error("GetWindowTextW"));
            }
            buffer.truncate(copied as usize);
            String::from_utf16(&buffer).map_err(Into::into)
        }
    }

    impl Drop for EditWindow {
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
        unsafe { last_error_code(operation, GetLastError()) }
    }

    fn last_error_code(operation: &'static str, code: u32) -> anyhow::Error {
        anyhow!(
            "{operation}: {}",
            std::io::Error::from_raw_os_error(code as i32)
        )
    }
}
