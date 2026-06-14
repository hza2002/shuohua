//! Property tests for hotkey state machines.
//!
//! Two state machines exercised:
//!
//! - [`Suppressor`]: covered against a HashSet reference model across
//!   arbitrary sequences of `KeyDown` / `KeyUp` plus occasional trigger
//!   swaps. Guards §5 invariant 8 (down/up pairing keyed off `held`, not
//!   the current trigger).
//!
//! - [`Tracker`] (pure-key only): covered against a bool reference model
//!   for auto-repeat debounce. Combo / modifier-only / double-tap state
//!   machines have richer unit tests in their own modules; their
//!   property-test coverage is intentionally limited here since the
//!   reference model has to mirror almost the entire impl.

use super::combo::{Combo, ModMatcher};
use super::{EventKind, HotkeyEvent, ModMask, RawEvent, Suppressor, Tracker};
use proptest::prelude::*;
use std::collections::HashSet;
use std::time::{Duration, Instant};

const F16: u16 = 0x6A;
const F17: u16 = 0x40;
const A: u16 = 0x00;
const B: u16 = 0x0B;

fn keycode() -> impl Strategy<Value = u16> {
    prop_oneof![Just(F16), Just(F17), Just(A), Just(B)]
}

fn key_event_kind() -> impl Strategy<Value = EventKind> {
    prop_oneof![Just(EventKind::KeyDown), Just(EventKind::KeyUp)]
}

fn key_event() -> impl Strategy<Value = RawEvent> {
    (key_event_kind(), keycode()).prop_map(|(kind, code)| RawEvent {
        kind,
        code,
        mods: ModMask::empty(),
    })
}

fn pure_key_combo(code: u16) -> Combo {
    Combo {
        mods: [ModMatcher::NotPresent; 4],
        key: Some(code),
        double: false,
    }
}

#[derive(Debug, Clone)]
enum Action {
    Event(RawEvent),
    SwapTrigger(u16),
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        9 => key_event().prop_map(Action::Event),
        1 => keycode().prop_map(Action::SwapTrigger),
    ]
}

proptest! {
    /// `Suppressor` matches a HashSet-based reference model on every step.
    /// The model encodes: KeyDown of (current trigger OR already-held)
    /// adds to held + returns true; KeyUp of held removes + returns true;
    /// everything else returns false. Trigger swap preserves the held set.
    #[test]
    fn suppressor_matches_reference_model(
        initial in keycode(),
        actions in proptest::collection::vec(action(), 0..64),
    ) {
        let mut s = Suppressor::new(pure_key_combo(initial));
        let mut model_held: HashSet<u16> = HashSet::new();
        let mut model_trigger = initial;

        for a in actions {
            match a {
                Action::SwapTrigger(code) => {
                    s.set_trigger(pure_key_combo(code));
                    model_trigger = code;
                }
                Action::Event(ev) => {
                    let actual = s.on_raw(ev);
                    let expected = match ev.kind {
                        EventKind::KeyDown => {
                            if model_held.contains(&ev.code) {
                                true
                            } else if ev.code == model_trigger {
                                // ev.mods is always empty for pure-key combos,
                                // and pure-key combo requires empty mods → match.
                                model_held.insert(ev.code);
                                true
                            } else {
                                false
                            }
                        }
                        EventKind::KeyUp => model_held.remove(&ev.code),
                        EventKind::FlagsChanged => false,
                    };
                    prop_assert_eq!(actual, expected, "ev={:?} trigger={:#x}", ev, model_trigger);
                    let impl_held: HashSet<u16> = s.held().iter().copied().collect();
                    prop_assert_eq!(impl_held, model_held.clone());
                }
            }
        }
    }

    /// Global down/up pairing: for any code, the number of suppressed
    /// `KeyDown`s entering a held cycle equals the number of suppressed
    /// `KeyUp`s plus 1 if the code is still held at the end. Protects the
    /// §5 invariant 8 across an arbitrary sequence (no orphan eaten
    /// `KeyUp` either way).
    #[test]
    fn suppressed_downs_pair_with_suppressed_ups(
        initial in keycode(),
        actions in proptest::collection::vec(action(), 0..64),
    ) {
        let mut s = Suppressor::new(pure_key_combo(initial));
        let mut entered = std::collections::HashMap::<u16, i32>::new();
        let mut left = std::collections::HashMap::<u16, i32>::new();
        let mut held_set: HashSet<u16> = HashSet::new();

        for a in actions {
            match a {
                Action::SwapTrigger(code) => s.set_trigger(pure_key_combo(code)),
                Action::Event(ev) => {
                    let suppressed = s.on_raw(ev);
                    match ev.kind {
                        EventKind::KeyDown if suppressed && !held_set.contains(&ev.code) => {
                            // Fresh entry into held.
                            *entered.entry(ev.code).or_default() += 1;
                            held_set.insert(ev.code);
                        }
                        EventKind::KeyUp if suppressed => {
                            *left.entry(ev.code).or_default() += 1;
                            held_set.remove(&ev.code);
                        }
                        _ => {}
                    }
                }
            }
        }

        for code in entered.keys().chain(left.keys()).copied().collect::<HashSet<_>>() {
            let e = entered.get(&code).copied().unwrap_or(0);
            let l = left.get(&code).copied().unwrap_or(0);
            let still_held = held_set.contains(&code);
            let expected_diff = if still_held { 1 } else { 0 };
            prop_assert_eq!(e - l, expected_diff, "code={:#x}", code);
        }
    }

    /// `Tracker::on_event` on a pure-key trigger fires exactly once per
    /// "fresh down" — i.e. KeyDown of the trigger code with no preceding
    /// un-released KeyDown of the same code. Auto-repeats are debounced.
    /// Other codes never fire.
    #[test]
    fn tracker_pure_key_matches_reference(
        trigger in keycode(),
        events in proptest::collection::vec(key_event(), 0..64),
    ) {
        let mut t = Tracker::new(pure_key_combo(trigger));
        let base = Instant::now();
        let mut model_pressed = false;

        for (i, ev) in events.iter().enumerate() {
            let now = base + Duration::from_millis((i as u64) * 10);
            let actual = t.on_event(*ev, now);
            let expected = if ev.code != trigger {
                None
            } else {
                match ev.kind {
                    EventKind::KeyDown if !model_pressed => {
                        model_pressed = true;
                        Some(HotkeyEvent::TriggerRecord)
                    }
                    EventKind::KeyDown => None, // auto-repeat
                    EventKind::KeyUp => {
                        model_pressed = false;
                        None
                    }
                    EventKind::FlagsChanged => None,
                }
            };
            prop_assert_eq!(actual, expected, "i={} ev={:?}", i, ev);
        }
    }

    /// Pure-key `:double`: a `TriggerRecord` is emitted iff the second
    /// `KeyDown` of the trigger within the double-tap window has a
    /// preceding `KeyDown` (also of the trigger) at a fresh cycle and
    /// within window. We verify the weaker invariant: the emit count
    /// equals floor(N / 2) for N "fresh keydowns" all clustered within
    /// the window.
    #[test]
    fn tracker_double_tap_fires_on_even_taps(
        trigger in keycode(),
        n_taps in 0u32..8,
    ) {
        let mut combo = pure_key_combo(trigger);
        combo.double = true;
        let mut t = Tracker::new(combo);
        let base = Instant::now();
        let mut emits = 0u32;

        for i in 0..n_taps {
            // All within window: 50ms spacing → far below 400ms default.
            let now = base + Duration::from_millis(i as u64 * 50);
            let r = t.on_event(
                RawEvent { kind: EventKind::KeyDown, code: trigger, mods: ModMask::empty() },
                now,
            );
            if matches!(r, Some(HotkeyEvent::TriggerRecord)) {
                emits += 1;
            }
            t.on_event(
                RawEvent { kind: EventKind::KeyUp, code: trigger, mods: ModMask::empty() },
                now + Duration::from_millis(1),
            );
        }

        prop_assert_eq!(emits, n_taps / 2);
    }
}
