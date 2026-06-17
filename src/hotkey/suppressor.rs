//! Decide whether to drop a `RawEvent` so it never reaches the foreground
//! app. Pure state; the CGEventTap callback wraps an instance in
//! `Mutex<Suppressor>`.
//!
//! Per-trigger-type behavior:
//!
//! - **Pure key / combo (`trigger.key` is `Some`)**: suppress the
//!   configured key's `KeyDown` (when modifiers exactly match at that
//!   instant), plus its auto-repeat `KeyDown`s and the matching `KeyUp`.
//!   Modifier events are never suppressed — apps that rely on `Cmd` /
//!   `Shift` transitions need them.
//!
//! - **Modifier-only**: nothing is suppressed. Stealing modifier events
//!   breaks far too many foreground interactions (text selection, app
//!   shortcuts, system gestures). The cost is that the foreground app
//!   sees a brief modifier flash when the user taps the trigger —
//!   imperceptible in practice and matches macOS Dictation's behavior.
//!
//! Cancel uses the same reserved-key behavior while recording is active.
//! Outside recording, cancel is not suppressed so normal Escape / Delete /
//! app shortcuts keep working.
//!
//! The held-key set is independent of `trigger.key`: once a code is
//! suppressed on `KeyDown`, its `KeyUp` is suppressed too, even if the
//! binding has been re-bound mid-hold (§5 invariant 8). Auto-repeat
//! `KeyDown`s of a held code are also suppressed.

use super::combo::Combo;
use super::{EventKind, RawEvent};

#[derive(Debug)]
pub struct Suppressor {
    trigger: Combo,
    cancel: Option<Combo>,
    cancel_active: bool,
    /// Physical keycodes we've eaten the down of and not yet seen the up.
    held: Vec<u16>,
}

impl Suppressor {
    pub fn new(trigger: Combo) -> Self {
        Self {
            trigger,
            cancel: None,
            cancel_active: false,
            held: Vec::new(),
        }
    }

    pub fn set_trigger(&mut self, trigger: Combo) {
        self.trigger = trigger;
        // Intentionally keep `held` — see §5 invariant 8: a key whose down
        // was suppressed must still have its up suppressed even if the
        // trigger has changed.
    }

    pub fn set_cancel(&mut self, cancel: Combo) {
        self.cancel = Some(cancel);
        // Intentionally keep `held`; see `set_trigger`.
    }

    pub fn set_cancel_active(&mut self, active: bool) {
        self.cancel_active = active;
    }

    /// Returns `true` when the OS-level event should be dropped.
    pub fn on_raw(&mut self, ev: RawEvent) -> bool {
        let already_held = self.held.contains(&ev.code);
        match ev.kind {
            EventKind::KeyDown => {
                if already_held {
                    return true; // auto-repeat of a key whose down was eaten
                }
                if self.should_suppress_fresh_down(ev) {
                    self.held.push(ev.code);
                    return true;
                }
                false
            }
            EventKind::KeyUp => {
                if already_held {
                    self.held.retain(|c| *c != ev.code);
                    return true;
                }
                false
            }
            EventKind::FlagsChanged => {
                // Modifier transitions always flow through (see module docs).
                false
            }
        }
    }

    fn should_suppress_fresh_down(&self, ev: RawEvent) -> bool {
        matches_keyed_binding(&self.trigger, ev)
            || (self.cancel_active
                && self
                    .cancel
                    .as_ref()
                    .is_some_and(|cancel| matches_keyed_binding(cancel, ev)))
    }

    #[cfg(test)]
    pub fn held(&self) -> &[u16] {
        &self.held
    }
}

fn matches_keyed_binding(binding: &Combo, ev: RawEvent) -> bool {
    let Some(key) = binding.key else {
        return false; // modifier-only binding: nothing to suppress
    };
    ev.code == key && ev.mods.matches_combo(binding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::combo::{ModMask, ModMatcher, ModType, Side};

    const F16: u16 = 0x6A;
    const F17: u16 = 0x40;
    const R: u16 = 0x0F;
    const A: u16 = 0x00;
    const L_CMD: u16 = 0x37;

    fn pure_key(code: u16) -> Combo {
        Combo {
            mods: [ModMatcher::NotPresent; 4],
            key: Some(code),
            double: false,
        }
    }

    fn pure_key_double(code: u16) -> Combo {
        Combo {
            mods: [ModMatcher::NotPresent; 4],
            key: Some(code),
            double: true,
        }
    }

    fn cmd_plus(code: u16) -> Combo {
        let mut mods = [ModMatcher::NotPresent; 4];
        mods[ModType::Cmd as usize] = ModMatcher::EitherSide;
        Combo {
            mods,
            key: Some(code),
            double: false,
        }
    }

    fn right_shift_only() -> Combo {
        let mut mods = [ModMatcher::NotPresent; 4];
        mods[ModType::Shift as usize] = ModMatcher::Specific(Side::Right);
        Combo {
            mods,
            key: None,
            double: false,
        }
    }

    fn cmd_mod() -> ModMask {
        let mut m = ModMask::empty();
        m.set(ModType::Cmd, Side::Left, true);
        m
    }

    fn down(code: u16, mods: ModMask) -> RawEvent {
        RawEvent {
            kind: EventKind::KeyDown,
            code,
            mods,
        }
    }
    fn up(code: u16, mods: ModMask) -> RawEvent {
        RawEvent {
            kind: EventKind::KeyUp,
            code,
            mods,
        }
    }
    fn flag(code: u16, mods: ModMask) -> RawEvent {
        RawEvent {
            kind: EventKind::FlagsChanged,
            code,
            mods,
        }
    }

    // ---------- pure key ----------

    #[test]
    fn pure_key_suppresses_full_press_cycle() {
        let mut s = Suppressor::new(pure_key(F16));
        assert!(s.on_raw(down(F16, ModMask::empty())));
        assert!(s.on_raw(down(F16, ModMask::empty()))); // auto-repeat
        assert!(s.on_raw(up(F16, ModMask::empty())));
        assert!(s.held().is_empty());
    }

    #[test]
    fn pure_key_lone_keyup_passes() {
        let mut s = Suppressor::new(pure_key(F16));
        assert!(!s.on_raw(up(F16, ModMask::empty())));
    }

    #[test]
    fn pure_key_does_not_suppress_other_keys() {
        let mut s = Suppressor::new(pure_key(F16));
        assert!(!s.on_raw(down(A, ModMask::empty())));
        assert!(!s.on_raw(up(A, ModMask::empty())));
    }

    #[test]
    fn pure_key_does_not_suppress_when_extra_mods_present() {
        let mut s = Suppressor::new(pure_key(F16));
        let m = cmd_mod();
        assert!(!s.on_raw(down(F16, m)));
        // KeyUp not held → also pass through.
        assert!(!s.on_raw(up(F16, m)));
    }

    #[test]
    fn active_cancel_double_suppresses_both_press_cycles() {
        let mut s = Suppressor::new(pure_key(F16));
        s.set_cancel(pure_key_double(0x35));
        s.set_cancel_active(true);

        assert!(s.on_raw(down(0x35, ModMask::empty())));
        assert!(s.on_raw(up(0x35, ModMask::empty())));
        assert!(s.on_raw(down(0x35, ModMask::empty())));
        assert!(s.on_raw(up(0x35, ModMask::empty())));
    }

    #[test]
    fn inactive_cancel_passes_through() {
        let mut s = Suppressor::new(pure_key(F16));
        s.set_cancel(pure_key_double(0x35));
        s.set_cancel_active(false);

        assert!(!s.on_raw(down(0x35, ModMask::empty())));
        assert!(!s.on_raw(up(0x35, ModMask::empty())));
    }

    // ---------- combo ----------

    #[test]
    fn combo_suppresses_key_only_when_mods_match() {
        let mut s = Suppressor::new(cmd_plus(R));
        // Without cmd: pass through.
        assert!(!s.on_raw(down(R, ModMask::empty())));
        assert!(!s.on_raw(up(R, ModMask::empty())));
        // With cmd: suppress key.
        assert!(s.on_raw(down(R, cmd_mod())));
        assert!(s.on_raw(up(R, cmd_mod())));
    }

    #[test]
    fn combo_does_not_suppress_modifier_events() {
        let mut s = Suppressor::new(cmd_plus(R));
        assert!(!s.on_raw(flag(L_CMD, cmd_mod())));
        assert!(!s.on_raw(flag(L_CMD, ModMask::empty())));
    }

    #[test]
    fn combo_keyup_after_mod_release_still_suppressed() {
        // User presses cmd+r, releases cmd, then releases r. The r-up
        // should still be suppressed because its down was suppressed.
        let mut s = Suppressor::new(cmd_plus(R));
        assert!(s.on_raw(down(R, cmd_mod())));
        // Cmd released — modifier event passes through, doesn't affect held.
        assert!(!s.on_raw(flag(L_CMD, ModMask::empty())));
        assert!(s.on_raw(up(R, ModMask::empty())));
        assert!(s.held().is_empty());
    }

    // ---------- modifier-only ----------

    #[test]
    fn modifier_only_suppresses_nothing() {
        let mut s = Suppressor::new(right_shift_only());
        assert!(!s.on_raw(flag(0x3C, ModMask::empty())));
        assert!(!s.on_raw(down(A, ModMask::empty())));
        assert!(!s.on_raw(up(A, ModMask::empty())));
    }

    // ---------- trigger swap ----------

    #[test]
    fn trigger_swap_preserves_held_pairing() {
        // Held key's up must be suppressed even if trigger changes
        // mid-hold (§5 invariant 8).
        let mut s = Suppressor::new(pure_key(F16));
        assert!(s.on_raw(down(F16, ModMask::empty())));
        s.set_trigger(pure_key(F17));
        assert!(s.on_raw(up(F16, ModMask::empty())));
        assert!(s.held().is_empty());
    }

    #[test]
    fn trigger_swap_to_modifier_only_still_pairs_old_keyup() {
        let mut s = Suppressor::new(pure_key(F16));
        assert!(s.on_raw(down(F16, ModMask::empty())));
        s.set_trigger(right_shift_only());
        // F16 release must still be eaten — the foreground app must not
        // see an orphan KeyUp.
        assert!(s.on_raw(up(F16, ModMask::empty())));
    }
}
