//! Property tests for hotkey state machines.
//!
//! Two state machines are exercised: `Suppressor` (callback-side, decides
//! whether to drop the OS event — guards §5 invariant 8) and `Tracker`
//! (tokio-side, debounces auto-repeat into a single `TriggerRecord`).
//!
//! Each test runs a generated sequence against the impl and an independent
//! reference model. The reference model IS the spec; agreement after every
//! step is the property.

use super::{HotkeyEvent, RawKey, Suppressor, Tracker};
use proptest::prelude::*;
use std::collections::HashSet;

const F16: u16 = 0x6A;
const F17: u16 = 0x40;
const A: u16 = 0x00;
const B: u16 = 0x0B;

/// Constrain the alphabet so shrinking converges quickly.
fn keycode() -> impl Strategy<Value = u16> {
    prop_oneof![Just(F16), Just(F17), Just(A), Just(B)]
}

fn raw_key() -> impl Strategy<Value = RawKey> {
    (any::<bool>(), keycode()).prop_map(|(down, code)| RawKey { down, code })
}

#[derive(Debug, Clone)]
enum SuppressAction {
    Raw(RawKey),
    SetTrigger(u16),
}

fn suppress_action() -> impl Strategy<Value = SuppressAction> {
    prop_oneof![
        9 => raw_key().prop_map(SuppressAction::Raw),
        1 => keycode().prop_map(SuppressAction::SetTrigger),
    ]
}

proptest! {
    /// `Suppressor` matches a HashSet-based reference model on every step.
    /// Covers down/up pairing, trigger hot-swap, auto-repeat, and lone keyups
    /// in one sweep.
    #[test]
    fn suppressor_matches_reference_model(
        initial_trigger in keycode(),
        actions in proptest::collection::vec(suppress_action(), 0..64),
    ) {
        let mut s = Suppressor::new(initial_trigger);
        let mut model_held: HashSet<u16> = HashSet::new();
        let mut model_trigger = initial_trigger;

        for action in actions {
            match action {
                SuppressAction::SetTrigger(code) => {
                    s.set_trigger(code);
                    model_trigger = code;
                }
                SuppressAction::Raw(raw) => {
                    let actual = s.on_raw(raw);
                    let expected = if raw.down {
                        if raw.code == model_trigger || model_held.contains(&raw.code) {
                            model_held.insert(raw.code);
                            true
                        } else {
                            false
                        }
                    } else {
                        model_held.remove(&raw.code)
                    };
                    prop_assert_eq!(actual, expected, "raw={:?} trigger={:#x}", raw, model_trigger);

                    // Held sets must agree.
                    let impl_held: HashSet<u16> = s.held().iter().copied().collect();
                    prop_assert_eq!(impl_held, model_held.clone());
                }
            }
        }
    }

    /// §5 invariant 8 lifted to a global property: across an arbitrary
    /// sequence, the *count* of suppressed KeyDowns per code equals the count
    /// of suppressed KeyUps for that code, plus 1 if still held at end.
    #[test]
    fn suppressed_downs_pair_with_suppressed_ups(
        initial_trigger in keycode(),
        actions in proptest::collection::vec(suppress_action(), 0..64),
    ) {
        let mut s = Suppressor::new(initial_trigger);
        let mut down_count = std::collections::HashMap::<u16, i32>::new();
        let mut up_count = std::collections::HashMap::<u16, i32>::new();

        for action in actions {
            match action {
                SuppressAction::SetTrigger(code) => s.set_trigger(code),
                SuppressAction::Raw(raw) => {
                    let suppressed = s.on_raw(raw);
                    if suppressed {
                        if raw.down {
                            // Only count the *first* suppressed down per held cycle,
                            // since auto-repeats reuse the same `held` slot.
                            if !s.held().contains(&raw.code) {
                                unreachable!("a suppressed down must leave the code in held");
                            }
                            // Increment only if this was the cycle-start (transition
                            // from absent → present). Reference: track via separate
                            // model below.
                        }
                    }
                    // Replay the same event into the simple counter model:
                    //   - count keydowns of trigger / already-held only when entering
                    //   - count keyups when leaving
                    let was_held_before = down_count.get(&raw.code).copied().unwrap_or(0)
                        > up_count.get(&raw.code).copied().unwrap_or(0);
                    let _ = was_held_before; // not needed; the suppressed-flag drives counts
                    if suppressed && raw.down && !was_held_before {
                        *down_count.entry(raw.code).or_default() += 1;
                    }
                    if suppressed && !raw.down {
                        *up_count.entry(raw.code).or_default() += 1;
                    }
                }
            }
        }

        // For each code: suppressed-up count ≤ suppressed-down count, and the
        // difference ≤ 1 (only when the key is still held at the end).
        let still_held: HashSet<u16> = s.held().iter().copied().collect();
        for code in down_count.keys().chain(up_count.keys()).copied().collect::<HashSet<_>>() {
            let d = down_count.get(&code).copied().unwrap_or(0);
            let u = up_count.get(&code).copied().unwrap_or(0);
            let diff = d - u;
            let expected_diff = if still_held.contains(&code) { 1 } else { 0 };
            prop_assert_eq!(
                diff, expected_diff,
                "code={:#x} downs={} ups={} held={}",
                code, d, u, still_held.contains(&code)
            );
        }
    }

    /// `Tracker` emits exactly one `TriggerRecord` per fresh down-cycle of the
    /// trigger key. Reference model: emit on `down && !pressed`.
    #[test]
    fn tracker_matches_reference_model(
        trigger in keycode(),
        events in proptest::collection::vec(raw_key(), 0..64),
    ) {
        let mut t = Tracker::new(trigger);
        let mut model_pressed = false;

        for raw in events {
            let actual = t.on_raw(raw);
            let expected = if raw.code != trigger {
                None
            } else if raw.down && !model_pressed {
                model_pressed = true;
                Some(HotkeyEvent::TriggerRecord)
            } else if raw.down {
                None // auto-repeat
            } else {
                model_pressed = false;
                None
            };
            prop_assert_eq!(actual, expected, "raw={:?} pressed_before={}", raw, !model_pressed);
        }
    }

    /// Non-trigger keys never affect Tracker emission count.
    #[test]
    fn tracker_ignores_non_trigger_codes(
        trigger in keycode(),
        noise in proptest::collection::vec(raw_key(), 0..32),
    ) {
        let mut t = Tracker::new(trigger);
        let noise: Vec<_> = noise.into_iter().filter(|r| r.code != trigger).collect();
        for raw in &noise {
            prop_assert_eq!(t.on_raw(*raw), None);
        }
        // Now feed a clean trigger cycle and confirm it still fires.
        prop_assert_eq!(t.on_raw(RawKey { down: true, code: trigger }), Some(HotkeyEvent::TriggerRecord));
    }
}
