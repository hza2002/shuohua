//! WH_KEYBOARD_LL -> pipe bridge + foreground-app suppress.
//!
//! This mirrors the macOS provider boundary: the hook callback writes every
//! decoded event to the `RawEvent` pipe, then asks `Suppressor` whether the
//! foreground app should receive it. Business logic stays on the tokio side.

use anyhow::{anyhow, Result};
use os_pipe::PipeWriter;
use std::io::Write;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::{GetLastError, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_0, VK_9, VK_A, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_ESCAPE, VK_F1,
    VK_F20, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_OEM_1, VK_OEM_2,
    VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD,
    VK_OEM_PLUS, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
    VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use super::combo::{ModMask, ModType, Side};
use super::{EventKind, Key, RawEvent, Suppressor};

static HOOK_STATE: AtomicPtr<HookState> = AtomicPtr::new(null_mut());

struct HookState {
    pipe: Mutex<PipeWriter>,
    suppressor: Arc<Mutex<Suppressor>>,
    mods: Mutex<ModMask>,
}

struct HookHandle(HHOOK);

impl Drop for HookHandle {
    fn drop(&mut self) {
        unsafe {
            UnhookWindowsHookEx(self.0);
        }
    }
}

/// Install a low-level keyboard hook and block until the message loop stops.
pub fn run(writer: PipeWriter, suppressor: Arc<Mutex<Suppressor>>) -> Result<()> {
    let mut state = Box::new(HookState {
        pipe: Mutex::new(writer),
        suppressor,
        mods: Mutex::new(current_mods_snapshot()),
    });
    let state_ptr: *mut HookState = &mut *state;
    if HOOK_STATE
        .compare_exchange(null_mut(), state_ptr, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        anyhow::bail!("Windows hotkey hook is already installed in this process");
    }

    let result = run_with_installed_hook();
    HOOK_STATE.store(null_mut(), Ordering::SeqCst);
    result
}

fn run_with_installed_hook() -> Result<()> {
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    if module.is_null() {
        return Err(last_error("GetModuleHandleW"));
    }

    let hook =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), module, 0) };
    if hook.is_null() {
        return Err(last_error("SetWindowsHookExW(WH_KEYBOARD_LL)"));
    }
    let _hook = HookHandle(hook);

    let mut msg = MSG::default();
    loop {
        let ret = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if ret == -1 {
            return Err(last_error("GetMessageW"));
        }
        if ret == 0 {
            return Ok(());
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let state = HOOK_STATE.load(Ordering::SeqCst);
    if state.is_null() {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let Some(kind) = event_kind(wparam) else {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    };
    let hook = &*(lparam as *const KBDLLHOOKSTRUCT);
    let vk = hook.vkCode as u16;
    let key = key_from_windows_vk(vk);
    let raw = {
        let mut mods = (*state).mods.lock().ok();
        let snapshot = match mods.as_mut() {
            Some(mods) => {
                update_mods_for_key(&mut *mods, key, kind);
                **mods
            }
            None => current_mods_snapshot(),
        };
        RawEvent {
            kind: event_kind_for_key(kind, key),
            key,
            mods: snapshot,
        }
    };

    if let Ok(mut pipe) = (*state).pipe.lock() {
        let _ = pipe.write_all(&raw.encode());
    }

    let (drop_event, log_it) = match (*state).suppressor.lock() {
        Ok(mut s) => (s.on_raw(raw), s.binds_key(key)),
        Err(_) => (false, false),
    };
    if log_it {
        tracing::debug!(
            ?kind,
            ?key,
            mods = raw.mods.0,
            vk,
            drop = drop_event,
            "windows hotkey bound-key event",
        );
    }

    if drop_event {
        1
    } else {
        CallNextHookEx(null_mut(), code, wparam, lparam)
    }
}

fn event_kind(wparam: WPARAM) -> Option<EventKind> {
    match wparam as u32 {
        WM_KEYDOWN | WM_SYSKEYDOWN => Some(EventKind::KeyDown),
        WM_KEYUP | WM_SYSKEYUP => Some(EventKind::KeyUp),
        _ => None,
    }
}

fn event_kind_for_key(kind: EventKind, key: Key) -> EventKind {
    if key.modifier().is_some() {
        EventKind::FlagsChanged
    } else {
        kind
    }
}

fn update_mods_for_key(mask: &mut ModMask, key: Key, kind: EventKind) {
    let Some((ty, side)) = key.modifier() else {
        return;
    };
    mask.set(ty, side, matches!(kind, EventKind::KeyDown));
}

fn current_mods_snapshot() -> ModMask {
    let mut mask = ModMask::empty();
    for (vk, ty, side) in [
        (VK_LWIN, ModType::Cmd, Side::Left),
        (VK_RWIN, ModType::Cmd, Side::Right),
        (VK_LCONTROL, ModType::Ctrl, Side::Left),
        (VK_RCONTROL, ModType::Ctrl, Side::Right),
        (VK_LMENU, ModType::Opt, Side::Left),
        (VK_RMENU, ModType::Opt, Side::Right),
        (VK_LSHIFT, ModType::Shift, Side::Left),
        (VK_RSHIFT, ModType::Shift, Side::Right),
    ] {
        if unsafe { GetKeyState(vk as i32) } < 0 {
            mask.set(ty, side, true);
        }
    }
    mask
}

fn key_from_windows_vk(vk: u16) -> Key {
    if (VK_A..=VK_A + 25).contains(&vk) {
        return Key::Char(char::from_u32((b'a' + (vk - VK_A) as u8) as u32).unwrap());
    }
    match vk {
        VK_F1..=VK_F20 => Key::F((vk - VK_F1 + 1) as u8),
        VK_0..=VK_9 => Key::Digit((vk - VK_0) as u8),
        VK_SPACE => Key::Space,
        VK_TAB => Key::Tab,
        VK_RETURN => Key::Return,
        VK_ESCAPE => Key::Escape,
        VK_BACK => Key::Backspace,
        VK_DELETE => Key::Delete,
        VK_UP => Key::ArrowUp,
        VK_DOWN => Key::ArrowDown,
        VK_LEFT => Key::ArrowLeft,
        VK_RIGHT => Key::ArrowRight,
        VK_OEM_1 => Key::Punct(';'),
        VK_OEM_COMMA => Key::Punct(','),
        VK_OEM_PERIOD => Key::Punct('.'),
        VK_OEM_2 => Key::Punct('/'),
        VK_OEM_5 => Key::Punct('\\'),
        VK_OEM_4 => Key::Punct('['),
        VK_OEM_6 => Key::Punct(']'),
        VK_OEM_7 => Key::Punct('\''),
        VK_OEM_3 => Key::Punct('`'),
        VK_OEM_MINUS => Key::Punct('-'),
        VK_OEM_PLUS => Key::Punct('='),
        VK_LWIN => Key::Modifier(ModType::Cmd, Side::Left),
        VK_RWIN => Key::Modifier(ModType::Cmd, Side::Right),
        VK_LCONTROL => Key::Modifier(ModType::Ctrl, Side::Left),
        VK_RCONTROL => Key::Modifier(ModType::Ctrl, Side::Right),
        VK_LMENU => Key::Modifier(ModType::Opt, Side::Left),
        VK_RMENU => Key::Modifier(ModType::Opt, Side::Right),
        VK_LSHIFT => Key::Modifier(ModType::Shift, Side::Left),
        VK_RSHIFT => Key::Modifier(ModType::Shift, Side::Right),
        VK_CONTROL => Key::Modifier(ModType::Ctrl, Side::Left),
        VK_MENU => Key::Modifier(ModType::Opt, Side::Left),
        VK_SHIFT => Key::Modifier(ModType::Shift, Side::Left),
        other => Key::Unknown(other),
    }
}

fn last_error(context: &'static str) -> anyhow::Error {
    let code = unsafe { GetLastError() };
    anyhow!(
        "{context}: {}",
        std::io::Error::from_raw_os_error(code as i32)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::combo::{ModType, Side};

    #[test]
    fn maps_letters_digits_function_keys_and_punctuation() {
        assert_eq!(key_from_windows_vk(VK_A), Key::Char('a'));
        assert_eq!(key_from_windows_vk(VK_A + 25), Key::Char('z'));
        assert_eq!(key_from_windows_vk(VK_0), Key::Digit(0));
        assert_eq!(key_from_windows_vk(VK_9), Key::Digit(9));
        assert_eq!(key_from_windows_vk(VK_F1), Key::F(1));
        assert_eq!(key_from_windows_vk(VK_F20), Key::F(20));
        assert_eq!(key_from_windows_vk(VK_OEM_4), Key::Punct('['));
        assert_eq!(key_from_windows_vk(VK_OEM_PLUS), Key::Punct('='));
    }

    #[test]
    fn maps_left_and_right_modifiers() {
        assert_eq!(
            key_from_windows_vk(VK_LCONTROL).modifier(),
            Some((ModType::Ctrl, Side::Left))
        );
        assert_eq!(
            key_from_windows_vk(VK_RMENU).modifier(),
            Some((ModType::Opt, Side::Right))
        );
        assert_eq!(
            key_from_windows_vk(VK_LWIN).modifier(),
            Some((ModType::Cmd, Side::Left))
        );
        assert_eq!(
            key_from_windows_vk(VK_RSHIFT).modifier(),
            Some((ModType::Shift, Side::Right))
        );
    }

    #[test]
    fn modifier_events_are_flags_changed_with_post_transition_snapshot() {
        let mut mods = ModMask::empty();
        let key = key_from_windows_vk(VK_RSHIFT);
        update_mods_for_key(&mut mods, key, EventKind::KeyDown);
        assert!(mods.is_side_down(ModType::Shift, Side::Right));
        let raw = RawEvent {
            kind: event_kind_for_key(EventKind::KeyDown, key),
            key,
            mods,
        };
        assert_eq!(raw.kind, EventKind::FlagsChanged);

        update_mods_for_key(&mut mods, key, EventKind::KeyUp);
        assert!(!mods.is_any_side_down(ModType::Shift));
    }

    #[test]
    fn key_events_keep_keydown_keyup_kind() {
        let key = key_from_windows_vk(VK_F1 + 15);
        assert_eq!(
            event_kind_for_key(EventKind::KeyDown, key),
            EventKind::KeyDown
        );
        assert_eq!(event_kind_for_key(EventKind::KeyUp, key), EventKind::KeyUp);
    }

    #[test]
    #[ignore = "installs a global keyboard hook and sends a synthetic F16 key press"]
    fn hook_runtime_smoke_receives_synthetic_f16_down_up() {
        use crate::hotkey::{Combo, ModMask, Suppressor};
        use std::io::Read;
        use std::time::{Duration, Instant};
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        };

        let (mut reader, writer) = os_pipe::pipe().expect("pipe");
        let suppressor = Arc::new(Mutex::new(Suppressor::new(Combo {
            mods: [crate::hotkey::combo::ModMatcher::NotPresent; 4],
            key: Some(Key::F(16)),
            double: false,
        })));
        std::thread::spawn(move || run(writer, suppressor).expect("windows hook"));
        std::thread::sleep(Duration::from_millis(250));

        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_F1 + 15,
                        wScan: 0,
                        dwFlags: 0,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_F1 + 15,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
        ];
        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        assert_eq!(sent, inputs.len() as u32, "SendInput F16 down/up");

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut saw_down = false;
        let mut saw_up = false;
        while Instant::now() < deadline && !(saw_down && saw_up) {
            let mut buf = [0u8; 4];
            reader.read_exact(&mut buf).expect("read hotkey event");
            if let Some(raw) = RawEvent::decode(buf) {
                if raw.key == Key::F(16) && raw.mods == ModMask::empty() {
                    saw_down |= raw.kind == EventKind::KeyDown;
                    saw_up |= raw.kind == EventKind::KeyUp;
                }
            }
        }
        assert!(saw_down && saw_up, "expected F16 down/up within 10 seconds");
    }
}
