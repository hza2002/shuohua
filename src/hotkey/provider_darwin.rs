//! CGEventTap → pipe bridge + foreground-app suppress.
//!
//! docs/modules/hotkey.md（依赖红线 + 不变量 8）: the tap runs in `Default` mode so
//! the callback can `CallbackResult::Drop` events (returns NULL to the OS).
//! Every event (KeyDown / KeyUp / FlagsChanged) is encoded into the 4-byte
//! `RawEvent` wire format and pushed to the pipe so the tokio-side
//! `Tracker` keeps seeing everything, even events we drop. Suppression
//! decisions live in [`Suppressor`].

use anyhow::{anyhow, Result};
use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};
use os_pipe::PipeWriter;
use std::io::Write;
use std::sync::{Arc, Mutex};

use super::combo::{ModMask, ModType, Side};
use super::key::key_from_macos_keycode;
use super::{EventKind, RawEvent, Suppressor};

// Device-specific modifier bits inside `CGEventFlags::bits()` (low 16 bits).
// See IOLLEvent.h `NX_DEVICE*` constants. Right Control is the outlier at
// 0x2000 instead of being adjacent to Left Control — Apple convention.
const NX_LCTL: u64 = 0x0000_0001;
const NX_LSHIFT: u64 = 0x0000_0002;
const NX_RSHIFT: u64 = 0x0000_0004;
const NX_LCMD: u64 = 0x0000_0008;
const NX_RCMD: u64 = 0x0000_0010;
const NX_LOPT: u64 = 0x0000_0020;
const NX_ROPT: u64 = 0x0000_0040;
const NX_RCTL: u64 = 0x0000_2000;

/// Install a CGEventTap and block until the runloop stops.
pub fn run(writer: PipeWriter, suppressor: Arc<Mutex<Suppressor>>) -> Result<()> {
    let pipe = Mutex::new(writer);

    CGEventTap::with_enabled(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        // Default = active filter; ListenOnly would ignore the return
        // value. We need the active path to suppress.
        CGEventTapOptions::Default,
        // FlagsChanged lets the tokio-side Tracker detect modifier-only
        // triggers and maintain a current `ModMask` snapshot for combo matching.
        vec![
            CGEventType::KeyDown,
            CGEventType::KeyUp,
            CGEventType::FlagsChanged,
        ],
        move |_proxy, etype, event| {
            let kind = match etype {
                CGEventType::KeyDown => EventKind::KeyDown,
                CGEventType::KeyUp => EventKind::KeyUp,
                CGEventType::FlagsChanged => EventKind::FlagsChanged,
                _ => return CallbackResult::Keep,
            };
            let key = key_from_macos_keycode(
                event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16,
            );
            let mods = decode_mods(event.get_flags().bits());
            let raw = RawEvent { kind, key, mods };

            // Always forward — Tracker (tokio side) needs every event even
            // for keys we end up dropping for the foreground.
            if let Ok(mut w) = pipe.lock() {
                let _ = w.write_all(&raw.encode());
            }

            let drop_event = match suppressor.lock() {
                Ok(mut s) => s.on_raw(raw),
                // Poisoned mutex: a Suppressor user panicked. Let events
                // through rather than silently eating them.
                Err(_) => false,
            };

            if drop_event {
                CallbackResult::Drop
            } else {
                CallbackResult::Keep
            }
        },
        || {
            CFRunLoop::run_current();
        },
    )
    .map_err(|_| {
        anyhow!(
            "CGEventTapCreate failed. Default-mode taps require Accessibility \
             permission — grant it to the terminal running `shuo` in System \
             Settings → Privacy & Security → Accessibility."
        )
    })?;

    Ok(())
}

/// Decode CGEvent's `flags` field into our `ModMask`. Reads the device-
/// specific (left/right) bits; the device-independent high-half bits
/// (`CGEventFlagCommand` etc.) are ignored because they don't distinguish
/// sides. Synthetic events that set only the high half won't be
/// recognized — acceptable since shuohua only consumes hardware events.
fn decode_mods(flags: u64) -> ModMask {
    let mut m = ModMask::empty();
    if flags & NX_LCTL != 0 {
        m.set(ModType::Ctrl, Side::Left, true);
    }
    if flags & NX_RCTL != 0 {
        m.set(ModType::Ctrl, Side::Right, true);
    }
    if flags & NX_LSHIFT != 0 {
        m.set(ModType::Shift, Side::Left, true);
    }
    if flags & NX_RSHIFT != 0 {
        m.set(ModType::Shift, Side::Right, true);
    }
    if flags & NX_LCMD != 0 {
        m.set(ModType::Cmd, Side::Left, true);
    }
    if flags & NX_RCMD != 0 {
        m.set(ModType::Cmd, Side::Right, true);
    }
    if flags & NX_LOPT != 0 {
        m.set(ModType::Opt, Side::Left, true);
    }
    if flags & NX_ROPT != 0 {
        m.set(ModType::Opt, Side::Right, true);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_empty() {
        assert_eq!(decode_mods(0), ModMask::empty());
    }

    #[test]
    fn decode_individual_sides() {
        for (flag, ty, side) in [
            (NX_LCTL, ModType::Ctrl, Side::Left),
            (NX_RCTL, ModType::Ctrl, Side::Right),
            (NX_LSHIFT, ModType::Shift, Side::Left),
            (NX_RSHIFT, ModType::Shift, Side::Right),
            (NX_LCMD, ModType::Cmd, Side::Left),
            (NX_RCMD, ModType::Cmd, Side::Right),
            (NX_LOPT, ModType::Opt, Side::Left),
            (NX_ROPT, ModType::Opt, Side::Right),
        ] {
            let m = decode_mods(flag);
            assert!(m.is_side_down(ty, side), "expected {ty:?}/{side:?} set");
            // No other bits.
            for ty2 in ModType::ALL {
                for side2 in [Side::Left, Side::Right] {
                    if (ty2, side2) != (ty, side) {
                        assert!(!m.is_side_down(ty2, side2));
                    }
                }
            }
        }
    }

    #[test]
    fn decode_multiple_sides_combined() {
        // Left Cmd + Right Shift held simultaneously.
        let m = decode_mods(NX_LCMD | NX_RSHIFT);
        assert!(m.is_side_down(ModType::Cmd, Side::Left));
        assert!(m.is_side_down(ModType::Shift, Side::Right));
        assert!(!m.is_side_down(ModType::Cmd, Side::Right));
    }

    #[test]
    fn decode_ignores_device_independent_high_bits() {
        // Synthetic event that only sets `CGEventFlagCommand` (0x00100000)
        // without a side bit → we ignore it.
        let m = decode_mods(0x0010_0000);
        assert_eq!(m, ModMask::empty());
    }

    #[test]
    fn keycode_mapping_keeps_modifier_sides() {
        assert_eq!(
            key_from_macos_keycode(0x37).modifier(),
            Some((ModType::Cmd, Side::Left))
        );
        assert_eq!(
            key_from_macos_keycode(0x36).modifier(),
            Some((ModType::Cmd, Side::Right))
        );
        assert_eq!(
            key_from_macos_keycode(0x38).modifier(),
            Some((ModType::Shift, Side::Left))
        );
        assert_eq!(
            key_from_macos_keycode(0x3C).modifier(),
            Some((ModType::Shift, Side::Right))
        );
        assert_eq!(
            key_from_macos_keycode(0x3A).modifier(),
            Some((ModType::Opt, Side::Left))
        );
        assert_eq!(
            key_from_macos_keycode(0x3D).modifier(),
            Some((ModType::Opt, Side::Right))
        );
        assert_eq!(
            key_from_macos_keycode(0x3B).modifier(),
            Some((ModType::Ctrl, Side::Left))
        );
        assert_eq!(
            key_from_macos_keycode(0x3E).modifier(),
            Some((ModType::Ctrl, Side::Right))
        );
        assert_eq!(key_from_macos_keycode(0x6A), crate::hotkey::Key::F(16));
        assert_eq!(key_from_macos_keycode(0x00), crate::hotkey::Key::Char('a'));
    }
}
