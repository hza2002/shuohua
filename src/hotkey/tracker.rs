//! Pure state machine: RawKey → HotkeyEvent. No I/O, no time, no syscalls.
//!
//! M1 tracker handles a single trigger keycode without modifiers. M2 will
//! generalize this to `Combo { keycode, mods }` and a registry of combos.

use super::{HotkeyEvent, RawKey};

/// Debounces auto-repeat keydowns of the configured trigger key.
#[derive(Debug)]
pub struct Tracker {
    trigger_code: u16,
    trigger_pressed: bool,
}

impl Tracker {
    pub fn new(trigger_code: u16) -> Self {
        Self {
            trigger_code,
            trigger_pressed: false,
        }
    }

    pub fn on_raw(&mut self, raw: RawKey) -> Option<HotkeyEvent> {
        if raw.code != self.trigger_code {
            return None;
        }
        match raw.down {
            true if !self.trigger_pressed => {
                self.trigger_pressed = true;
                Some(HotkeyEvent::TriggerRecord)
            }
            true => None, // auto-repeat
            false => {
                self.trigger_pressed = false;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F16: u16 = 0x6A;
    const A: u16 = 0x00;

    fn down(code: u16) -> RawKey {
        RawKey { down: true, code }
    }
    fn up(code: u16) -> RawKey {
        RawKey { down: false, code }
    }

    #[test]
    fn trigger_keydown_emits_once() {
        let mut t = Tracker::new(F16);
        assert_eq!(t.on_raw(down(F16)), Some(HotkeyEvent::TriggerRecord));
    }

    #[test]
    fn auto_repeat_keydown_does_not_retrigger() {
        let mut t = Tracker::new(F16);
        t.on_raw(down(F16));
        assert_eq!(t.on_raw(down(F16)), None);
        assert_eq!(t.on_raw(down(F16)), None);
    }

    #[test]
    fn keyup_releases_so_next_keydown_triggers() {
        let mut t = Tracker::new(F16);
        assert_eq!(t.on_raw(down(F16)), Some(HotkeyEvent::TriggerRecord));
        t.on_raw(up(F16));
        assert_eq!(t.on_raw(down(F16)), Some(HotkeyEvent::TriggerRecord));
    }

    #[test]
    fn non_trigger_keys_are_ignored_and_do_not_affect_state() {
        let mut t = Tracker::new(F16);
        assert_eq!(t.on_raw(down(A)), None);
        assert_eq!(t.on_raw(up(A)), None);
        assert_eq!(t.on_raw(down(F16)), Some(HotkeyEvent::TriggerRecord));
    }

    #[test]
    fn lone_keyup_is_a_noop() {
        let mut t = Tracker::new(F16);
        assert_eq!(t.on_raw(up(F16)), None);
        assert_eq!(t.on_raw(down(F16)), Some(HotkeyEvent::TriggerRecord));
    }

    #[test]
    fn different_trigger_code_is_respected() {
        // Constructing with code=A means F16 should NOT trigger.
        let mut t = Tracker::new(A);
        assert_eq!(t.on_raw(down(F16)), None);
        assert_eq!(t.on_raw(down(A)), Some(HotkeyEvent::TriggerRecord));
    }

    #[test]
    fn wire_roundtrip() {
        for (down, code) in [(true, 0x6Au16), (false, 0x6A), (true, 0x00), (false, 0xFFFF)] {
            let buf = RawKey::encode(down, code);
            let decoded = RawKey::decode(buf);
            assert_eq!(decoded, RawKey { down, code });
        }
    }
}
