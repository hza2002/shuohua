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

    pub(crate) fn entries(&self) -> &[Binding] {
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
    trackers: Vec<(HotkeyAction, Tracker)>,
}

impl TrackerSet {
    pub(crate) fn new(bindings: &Bindings) -> Self {
        Self {
            trackers: bindings
                .entries()
                .iter()
                .map(|binding| (binding.action, Tracker::new(binding.combo.clone())))
                .collect(),
        }
    }

    pub(crate) fn set_bindings(&mut self, bindings: &Bindings) {
        *self = Self::new(bindings);
    }

    pub(crate) fn on_event(&mut self, ev: RawEvent, now: Instant) -> Option<HotkeyAction> {
        self.trackers.iter_mut().find_map(|(action, tracker)| {
            matches!(tracker.on_event(ev, now), Some(HotkeyEvent::TriggerRecord)).then_some(*action)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::hotkey::{EventKind, Key, ModMask, RawEvent};

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
}
