//! 模拟 Cmd+V 上屏。
//!
//! 通过 CGEventCreateKeyboardEvent 合成两个键盘事件（V down + V up），各带
//! kCGEventFlagMaskCommand 修饰，CGEventPost 到 kCGHIDEventTap。
//!
//! V 的虚拟键码 = 0x09（HIToolbox/Events.h，物理位置稳定，跟键盘布局无关）。
//!
//! 需要 Accessibility 权限（跟 hotkey CGEventTap 是同一份）。撤回权限时
//! 事件会被悄悄丢弃；这层无法区分"丢弃"和"成功"，只在 voice 层加 toast
//! 提示。

use anyhow::{anyhow, Result};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

const KEY_V: u16 = 0x09;

pub fn paste() -> Result<()> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| anyhow!("CGEventSource::new failed"))?;

    let down = CGEvent::new_keyboard_event(source.clone(), KEY_V, true)
        .map_err(|_| anyhow!("create V keydown event"))?;
    down.set_flags(CGEventFlags::CGEventFlagCommand);
    down.post(CGEventTapLocation::HID);

    let up = CGEvent::new_keyboard_event(source, KEY_V, false)
        .map_err(|_| anyhow!("create V keyup event"))?;
    up.set_flags(CGEventFlags::CGEventFlagCommand);
    up.post(CGEventTapLocation::HID);

    Ok(())
}
