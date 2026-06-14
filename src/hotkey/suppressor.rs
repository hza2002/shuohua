//! Pure state machine: decide whether a `RawKey` should be suppressed before
//! it reaches the foreground app. Mirror of the §5 invariant 8 contract.
//!
//! Inputs come from the CGEventTap callback (one per system key event); the
//! output is the `suppress: bool` returned by the callback. The state machine
//! is independent from the Tracker that emits `HotkeyEvent` — the callback
//! always also forwards events into the pipe so the Tracker keeps working
//! even for keys that we suppress.
//!
//! Invariants enforced (covered by [`tests`] + `proptest`):
//!
//! 1. **Down/up pairing**. Once a code C is added to `held` by a suppressed
//!    KeyDown, the *next* KeyUp of C is also suppressed and removes C from
//!    `held`. Foreground apps therefore never see an orphan KeyUp whose
//!    matching KeyDown was eaten.
//! 2. **Trigger hot-swap is safe mid-hold**. Changing the trigger code while
//!    a key is held does not orphan its KeyUp — pairing is keyed off `held`,
//!    not the current `trigger_code`.
//! 3. **Auto-repeat keeps suppressing**. Subsequent KeyDowns of a code that
//!    is already in `held` are suppressed (foreground app never sees the
//!    OS-generated repeats either).
//! 4. **Non-trigger keys pass through**. Any KeyDown of a code that is
//!    neither the current trigger nor already held is forwarded unchanged,
//!    and so is its KeyUp.

use super::RawKey;

#[derive(Debug)]
pub struct Suppressor {
    trigger_code: u16,
    held: Vec<u16>,
}

impl Suppressor {
    pub fn new(trigger_code: u16) -> Self {
        Self {
            trigger_code,
            held: Vec::new(),
        }
    }

    pub fn set_trigger(&mut self, code: u16) {
        self.trigger_code = code;
    }

    /// Returns `true` when the OS-level event should be dropped.
    pub fn on_raw(&mut self, raw: RawKey) -> bool {
        let already_held = self.held.contains(&raw.code);
        if raw.down {
            if raw.code == self.trigger_code || already_held {
                if !already_held {
                    self.held.push(raw.code);
                }
                true
            } else {
                false
            }
        } else if already_held {
            self.held.retain(|c| *c != raw.code);
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    pub fn held(&self) -> &[u16] {
        &self.held
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F16: u16 = 0x6A;
    const F17: u16 = 0x40;
    const A: u16 = 0x00;

    fn down(code: u16) -> RawKey {
        RawKey { down: true, code }
    }
    fn up(code: u16) -> RawKey {
        RawKey { down: false, code }
    }

    #[test]
    fn trigger_keydown_is_suppressed_and_marks_held() {
        let mut s = Suppressor::new(F16);
        assert!(s.on_raw(down(F16)));
        assert_eq!(s.held(), &[F16]);
    }

    #[test]
    fn matching_keyup_is_suppressed_and_clears_held() {
        let mut s = Suppressor::new(F16);
        s.on_raw(down(F16));
        assert!(s.on_raw(up(F16)));
        assert!(s.held().is_empty());
    }

    #[test]
    fn auto_repeat_keydown_keeps_suppressing() {
        let mut s = Suppressor::new(F16);
        s.on_raw(down(F16));
        assert!(s.on_raw(down(F16)));
        assert!(s.on_raw(down(F16)));
        assert_eq!(s.held(), &[F16]); // still exactly one entry
    }

    #[test]
    fn non_trigger_key_passes_through() {
        let mut s = Suppressor::new(F16);
        assert!(!s.on_raw(down(A)));
        assert!(!s.on_raw(up(A)));
        assert!(s.held().is_empty());
    }

    #[test]
    fn lone_keyup_passes_through() {
        // Stray KeyUp without matching KeyDown (e.g. process started while key
        // was already held). Must NOT be suppressed — foreground app needs it
        // to clear its own pressed-key state.
        let mut s = Suppressor::new(F16);
        assert!(!s.on_raw(up(F16)));
        assert!(s.held().is_empty());
    }

    #[test]
    fn trigger_swap_mid_hold_still_pairs_keyup() {
        // User holds F16, config reload changes trigger to F17, user releases F16.
        // The F16 KeyUp must still be suppressed (invariant 2).
        let mut s = Suppressor::new(F16);
        assert!(s.on_raw(down(F16)));
        s.set_trigger(F17);
        assert!(s.on_raw(up(F16)), "F16 KeyUp must be suppressed");
        assert!(s.held().is_empty());
        // After swap, F16 is back to a normal key:
        assert!(!s.on_raw(down(F16)));
    }

    #[test]
    fn trigger_swap_mid_hold_new_trigger_works_immediately() {
        let mut s = Suppressor::new(F16);
        s.on_raw(down(F16));
        s.set_trigger(F17);
        assert!(s.on_raw(down(F17)));
        assert!(s.on_raw(up(F17)));
        assert!(s.on_raw(up(F16))); // still paired
    }

    #[test]
    fn keyup_of_non_held_passes_even_with_matching_trigger_code() {
        // Edge case: spurious KeyUp of the current trigger when we never saw
        // its KeyDown (e.g. provider started mid-hold). Must pass through.
        let mut s = Suppressor::new(F16);
        assert!(!s.on_raw(up(F16)));
    }
}
