use std::time::Instant;

use anyhow::{Context, Result};

use super::{parse, Combo, HotkeyEvent, RawEvent, Tracker};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum HotkeyAction {
    Toggle,
    Cancel,
    Resume,
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
    pub(crate) fn parse(trigger: &str, cancel: &str, resume: &str) -> Result<Self> {
        let trigger = parse::parse(trigger)
            .with_context(|| format!("parse [hotkey] trigger = {trigger:?}"))?;
        let cancel =
            parse::parse(cancel).with_context(|| format!("parse [hotkey] cancel = {cancel:?}"))?;
        let resume =
            parse::parse(resume).with_context(|| format!("parse [hotkey] resume = {resume:?}"))?;
        anyhow::ensure!(
            trigger != cancel,
            "[hotkey] trigger and cancel must be different"
        );
        anyhow::ensure!(
            trigger != resume,
            "[hotkey] trigger and resume must be different"
        );
        anyhow::ensure!(
            cancel != resume,
            "[hotkey] cancel and resume must be different"
        );
        Ok(Self {
            entries: vec![
                Binding {
                    action: HotkeyAction::Toggle,
                    combo: trigger,
                },
                Binding {
                    action: HotkeyAction::Cancel,
                    combo: cancel,
                },
                Binding {
                    action: HotkeyAction::Resume,
                    combo: resume,
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
    resume_tracker: Tracker,
}

impl TrackerSet {
    pub(crate) fn new(bindings: &Bindings) -> Self {
        Self {
            trigger_tracker: Tracker::new(
                bindings
                    .combo_for(HotkeyAction::Toggle)
                    .expect("missing toggle-record hotkey binding")
                    .clone(),
            ),
            cancel_tracker: Tracker::new(
                bindings
                    .combo_for(HotkeyAction::Cancel)
                    .expect("missing cancel-record hotkey binding")
                    .clone(),
            ),
            resume_tracker: Tracker::new(
                bindings
                    .combo_for(HotkeyAction::Resume)
                    .expect("missing resume-record hotkey binding")
                    .clone(),
            ),
        }
    }

    pub(crate) fn set_bindings(&mut self, bindings: &Bindings) {
        *self = Self::new(bindings);
    }

    pub(crate) fn on_event(&mut self, ev: RawEvent, now: Instant) -> Option<HotkeyAction> {
        let cancel = self.cancel_tracker.on_event(ev, now);
        let resume = self.resume_tracker.on_event(ev, now);
        let trigger = self.trigger_tracker.on_event(ev, now);

        if matches!(cancel, Some(HotkeyEvent::TriggerRecord)) {
            return Some(HotkeyAction::Cancel);
        }
        if matches!(resume, Some(HotkeyEvent::TriggerRecord)) {
            return Some(HotkeyAction::Resume);
        }
        matches!(trigger, Some(HotkeyEvent::TriggerRecord)).then_some(HotkeyAction::Toggle)
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
        let bindings = Bindings::parse("f16", "escape:double", "shift+f17").unwrap();

        assert_eq!(bindings.entries().len(), 3);
        assert_eq!(bindings.entries()[0].action, HotkeyAction::Toggle);
        assert_eq!(bindings.entries()[0].combo.to_string(), "f16");
        assert_eq!(bindings.entries()[1].action, HotkeyAction::Cancel);
        assert_eq!(bindings.entries()[1].combo.to_string(), "escape:double");
        assert_eq!(bindings.entries()[2].action, HotkeyAction::Resume);
        assert_eq!(bindings.entries()[2].combo.to_string(), "shift+f17");

        let err = Bindings::parse("escape", "escape", "shift+f17").unwrap_err();
        assert!(err.to_string().contains("must be different"));
    }

    #[test]
    fn tracker_set_emits_bound_action() {
        let bindings = Bindings::parse("f16", "escape", "shift+f17").unwrap();
        let mut trackers = TrackerSet::new(&bindings);

        let action = trackers.on_event(
            RawEvent {
                kind: EventKind::KeyDown,
                key: Key::F(16),
                mods: ModMask::empty(),
            },
            Instant::now(),
        );

        assert_eq!(action, Some(HotkeyAction::Toggle));
    }

    #[test]
    fn trigger_event_disqualifies_modifier_only_cancel() {
        let bindings = Bindings::parse("cmd+r", "cmd", "shift+f17").unwrap();
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
            Some(HotkeyAction::Toggle)
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
        let bindings = Bindings::parse("left_cmd", "cmd", "shift+f17").unwrap();
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
            Some(HotkeyAction::Cancel)
        );
    }

    #[test]
    fn bindings_parse_three_record_actions_and_reject_duplicate_combos() {
        let bindings = Bindings::parse("f16", "escape:double", "shift+f17").unwrap();

        assert_eq!(bindings.entries().len(), 3);
        assert_eq!(bindings.entries()[0].action, HotkeyAction::Toggle);
        assert_eq!(bindings.entries()[0].combo.to_string(), "f16");
        assert_eq!(bindings.entries()[1].action, HotkeyAction::Cancel);
        assert_eq!(bindings.entries()[1].combo.to_string(), "escape:double");
        assert_eq!(bindings.entries()[2].action, HotkeyAction::Resume);
        assert_eq!(bindings.entries()[2].combo.to_string(), "shift+f17");

        let err = Bindings::parse("escape", "f16", "escape").unwrap_err();
        assert!(err.to_string().contains("trigger and resume"));
        let err = Bindings::parse("f16", "escape", "escape").unwrap_err();
        assert!(err.to_string().contains("cancel and resume"));
    }

    #[test]
    fn tracker_set_emits_resume_action() {
        let bindings = Bindings::parse("f16", "escape", "shift+f17").unwrap();
        let mut trackers = TrackerSet::new(&bindings);

        let mut mods = ModMask::empty();
        mods.set(ModType::Shift, Side::Left, true);
        let action = trackers.on_event(
            RawEvent {
                kind: EventKind::KeyDown,
                key: Key::F(17),
                mods,
            },
            Instant::now(),
        );

        assert_eq!(action, Some(HotkeyAction::Resume));
    }

    #[test]
    fn action_priority_is_cancel_then_resume_then_toggle() {
        let bindings = Bindings::parse("left_cmd", "cmd", "left_cmd").unwrap_err();
        assert!(bindings.to_string().contains("trigger and resume"));

        let bindings = Bindings {
            entries: vec![
                Binding {
                    action: HotkeyAction::Toggle,
                    combo: parse::parse("left_cmd").unwrap(),
                },
                Binding {
                    action: HotkeyAction::Cancel,
                    combo: parse::parse("cmd").unwrap(),
                },
                Binding {
                    action: HotkeyAction::Resume,
                    combo: parse::parse("left_cmd").unwrap(),
                },
            ],
        };
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
            Some(HotkeyAction::Cancel)
        );
    }

    #[test]
    fn resume_wins_over_toggle_when_both_trackers_match_the_same_event() {
        let bindings = Bindings {
            entries: vec![
                Binding {
                    action: HotkeyAction::Toggle,
                    combo: parse::parse("left_cmd").unwrap(),
                },
                Binding {
                    action: HotkeyAction::Cancel,
                    combo: parse::parse("f16").unwrap(),
                },
                Binding {
                    action: HotkeyAction::Resume,
                    combo: parse::parse("left_cmd").unwrap(),
                },
            ],
        };
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
            Some(HotkeyAction::Resume)
        );
    }

    #[test]
    fn lower_priority_trackers_still_consume_winning_event() {
        let bindings = Bindings {
            entries: vec![
                Binding {
                    action: HotkeyAction::Toggle,
                    combo: parse::parse("f16").unwrap(),
                },
                Binding {
                    action: HotkeyAction::Cancel,
                    combo: parse::parse("cmd").unwrap(),
                },
                Binding {
                    action: HotkeyAction::Resume,
                    combo: parse::parse("left_cmd").unwrap(),
                },
            ],
        };
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
                now + std::time::Duration::from_millis(80),
            ),
            Some(HotkeyAction::Cancel)
        );
        assert_eq!(
            trackers.on_event(
                RawEvent {
                    kind: EventKind::FlagsChanged,
                    key: Key::Modifier(ModType::Cmd, Side::Left),
                    mods: ModMask::empty(),
                },
                now + std::time::Duration::from_millis(160),
            ),
            None
        );
    }

    #[test]
    fn default_resume_modifier_double_tap_emits_resume() {
        let bindings = Bindings::parse("f16", "escape", "shift+right_option:double").unwrap();
        let mut trackers = TrackerSet::new(&bindings);
        let now = Instant::now();
        let r_opt = Key::Modifier(ModType::Opt, Side::Right);
        let l_shift = Key::Modifier(ModType::Shift, Side::Left);
        let mut both = ModMask::empty();
        both.set(ModType::Opt, Side::Right, true);
        both.set(ModType::Shift, Side::Left, true);
        let mut shift_only = ModMask::empty();
        shift_only.set(ModType::Shift, Side::Left, true);

        assert_eq!(
            trackers.on_event(
                RawEvent {
                    kind: EventKind::FlagsChanged,
                    key: l_shift,
                    mods: shift_only,
                },
                now,
            ),
            None
        );
        assert_eq!(
            trackers.on_event(
                RawEvent {
                    kind: EventKind::FlagsChanged,
                    key: r_opt,
                    mods: both,
                },
                now + std::time::Duration::from_millis(20),
            ),
            None
        );
        assert_eq!(
            trackers.on_event(
                RawEvent {
                    kind: EventKind::FlagsChanged,
                    key: r_opt,
                    mods: shift_only,
                },
                now + std::time::Duration::from_millis(80),
            ),
            None
        );
        assert_eq!(
            trackers.on_event(
                RawEvent {
                    kind: EventKind::FlagsChanged,
                    key: r_opt,
                    mods: both,
                },
                now + std::time::Duration::from_millis(180),
            ),
            None
        );
        assert_eq!(
            trackers.on_event(
                RawEvent {
                    kind: EventKind::FlagsChanged,
                    key: r_opt,
                    mods: shift_only,
                },
                now + std::time::Duration::from_millis(240),
            ),
            Some(HotkeyAction::Resume)
        );
    }
}
