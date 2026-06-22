//! Hotkey trigger types: `Combo` + modifier matching.
//!
//! A `Combo` is a static description of "what counts as the trigger". It is
//! produced by [`crate::hotkey::parse::parse`] from a config string and
//! consumed by `Tracker` (decide when to emit `TriggerRecord`) and
//! `Suppressor` (decide what to drop for the foreground app).
//!
//! Three shapes are supported (all may carry the `:double` suffix):
//!
//! - **Pure key**: e.g. `f12`, `a`, `space`. `mods` is all `NotPresent`,
//!   `key` is `Some(keycode)`.
//! - **Modifier + key**: e.g. `cmd+r`, `left_cmd+shift+r`. `mods` has at
//!   least one non-`NotPresent` matcher, `key` is `Some(keycode)`.
//! - **Modifier-only**: e.g. `right_shift`, `cmd+shift`. `mods` has at least
//!   one non-`NotPresent` matcher, `key` is `None`.
//!
//! Matching is "exact": modifiers not mentioned in the combo must be UP.
//! So `cmd+r` does *not* fire when the user presses `cmd+shift+r` — that's
//! a different combo that might be bound elsewhere.

use std::fmt;

use super::key::Key;

/// macOS virtual keycode (HIToolbox/Events.h). Side-agnostic — modifiers'
/// L/R distinction lives in [`Side`] / [`ModMatcher`], not here.
/// The four modifier classes we recognize. Stored as an index 0..4 so they
/// can address into `Combo::mods` and `ModMask` bit pairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModType {
    Cmd = 0,
    Ctrl = 1,
    Opt = 2,
    Shift = 3,
}

impl ModType {
    pub const ALL: [ModType; 4] = [Self::Cmd, Self::Ctrl, Self::Opt, Self::Shift];

    pub fn name(self) -> &'static str {
        match self {
            Self::Cmd => "cmd",
            Self::Ctrl => "ctrl",
            Self::Opt => "opt",
            Self::Shift => "shift",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// Per-modifier requirement inside a [`Combo`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ModMatcher {
    /// The modifier must be entirely UP (neither side pressed).
    #[default]
    NotPresent,
    /// The modifier must be DOWN on either side.
    EitherSide,
    /// The modifier must be DOWN on the specified side. The opposite side
    /// is unconstrained (may be up or down).
    Specific(Side),
}

/// Static trigger description. See module docs for the three shapes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combo {
    /// Required state of each modifier, indexed by `ModType as usize`.
    pub mods: [ModMatcher; 4],
    /// `Some(keycode)` for key-bearing combos, `None` for modifier-only.
    pub key: Option<Key>,
    /// `:double` suffix — fires only on the second tap within
    /// [`crate::hotkey::tracker::DOUBLE_TAP_WINDOW`].
    pub double: bool,
}

impl Combo {
    pub fn matcher(&self, ty: ModType) -> ModMatcher {
        self.mods[ty as usize]
    }

    #[cfg(test)]
    pub fn is_modifier_only(&self) -> bool {
        self.key.is_none()
    }

    /// True iff at least one modifier matcher is not `NotPresent`.
    pub fn has_modifier_requirement(&self) -> bool {
        self.mods
            .iter()
            .any(|m| !matches!(m, ModMatcher::NotPresent))
    }
}

/// Snapshot of which modifier keys are currently DOWN, packed as bits.
///
/// Layout: pair per `ModType`, low bit = left, high bit = right.
/// `Cmd` occupies bits 0/1, `Ctrl` 2/3, `Opt` 4/5, `Shift` 6/7.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModMask(pub u8);

impl ModMask {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub fn is_side_down(self, ty: ModType, side: Side) -> bool {
        let shift = (ty as u8) * 2 + side_shift(side);
        self.0 & (1 << shift) != 0
    }

    pub fn is_any_side_down(self, ty: ModType) -> bool {
        let lo = (ty as u8) * 2;
        let pair_mask = 0b11 << lo;
        self.0 & pair_mask != 0
    }

    pub fn set(&mut self, ty: ModType, side: Side, down: bool) {
        let shift = (ty as u8) * 2 + side_shift(side);
        let bit = 1u8 << shift;
        if down {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }

    /// Check whether this snapshot exactly matches a combo's modifier
    /// requirements. The combo's key (if any) is *not* considered here —
    /// that's the caller's job.
    pub fn matches_combo(self, combo: &Combo) -> bool {
        for ty in ModType::ALL {
            let down_any = self.is_any_side_down(ty);
            match combo.matcher(ty) {
                ModMatcher::NotPresent => {
                    if down_any {
                        return false;
                    }
                }
                ModMatcher::EitherSide => {
                    if !down_any {
                        return false;
                    }
                }
                ModMatcher::Specific(side) => {
                    if !self.is_side_down(ty, side) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

const fn side_shift(side: Side) -> u8 {
    match side {
        Side::Left => 0,
        Side::Right => 1,
    }
}

impl fmt::Display for Combo {
    /// Canonical form. Stable enough that the TUI key-capture path can
    /// round-trip through `parse(c.to_string()) == c`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        let mut write_token = |s: &str, f: &mut fmt::Formatter<'_>| -> fmt::Result {
            if !first {
                f.write_str("+")?;
            }
            first = false;
            f.write_str(s)
        };
        for ty in ModType::ALL {
            match self.matcher(ty) {
                ModMatcher::NotPresent => {}
                ModMatcher::EitherSide => write_token(ty.name(), f)?,
                ModMatcher::Specific(side) => {
                    let token = format!("{}_{}", side.name(), ty.name());
                    write_token(&token, f)?;
                }
            }
        }
        if let Some(key) = self.key {
            let name = key.name().unwrap_or("?");
            write_token(name, f)?;
        }
        if self.double {
            f.write_str(":double")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo_pure_key(key: Key) -> Combo {
        Combo {
            mods: [ModMatcher::NotPresent; 4],
            key: Some(key),
            double: false,
        }
    }

    #[test]
    fn modmask_round_trip() {
        let mut m = ModMask::empty();
        m.set(ModType::Cmd, Side::Left, true);
        assert!(m.is_side_down(ModType::Cmd, Side::Left));
        assert!(!m.is_side_down(ModType::Cmd, Side::Right));
        assert!(m.is_any_side_down(ModType::Cmd));
        m.set(ModType::Cmd, Side::Left, false);
        assert!(!m.is_any_side_down(ModType::Cmd));
    }

    #[test]
    fn matches_pure_key_requires_no_mods() {
        let c = combo_pure_key(Key::F(16));
        assert!(ModMask::empty().matches_combo(&c));

        let mut m = ModMask::empty();
        m.set(ModType::Cmd, Side::Left, true);
        assert!(
            !m.matches_combo(&c),
            "any modifier presence must fail pure-key"
        );
    }

    #[test]
    fn matches_either_side() {
        let c = Combo {
            mods: [
                ModMatcher::EitherSide,
                ModMatcher::NotPresent,
                ModMatcher::NotPresent,
                ModMatcher::NotPresent,
            ],
            key: Some(Key::Char('r')),
            double: false,
        };
        let mut m = ModMask::empty();
        m.set(ModType::Cmd, Side::Left, true);
        assert!(m.matches_combo(&c));
        m.set(ModType::Cmd, Side::Left, false);
        m.set(ModType::Cmd, Side::Right, true);
        assert!(m.matches_combo(&c));
    }

    #[test]
    fn matches_specific_side() {
        let c = Combo {
            mods: [
                ModMatcher::Specific(Side::Right),
                ModMatcher::NotPresent,
                ModMatcher::NotPresent,
                ModMatcher::NotPresent,
            ],
            key: Some(Key::Char('r')),
            double: false,
        };
        let mut m = ModMask::empty();
        m.set(ModType::Cmd, Side::Left, true);
        assert!(
            !m.matches_combo(&c),
            "left cmd alone must not satisfy right cmd"
        );
        m.set(ModType::Cmd, Side::Right, true);
        assert!(m.matches_combo(&c));
    }

    #[test]
    fn matches_rejects_extra_modifier() {
        // `cmd+r`-style combo: cmd required, others NotPresent.
        let c = Combo {
            mods: [
                ModMatcher::EitherSide,
                ModMatcher::NotPresent,
                ModMatcher::NotPresent,
                ModMatcher::NotPresent,
            ],
            key: Some(Key::Char('r')),
            double: false,
        };
        let mut m = ModMask::empty();
        m.set(ModType::Cmd, Side::Left, true);
        m.set(ModType::Shift, Side::Left, true);
        assert!(!m.matches_combo(&c), "extra mod must break exact match");
    }
}
