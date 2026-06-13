//! Hotkey: CGEventTap → pipe → Tracker → HotkeyEvent.
//!
//! At M1 the trigger keycode is supplied as a `u16` to `Tracker::new` and
//! lives as a constant in `main.rs`. At M2 the value will be parsed from
//! `config.toml` (`[hotkey] trigger = "F16"`) via a string→keycode table,
//! and `Combo` (keycode + modifier mask) will replace the bare `u16` here.
//!
//! macOS virtual keycodes are physical-position-based (same across all
//! keyboard layouts) and defined in HIToolbox/Events.h: F16 = 0x6A, etc.

pub mod provider_darwin;
pub mod tracker;

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
