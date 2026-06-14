//! Hotkey: CGEventTap → pipe → Tracker → HotkeyEvent.
//!
//! M2.b: trigger keycode parsed from `[hotkey] trigger = "f16"` via `parse`.
//! M6 will introduce `Combo` (keycode + modifier mask) + `registry` for
//! multi-binding + real suppress; M2 only ships single-key F1–F20.
//!
//! macOS virtual keycodes are physical-position-based (same across all
//! keyboard layouts) and defined in HIToolbox/Events.h: F16 = 0x6A, etc.

pub mod parse;
pub mod provider_darwin;
pub mod suppressor;
pub mod tracker;

#[cfg(test)]
mod proptests;

pub use suppressor::Suppressor;
pub use tracker::Tracker;

/// A keyboard event after CGEventTap → pipe decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawKey {
    pub down: bool,
    pub code: u16,
}

impl RawKey {
    /// Wire format: [down:u8] [code_lo:u8] [code_hi:u8] [pad:u8].
    pub fn encode(down: bool, code: u16) -> [u8; 4] {
        [down as u8, (code & 0xff) as u8, (code >> 8) as u8, 0]
    }

    pub fn decode(buf: [u8; 4]) -> Self {
        Self {
            down: buf[0] != 0,
            code: u16::from_le_bytes([buf[1], buf[2]]),
        }
    }
}

/// Semantic hotkey event emitted by Tracker after debouncing auto-repeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The configured trigger key was pressed. Caller should start recording.
    TriggerRecord,
}
