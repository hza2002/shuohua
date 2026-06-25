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
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_CONTROL, VK_V};

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
}
