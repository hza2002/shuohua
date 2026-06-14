//! Hotkey subsystem: CGEventTap → pipe → Tracker → `HotkeyEvent`.
//!
//! Module layout:
//!
//! - [`combo`] — `Combo` / `ModMatcher` / `ModMask` / `Side` / `ModType`.
//!   Static description of a configured trigger.
//! - [`parse`] — grammar `"left_cmd+shift+r:double"` → `Combo`.
//! - [`tracker`] — pure state machine: `RawEvent + Instant` → `HotkeyEvent`.
//!   Handles auto-repeat debounce, combo exact match, modifier-only tap
//!   detection, and double-tap windowing.
//! - [`suppressor`] — pure state machine: `RawEvent` → `bool` ("drop?").
//!   Per-trigger-type behavior: pure-key fully suppressed, combos suppress
//!   only the key portion, modifier-only triggers never suppress.
//! - [`provider_darwin`] — CGEventTap on a dedicated CFRunLoop thread,
//!   encoding every observed event to the 4-byte wire format.
//!
//! Virtual keycodes are macOS HIToolbox values, stable across keyboard
//! layouts (physical-position based).

pub mod combo;
pub mod parse;
pub mod provider_darwin;
pub mod suppressor;
pub mod tracker;

#[cfg(test)]
mod proptests;

pub use combo::{Combo, ModMask};
pub use suppressor::Suppressor;
pub use tracker::Tracker;

/// Event kind tag on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EventKind {
    KeyDown = 0,
    KeyUp = 1,
    /// Modifier-key transition. `RawEvent::code` is the modifier's keycode
    /// (e.g. 0x37 for left Cmd), `mods` is the post-transition snapshot.
    FlagsChanged = 2,
}

/// A keyboard event decoded from the CGEventTap pipe.
///
/// Wire format: 4 bytes `[kind, code_lo, code_hi, mods]`. Time is *not* on
/// the wire — the Tracker stamps `Instant::now()` on receipt, which adds
/// sub-millisecond noise (well below the 250ms tap / 400ms double-tap
/// windows). Avoids serializing a monotonic clock value and keeps the wire
/// stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawEvent {
    pub kind: EventKind,
    pub code: u16,
    pub mods: ModMask,
}

impl RawEvent {
    pub fn encode(self) -> [u8; 4] {
        [
            self.kind as u8,
            (self.code & 0xff) as u8,
            (self.code >> 8) as u8,
            self.mods.0,
        ]
    }

    pub fn decode(buf: [u8; 4]) -> Option<Self> {
        let kind = match buf[0] {
            0 => EventKind::KeyDown,
            1 => EventKind::KeyUp,
            2 => EventKind::FlagsChanged,
            _ => return None,
        };
        Some(Self {
            kind,
            code: u16::from_le_bytes([buf[1], buf[2]]),
            mods: ModMask(buf[3]),
        })
    }
}

/// Semantic hotkey event emitted by `Tracker` after debouncing auto-repeat,
/// matching combos exactly, and applying tap / double-tap windowing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The configured trigger fired. Caller toggles recording state.
    TriggerRecord,
}

#[cfg(test)]
mod wire_tests {
    use super::*;

    #[test]
    fn round_trip_all_kinds() {
        for kind in [
            EventKind::KeyDown,
            EventKind::KeyUp,
            EventKind::FlagsChanged,
        ] {
            for &(code, mods) in &[(0u16, 0u8), (0x6A, 0b1010_1010), (0xFFFF, 0xFF)] {
                let ev = RawEvent {
                    kind,
                    code,
                    mods: ModMask(mods),
                };
                assert_eq!(RawEvent::decode(ev.encode()), Some(ev));
            }
        }
    }

    #[test]
    fn unknown_kind_byte_rejected() {
        assert!(RawEvent::decode([3, 0, 0, 0]).is_none());
        assert!(RawEvent::decode([255, 0, 0, 0]).is_none());
    }
}
