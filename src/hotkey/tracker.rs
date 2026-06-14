//! Pure state machine: `RawEvent + Instant` → `HotkeyEvent`.
//!
//! Three trigger shapes from a [`Combo`] config (see [`super::combo`]):
//! pure key (e.g. `f16`), combo (e.g. `cmd+r`), modifier-only (e.g.
//! `right_shift`). `:double` is orthogonal — any shape may require two
//! taps within [`DOUBLE_TAP_WINDOW`] to fire.
//!
//! Semantics chosen for predictability:
//!
//! - **Pure key / combo**: fire on `KeyDown` of the configured key iff the
//!   modifier snapshot exactly matches at that instant. Auto-repeat
//!   `KeyDown`s are debounced (no second fire until `KeyUp`). Releasing /
//!   adding a modifier *while the key is held* does not fire — user must
//!   release and re-press the key. This matches VSCode-style binding
//!   semantics and is the least surprising.
//!
//! - **Modifier-only**: fire on a "clean tap" — the configured modifier
//!   combination becomes matched, then becomes unmatched via a release of
//!   one of the required modifiers, within [`MOD_HOLD_THRESHOLD`], with no
//!   intervening non-modifier key event and no extra modifier added in
//!   between (Karabiner `to_if_alone` semantics).
//!
//! - **Double-tap (`:double`)**: two consecutive "taps" (as defined above
//!   for the shape) within [`DOUBLE_TAP_WINDOW`] fire one `TriggerRecord`
//!   on the second one. The first tap is silent. There is no timer — the
//!   window is checked on the second tap. If the second never arrives,
//!   the stale `last_tap_at` is harmless and naturally overwritten by the
//!   next tap.
//!
//! No I/O, no time source — `Instant` is supplied by the caller so tests
//! drive the clock deterministically.

use std::time::{Duration, Instant};

use super::combo::Combo;
use super::parse::modifier_from_keycode;
use super::{EventKind, HotkeyEvent, RawEvent};

/// How long the configured modifier(s) may stay down before a release no
/// longer counts as a "tap". Chosen to match community-validated values
/// (BetterTouchTool / Hammerspoon ergonomics). Karabiner's default of 1000
/// ms feels laggy; 250 ms misses slow taps. 500 ms is the middle ground.
pub const MOD_HOLD_THRESHOLD: Duration = Duration::from_millis(500);

/// Window between the first and second tap of a `:double` trigger. macOS
/// Dictation uses ~350 ms for Right Shift x2; 400 ms gives 50 ms of slack.
pub const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(400);

#[derive(Debug)]
pub struct Tracker {
    trigger: Combo,

    /// Pure-key / combo: the configured key is currently held. Used to
    /// debounce auto-repeat. Reset on `KeyUp` of the trigger key or on
    /// trigger swap.
    key_held: bool,

    /// Modifier-only: timestamp when the configured modifier combination
    /// first became matched. `None` between taps.
    mod_match_since: Option<Instant>,

    /// Modifier-only: a non-modifier key event has occurred while the
    /// modifier match was active. Disqualifies the current candidate.
    mod_match_intervening: bool,

    /// Time of the most recent "tap" emitted by [`Self::register_tap`].
    /// Only consulted when `trigger.double` — checked on the next tap to
    /// decide whether it falls inside [`DOUBLE_TAP_WINDOW`].
    last_tap_at: Option<Instant>,
}

impl Tracker {
    pub fn new(trigger: Combo) -> Self {
        Self {
            trigger,
            key_held: false,
            mod_match_since: None,
            mod_match_intervening: false,
            last_tap_at: None,
        }
    }

    pub fn set_trigger(&mut self, trigger: Combo) {
        // Trigger swap: clear in-flight tap candidates so old state can't
        // bleed into the new trigger. Pressing held trigger keys at the
        // moment of swap is rare and recoverable by releasing+re-pressing.
        self.trigger = trigger;
        self.key_held = false;
        self.mod_match_since = None;
        self.mod_match_intervening = false;
        self.last_tap_at = None;
    }

    pub fn on_event(&mut self, ev: RawEvent, now: Instant) -> Option<HotkeyEvent> {
        match self.trigger.key {
            Some(key) => self.on_keyed_combo(ev, now, key),
            None => self.on_modifier_only(ev, now),
        }
    }

    // --- keyed combo / pure key ---

    fn on_keyed_combo(&mut self, ev: RawEvent, now: Instant, key: u16) -> Option<HotkeyEvent> {
        match ev.kind {
            EventKind::KeyDown if ev.code == key => {
                if self.key_held {
                    return None; // auto-repeat
                }
                self.key_held = true;
                if ev.mods.matches_combo(&self.trigger) {
                    self.register_tap(now)
                } else {
                    None
                }
            }
            EventKind::KeyUp if ev.code == key => {
                self.key_held = false;
                None
            }
            _ => None,
        }
    }

    // --- modifier-only ---

    fn on_modifier_only(&mut self, ev: RawEvent, now: Instant) -> Option<HotkeyEvent> {
        match ev.kind {
            EventKind::FlagsChanged => self.on_flags_changed(ev, now),
            EventKind::KeyDown | EventKind::KeyUp => {
                // Any non-modifier key activity during a pending candidate
                // disqualifies it (Karabiner `to_if_alone`).
                if self.mod_match_since.is_some() {
                    self.mod_match_intervening = true;
                }
                None
            }
        }
    }

    fn on_flags_changed(&mut self, ev: RawEvent, now: Instant) -> Option<HotkeyEvent> {
        let now_match = ev.mods.matches_combo(&self.trigger);
        let was_match = self.mod_match_since.is_some();

        if now_match && !was_match {
            // Required modifier combination just became satisfied.
            self.mod_match_since = Some(now);
            self.mod_match_intervening = false;
            return None;
        }

        if !now_match && was_match {
            // Match broken. Distinguish "required modifier released" (real
            // tap candidate) from "extra modifier added" (abort — the user
            // is composing a longer combo, not tapping).
            let started = self.mod_match_since.take().unwrap();
            let Some((ty, side)) = modifier_from_keycode(ev.code) else {
                // Match was broken by something other than a modifier
                // transition? Shouldn't happen — FlagsChanged events
                // always carry a modifier keycode on macOS. Treat as
                // abort.
                return None;
            };
            let mod_went_down = ev.mods.is_side_down(ty, side);
            if mod_went_down {
                // Extra modifier added; abort candidate silently.
                return None;
            }
            let dur = now.saturating_duration_since(started);
            if self.mod_match_intervening || dur > MOD_HOLD_THRESHOLD {
                return None;
            }
            return self.register_tap(now);
        }

        // No change in match status. Either both true (extra modifier
        // toggled within the EitherSide pair) or both false (event on a
        // modifier we don't track / care about). Nothing to do.
        None
    }

    // --- tap accounting ---

    /// Called when the shape-specific machinery has decided a tap occurred.
    /// Routes to single-emit or double-tap accounting based on the
    /// trigger configuration.
    fn register_tap(&mut self, now: Instant) -> Option<HotkeyEvent> {
        if !self.trigger.double {
            return Some(HotkeyEvent::TriggerRecord);
        }
        match self.last_tap_at {
            Some(prev) if now.saturating_duration_since(prev) <= DOUBLE_TAP_WINDOW => {
                // Second tap within window. Reset so a third tap doesn't
                // re-trigger off the second's timestamp.
                self.last_tap_at = None;
                Some(HotkeyEvent::TriggerRecord)
            }
            _ => {
                self.last_tap_at = Some(now);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::combo::{ModMask, ModMatcher, ModType, Side};

    // --- keycodes used throughout tests ---
    const F16: u16 = 0x6A;
    const R: u16 = 0x0F;
    const A: u16 = 0x00;
    const L_CMD: u16 = 0x37;
    const R_CMD: u16 = 0x36;
    const L_SHIFT: u16 = 0x38;
    const R_SHIFT: u16 = 0x3C;

    fn now_zero() -> Instant {
        Instant::now()
    }

    // --- combo constructors ---

    fn pure_key(code: u16, double: bool) -> Combo {
        Combo {
            mods: [ModMatcher::NotPresent; 4],
            key: Some(code),
            double,
        }
    }

    fn cmd_plus_key(code: u16, double: bool) -> Combo {
        let mut mods = [ModMatcher::NotPresent; 4];
        mods[ModType::Cmd as usize] = ModMatcher::EitherSide;
        Combo {
            mods,
            key: Some(code),
            double,
        }
    }

    fn right_shift(double: bool) -> Combo {
        let mut mods = [ModMatcher::NotPresent; 4];
        mods[ModType::Shift as usize] = ModMatcher::Specific(Side::Right);
        Combo {
            mods,
            key: None,
            double,
        }
    }

    // --- event constructors ---

    fn key_down(code: u16, mods: ModMask) -> RawEvent {
        RawEvent {
            kind: EventKind::KeyDown,
            code,
            mods,
        }
    }

    fn key_up(code: u16, mods: ModMask) -> RawEvent {
        RawEvent {
            kind: EventKind::KeyUp,
            code,
            mods,
        }
    }

    fn flags(code: u16, mods: ModMask) -> RawEvent {
        RawEvent {
            kind: EventKind::FlagsChanged,
            code,
            mods,
        }
    }

    fn mods_with(set: &[(ModType, Side)]) -> ModMask {
        let mut m = ModMask::empty();
        for (ty, side) in set {
            m.set(*ty, *side, true);
        }
        m
    }

    // ---------- pure key ----------

    #[test]
    fn pure_key_fires_on_keydown() {
        let mut t = Tracker::new(pure_key(F16, false));
        let now = now_zero();
        assert_eq!(
            t.on_event(key_down(F16, ModMask::empty()), now),
            Some(HotkeyEvent::TriggerRecord)
        );
    }

    #[test]
    fn pure_key_debounces_auto_repeat() {
        let mut t = Tracker::new(pure_key(F16, false));
        let now = now_zero();
        t.on_event(key_down(F16, ModMask::empty()), now);
        assert_eq!(t.on_event(key_down(F16, ModMask::empty()), now), None);
        assert_eq!(t.on_event(key_down(F16, ModMask::empty()), now), None);
    }

    #[test]
    fn pure_key_rearms_after_keyup() {
        let mut t = Tracker::new(pure_key(F16, false));
        let now = now_zero();
        t.on_event(key_down(F16, ModMask::empty()), now);
        t.on_event(key_up(F16, ModMask::empty()), now);
        assert_eq!(
            t.on_event(key_down(F16, ModMask::empty()), now),
            Some(HotkeyEvent::TriggerRecord)
        );
    }

    #[test]
    fn pure_key_does_not_fire_with_extra_modifier() {
        let mut t = Tracker::new(pure_key(F16, false));
        let now = now_zero();
        let mods = mods_with(&[(ModType::Shift, Side::Left)]);
        assert_eq!(t.on_event(key_down(F16, mods), now), None);
    }

    // ---------- combo ----------

    #[test]
    fn combo_fires_on_key_when_mods_match() {
        let mut t = Tracker::new(cmd_plus_key(R, false));
        let now = now_zero();
        let mods = mods_with(&[(ModType::Cmd, Side::Left)]);
        assert_eq!(
            t.on_event(key_down(R, mods), now),
            Some(HotkeyEvent::TriggerRecord)
        );
    }

    #[test]
    fn combo_does_not_fire_without_required_modifier() {
        let mut t = Tracker::new(cmd_plus_key(R, false));
        let now = now_zero();
        assert_eq!(t.on_event(key_down(R, ModMask::empty()), now), None);
    }

    #[test]
    fn combo_rejects_extra_modifier_at_keydown() {
        let mut t = Tracker::new(cmd_plus_key(R, false));
        let now = now_zero();
        let mods = mods_with(&[(ModType::Cmd, Side::Left), (ModType::Shift, Side::Left)]);
        assert_eq!(t.on_event(key_down(R, mods), now), None);
    }

    // ---------- modifier-only ----------

    #[test]
    fn modifier_only_fires_on_clean_tap() {
        let mut t = Tracker::new(right_shift(false));
        let now = now_zero();
        // RShift down.
        let after_down = t.on_event(
            flags(R_SHIFT, mods_with(&[(ModType::Shift, Side::Right)])),
            now,
        );
        assert_eq!(after_down, None);
        // RShift up shortly after.
        let later = now + Duration::from_millis(100);
        let after_up = t.on_event(flags(R_SHIFT, ModMask::empty()), later);
        assert_eq!(after_up, Some(HotkeyEvent::TriggerRecord));
    }

    #[test]
    fn modifier_only_does_not_fire_when_held_too_long() {
        let mut t = Tracker::new(right_shift(false));
        let now = now_zero();
        t.on_event(
            flags(R_SHIFT, mods_with(&[(ModType::Shift, Side::Right)])),
            now,
        );
        let later = now + MOD_HOLD_THRESHOLD + Duration::from_millis(1);
        assert_eq!(t.on_event(flags(R_SHIFT, ModMask::empty()), later), None);
    }

    #[test]
    fn modifier_only_aborts_on_intervening_key() {
        let mut t = Tracker::new(right_shift(false));
        let now = now_zero();
        let mods = mods_with(&[(ModType::Shift, Side::Right)]);
        t.on_event(flags(R_SHIFT, mods), now);
        // User presses A while shift is held — this is shift+a, not a tap.
        t.on_event(key_down(A, mods), now + Duration::from_millis(50));
        t.on_event(key_up(A, mods), now + Duration::from_millis(80));
        assert_eq!(
            t.on_event(
                flags(R_SHIFT, ModMask::empty()),
                now + Duration::from_millis(120)
            ),
            None
        );
    }

    #[test]
    fn modifier_only_aborts_when_extra_modifier_added() {
        let mut t = Tracker::new(right_shift(false));
        let now = now_zero();
        // RShift down.
        t.on_event(
            flags(R_SHIFT, mods_with(&[(ModType::Shift, Side::Right)])),
            now,
        );
        // Cmd pressed while shift held — break match by extra mod.
        let combined = mods_with(&[(ModType::Shift, Side::Right), (ModType::Cmd, Side::Left)]);
        let r = t.on_event(flags(L_CMD, combined), now + Duration::from_millis(50));
        assert_eq!(r, None);
        // Cmd released — match becomes true again, this is a fresh candidate.
        let r = t.on_event(
            flags(L_CMD, mods_with(&[(ModType::Shift, Side::Right)])),
            now + Duration::from_millis(80),
        );
        assert_eq!(r, None);
        // RShift released soon enough → tap fires (within new candidate window).
        let r = t.on_event(
            flags(R_SHIFT, ModMask::empty()),
            now + Duration::from_millis(100),
        );
        assert_eq!(r, Some(HotkeyEvent::TriggerRecord));
    }

    #[test]
    fn modifier_only_ignores_wrong_side() {
        let mut t = Tracker::new(right_shift(false));
        let now = now_zero();
        // Left shift down/up should not satisfy "right shift" trigger.
        t.on_event(
            flags(L_SHIFT, mods_with(&[(ModType::Shift, Side::Left)])),
            now,
        );
        let later = now + Duration::from_millis(100);
        assert_eq!(t.on_event(flags(L_SHIFT, ModMask::empty()), later), None);
    }

    // ---------- double tap ----------

    #[test]
    fn pure_key_double_tap_fires_on_second() {
        let mut t = Tracker::new(pure_key(F16, true));
        let now = now_zero();
        assert_eq!(t.on_event(key_down(F16, ModMask::empty()), now), None);
        t.on_event(key_up(F16, ModMask::empty()), now);
        let later = now + Duration::from_millis(200);
        assert_eq!(
            t.on_event(key_down(F16, ModMask::empty()), later),
            Some(HotkeyEvent::TriggerRecord)
        );
    }

    #[test]
    fn double_tap_misses_outside_window() {
        let mut t = Tracker::new(pure_key(F16, true));
        let now = now_zero();
        t.on_event(key_down(F16, ModMask::empty()), now);
        t.on_event(key_up(F16, ModMask::empty()), now);
        let late = now + DOUBLE_TAP_WINDOW + Duration::from_millis(1);
        // Too slow → counts as a fresh first tap, doesn't fire.
        assert_eq!(t.on_event(key_down(F16, ModMask::empty()), late), None);
        t.on_event(key_up(F16, ModMask::empty()), late);
        // A third quick press now fires off the second tap's timestamp.
        assert_eq!(
            t.on_event(
                key_down(F16, ModMask::empty()),
                late + Duration::from_millis(100)
            ),
            Some(HotkeyEvent::TriggerRecord)
        );
    }

    #[test]
    fn modifier_only_double_tap() {
        let mut t = Tracker::new(right_shift(true));
        let now = now_zero();
        let down = mods_with(&[(ModType::Shift, Side::Right)]);
        // First tap.
        t.on_event(flags(R_SHIFT, down), now);
        let r = t.on_event(
            flags(R_SHIFT, ModMask::empty()),
            now + Duration::from_millis(100),
        );
        assert_eq!(r, None, "first tap of :double must not fire");
        // Second tap within window.
        let t2_start = now + Duration::from_millis(200);
        t.on_event(flags(R_SHIFT, down), t2_start);
        let r = t.on_event(
            flags(R_SHIFT, ModMask::empty()),
            t2_start + Duration::from_millis(80),
        );
        assert_eq!(r, Some(HotkeyEvent::TriggerRecord));
    }

    #[test]
    fn combo_double_tap_through_modifier_release() {
        // Trigger: cmd+r:double. User presses cmd+r, releases r, presses
        // r again still holding cmd, within window.
        let mut t = Tracker::new(cmd_plus_key(R, true));
        let now = now_zero();
        let cmd = mods_with(&[(ModType::Cmd, Side::Left)]);
        assert_eq!(t.on_event(key_down(R, cmd), now), None);
        t.on_event(key_up(R, cmd), now + Duration::from_millis(50));
        let r = t.on_event(key_down(R, cmd), now + Duration::from_millis(150));
        assert_eq!(r, Some(HotkeyEvent::TriggerRecord));
    }

    // ---------- trigger swap ----------

    #[test]
    fn set_trigger_clears_in_flight_state() {
        let mut t = Tracker::new(pure_key(F16, false));
        let now = now_zero();
        t.on_event(key_down(F16, ModMask::empty()), now);
        // Swap.
        t.set_trigger(pure_key(R, false));
        // Old key release shouldn't error or leak state.
        t.on_event(key_up(F16, ModMask::empty()), now);
        assert_eq!(
            t.on_event(key_down(R, ModMask::empty()), now),
            Some(HotkeyEvent::TriggerRecord)
        );
    }

    #[test]
    fn other_side_cmd_does_not_count_when_right_required() {
        // Sanity: keyed combo with side-specific modifier ignores wrong side.
        // The realistic user sequence is "press wrong, release, press right";
        // we don't get two trigger KeyDowns without an intervening KeyUp at
        // the OS level (auto-repeat reuses the original press, not a new one).
        let mut combo = cmd_plus_key(R, false);
        combo.mods[ModType::Cmd as usize] = ModMatcher::Specific(Side::Right);
        let mut t = Tracker::new(combo);
        let now = now_zero();
        let left_cmd = mods_with(&[(ModType::Cmd, Side::Left)]);
        assert_eq!(t.on_event(key_down(R, left_cmd), now), None);
        t.on_event(key_up(R, left_cmd), now);
        let right_cmd = mods_with(&[(ModType::Cmd, Side::Right)]);
        assert_eq!(
            t.on_event(key_down(R, right_cmd), now),
            Some(HotkeyEvent::TriggerRecord)
        );
    }

    #[test]
    fn unused_constants_ref_silence_warnings() {
        // Keep constants referenced even if test set narrows later.
        let _ = (L_CMD, R_CMD, L_SHIFT, A);
    }
}
