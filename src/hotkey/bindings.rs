use std::time::Instant;

use anyhow::{Context, Result};

use super::{parse, Combo, HotkeyEvent, RawEvent, Tracker};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum HotkeyAction {
    ToggleRecord,
    CancelRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub(crate) action: HotkeyAction,
    pub(crate) combo: Combo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Bindings {
    entries: Vec<Binding>,
}

impl Bindings {
    pub(crate) fn parse(trigger: &str, cancel: &str) -> Result<Self> {
        let trigger = parse::parse(trigger)
            .with_context(|| format!("parse [hotkey] trigger = {trigger:?}"))?;
        let cancel =
            parse::parse(cancel).with_context(|| format!("parse [hotkey] cancel = {cancel:?}"))?;
        anyhow::ensure!(
            trigger != cancel,
            "[hotkey] trigger and cancel must be different"
        );
        Ok(Self {
            entries: vec![
                Binding {
                    action: HotkeyAction::ToggleRecord,
                    combo: trigger,
                },
                Binding {
                    action: HotkeyAction::CancelRecord,
                    combo: cancel,
                },
            ],
        })
    }

    #[cfg(test)]
    fn entries(&self) -> &[Binding] {
        &self.entries
    }

    pub(crate) fn combo_for(&self, action: HotkeyAction) -> Option<&Combo> {
        self.entries
            .iter()
            .find(|binding| binding.action == action)
            .map(|binding| &binding.combo)
    }
}

#[derive(Debug)]
pub(crate) struct TrackerSet {
    trigger_tracker: Tracker,
    cancel_tracker: Tracker,
}

impl TrackerSet {
    pub(crate) fn new(bindings: &Bindings) -> Self {
        Self {
            trigger_tracker: Tracker::new(
                bindings
                    .combo_for(HotkeyAction::ToggleRecord)
                    .expect("missing toggle-record hotkey binding")
                    .clone(),
            ),
            cancel_tracker: Tracker::new(
                bindings
                    .combo_for(HotkeyAction::CancelRecord)
                    .expect("missing cancel-record hotkey binding")
                    .clone(),
            ),
        }
    }

    pub(crate) fn set_bindings(&mut self, bindings: &Bindings) {
        *self = Self::new(bindings);
    }

    pub(crate) fn on_event(&mut self, ev: RawEvent, now: Instant) -> Option<HotkeyAction> {
        if matches!(
            self.cancel_tracker.on_event(ev, now),
            Some(HotkeyEvent::TriggerRecord)
        ) {
            return Some(HotkeyAction::CancelRecord);
        }
        matches!(
            self.trigger_tracker.on_event(ev, now),
            Some(HotkeyEvent::TriggerRecord)
        )
        .then_some(HotkeyAction::ToggleRecord)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::hotkey::combo::{ModType, Side};
    use crate::hotkey::{EventKind, Key, ModMask, RawEvent};

    fn left_cmd_mods() -> ModMask {
        let mut mods = ModMask::empty();
        mods.set(ModType::Cmd, Side::Left, true);
        mods
    }

    #[test]
    fn bindings_parse_record_actions_and_reject_duplicate_combos() {
        let bindings = Bindings::parse("f16", "escape:double").unwrap();

        assert_eq!(bindings.entries().len(), 2);
        assert_eq!(bindings.entries()[0].action, HotkeyAction::ToggleRecord);
        assert_eq!(bindings.entries()[0].combo.to_string(), "f16");
        assert_eq!(bindings.entries()[1].action, HotkeyAction::CancelRecord);
        assert_eq!(bindings.entries()[1].combo.to_string(), "escape:double");

        let err = Bindings::parse("escape", "escape").unwrap_err();
        assert!(err.to_string().contains("must be different"));
    }

    #[test]
    fn tracker_set_emits_bound_action() {
        let bindings = Bindings::parse("f16", "escape").unwrap();
        let mut trackers = TrackerSet::new(&bindings);

        let action = trackers.on_event(
            RawEvent {
                kind: EventKind::KeyDown,
                key: Key::F(16),
                mods: ModMask::empty(),
            },
            Instant::now(),
        );

        assert_eq!(action, Some(HotkeyAction::ToggleRecord));
    }

    #[test]
    fn trigger_event_disqualifies_modifier_only_cancel() {
        let bindings = Bindings::parse("cmd+r", "cmd").unwrap();
        let mut trackers = TrackerSet::new(&bindings);
        let now = Instant::now();

        assert_eq!(
            trackers.on_event(
                RawEvent {
                    kind: EventKind::FlagsChanged,
                    key: Key::Modifier(ModType::Cmd, Side::Left),
                    mods: left_cmd_mods(),
                },
                now,
            ),
            None
        );
        assert_eq!(
            trackers.on_event(
                RawEvent {
                    kind: EventKind::KeyDown,
                    key: Key::Char('r'),
                    mods: left_cmd_mods(),
                },
                now,
            ),
            Some(HotkeyAction::ToggleRecord)
        );
        assert_eq!(
            trackers.on_event(
                RawEvent {
                    kind: EventKind::FlagsChanged,
                    key: Key::Modifier(ModType::Cmd, Side::Left),
                    mods: ModMask::empty(),
                },
                now,
            ),
            None
        );
    }

    #[test]
    fn cancel_wins_when_both_trackers_match_the_same_event() {
        let bindings = Bindings::parse("left_cmd", "cmd").unwrap();
        let mut trackers = TrackerSet::new(&bindings);
        let now = Instant::now();

        assert_eq!(
            trackers.on_event(
                RawEvent {
                    kind: EventKind::FlagsChanged,
                    key: Key::Modifier(ModType::Cmd, Side::Left),
                    mods: left_cmd_mods(),
                },
                now,
            ),
            None
        );
        assert_eq!(
            trackers.on_event(
                RawEvent {
                    kind: EventKind::FlagsChanged,
                    key: Key::Modifier(ModType::Cmd, Side::Left),
                    mods: ModMask::empty(),
                },
                now,
            ),
            Some(HotkeyAction::CancelRecord)
        );
    }
}
