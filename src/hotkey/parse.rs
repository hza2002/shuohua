//! Hotkey string → [`Combo`].
//!
//! Grammar:
//!
//! ```text
//! trigger     := combo (":double")?
//! combo       := token ("+" token)*
//! token       := modifier | key
//! modifier    := ("left_" | "right_")? ("cmd" | "ctrl" | "opt" | "shift")
//! key         := f1..f20 | a..z | 0..9
//!             |  space | tab | escape | return | delete | backspace
//!             |  up | down | left | right
//!             |  ";" | "," | "." | "/" | "\\" | "[" | "]" | "'" | "`" | "-" | "="
//! ```
//!
//! Rules:
//!
//! - Case-insensitive (normalized to lowercase).
//! - In a multi-token combo, all tokens before the last must be modifiers;
//!   the last token may be a modifier (= modifier-only trigger) or a key
//!   (= combo trigger).
//! - Modifiers without a side prefix match either side; with `left_` /
//!   `right_` they match that side only (the opposite side is unconstrained).
//! - Modifiers may not be duplicated within the same `ModType` (so
//!   `cmd+cmd` is rejected, but `cmd+left_cmd` is allowed — the latter
//!   degenerates to "left cmd specifically").
//! - At most one `:double` suffix.
//! - Empty / trailing `+` / leading `+` / unknown tokens → error.

use anyhow::{anyhow, bail, Context, Result};

use super::combo::{Combo, KeyCode, ModMatcher, ModType, Side};

pub fn parse(s: &str) -> Result<Combo> {
    let s = s.trim();
    if s.is_empty() {
        bail!("empty hotkey string");
    }
    let s = s.to_ascii_lowercase();

    // Strip optional ":double" suffix. ":double:double" or ":doubledouble"
    // both rejected (no further colon allowed; suffix must be exactly the
    // literal).
    let (body, double) = match s.rsplit_once(':') {
        Some((body, "double")) => (body.to_string(), true),
        Some((_, suffix)) => bail!("unknown hotkey suffix {suffix:?}; only :double is recognized"),
        None => (s, false),
    };
    if body.contains(':') {
        bail!("multiple ':' in hotkey; only one trailing ':double' allowed");
    }
    if body.is_empty() {
        bail!("hotkey has ':double' but no combo before it");
    }

    let tokens: Vec<&str> = body.split('+').collect();
    if tokens.iter().any(|t| t.is_empty()) {
        bail!("empty token in hotkey {body:?} (check for leading/trailing/double '+')");
    }

    // Classify each token. Modifiers go into the matcher array; the last
    // non-modifier (if any) is the key.
    let mut mods: [ModMatcher; 4] = [ModMatcher::NotPresent; 4];
    let mut key: Option<KeyCode> = None;

    for (i, token) in tokens.iter().enumerate() {
        if let Some((ty, side_req)) = parse_modifier_token(token) {
            // Modifier token must not appear after a key.
            if key.is_some() {
                bail!(
                    "modifier {token:?} appears after a key; modifiers must precede the key \
                     (got hotkey {body:?})"
                );
            }
            merge_mod_matcher(&mut mods, ty, side_req).with_context(|| {
                format!("duplicate / contradictory modifier {token:?} in hotkey {body:?}")
            })?;
        } else if let Some(code) = name_to_keycode(token) {
            // Key token must be the last token.
            if i != tokens.len() - 1 {
                bail!("key {token:?} must be the last token in hotkey {body:?}");
            }
            key = Some(code);
        } else {
            bail!("unknown hotkey token {token:?} in {body:?}; see docs/DESIGN.md for grammar");
        }
    }

    let combo = Combo { mods, key, double };
    if !combo.has_modifier_requirement() && combo.key.is_none() {
        // Should be unreachable given the parsing above, but defense in depth.
        bail!("hotkey {body:?} resolved to nothing — must have at least a key or modifier");
    }
    Ok(combo)
}

/// `parse(combo.to_string()) == combo` for any combo we can construct.
/// Useful so `Display` impl on `Combo` round-trips through this module.
pub fn keycode_to_name(code: KeyCode) -> Option<&'static str> {
    LEXICON
        .iter()
        .find(|(_, c)| *c == code)
        .map(|(name, _)| *name)
}

/// macOS modifier-key keycodes (FlagsChanged events). Used by the
/// CGEventTap callback to maintain a `ModMask` snapshot, and by the
/// Suppressor to decide whether a key event is for a modifier.
pub fn modifier_from_keycode(code: KeyCode) -> Option<(ModType, Side)> {
    match code {
        0x37 => Some((ModType::Cmd, Side::Left)),
        0x36 => Some((ModType::Cmd, Side::Right)),
        0x38 => Some((ModType::Shift, Side::Left)),
        0x3C => Some((ModType::Shift, Side::Right)),
        0x3A => Some((ModType::Opt, Side::Left)),
        0x3D => Some((ModType::Opt, Side::Right)),
        0x3B => Some((ModType::Ctrl, Side::Left)),
        0x3E => Some((ModType::Ctrl, Side::Right)),
        _ => None,
    }
}

// --- modifier token parsing ---

enum SideRequirement {
    Either,
    Specific(Side),
}

fn parse_modifier_token(token: &str) -> Option<(ModType, SideRequirement)> {
    let (side_req, name) = if let Some(rest) = token.strip_prefix("left_") {
        (SideRequirement::Specific(Side::Left), rest)
    } else if let Some(rest) = token.strip_prefix("right_") {
        (SideRequirement::Specific(Side::Right), rest)
    } else {
        (SideRequirement::Either, token)
    };
    // Canonical names are 3-letter (cmd/ctrl/opt/shift) — used by Display
    // for round-trip. Aliases match the words users see on physical macOS
    // keyboards / cross-platform docs.
    let ty = match name {
        "cmd" | "command" => ModType::Cmd,
        "ctrl" | "control" => ModType::Ctrl,
        "opt" | "alt" | "option" => ModType::Opt,
        "shift" => ModType::Shift,
        _ => return None,
    };
    Some((ty, side_req))
}

fn merge_mod_matcher(
    mods: &mut [ModMatcher; 4],
    ty: ModType,
    new_side: SideRequirement,
) -> Result<()> {
    let slot = &mut mods[ty as usize];
    let new_matcher = match new_side {
        SideRequirement::Either => ModMatcher::EitherSide,
        SideRequirement::Specific(side) => ModMatcher::Specific(side),
    };
    *slot = match (*slot, new_matcher) {
        (ModMatcher::NotPresent, m) => m,
        // `cmd+cmd` or any genuine duplicate is rejected.
        (a, b) if a == b => return Err(anyhow!("duplicate modifier")),
        // `cmd+left_cmd` → degenerate to the stricter side-specific form.
        (ModMatcher::EitherSide, ModMatcher::Specific(side))
        | (ModMatcher::Specific(side), ModMatcher::EitherSide) => ModMatcher::Specific(side),
        // `left_cmd+right_cmd` would need a BothSides matcher. Reject it until
        // there is a real use case for requiring both sides of one modifier.
        (ModMatcher::Specific(a), ModMatcher::Specific(b)) if a != b => {
            return Err(anyhow!(
                "specifying both left_ and right_ for the same modifier is not supported"
            ));
        }
        // Any other shape is a genuine contradiction.
        _ => return Err(anyhow!("contradictory matcher")),
    };
    Ok(())
}

// --- lexicon ---

/// (name, macOS HIToolbox virtual keycode). Side-agnostic — modifier keys
/// are intentionally absent here; see [`modifier_from_keycode`].
const LEXICON: &[(&str, KeyCode)] = &[
    // Function keys F1–F20.
    ("f1", 0x7A),
    ("f2", 0x78),
    ("f3", 0x63),
    ("f4", 0x76),
    ("f5", 0x60),
    ("f6", 0x61),
    ("f7", 0x62),
    ("f8", 0x64),
    ("f9", 0x65),
    ("f10", 0x6D),
    ("f11", 0x67),
    ("f12", 0x6F),
    ("f13", 0x69),
    ("f14", 0x6B),
    ("f15", 0x71),
    ("f16", 0x6A),
    ("f17", 0x40),
    ("f18", 0x4F),
    ("f19", 0x50),
    ("f20", 0x5A),
    // Letters a–z.
    ("a", 0x00),
    ("b", 0x0B),
    ("c", 0x08),
    ("d", 0x02),
    ("e", 0x0E),
    ("f", 0x03),
    ("g", 0x05),
    ("h", 0x04),
    ("i", 0x22),
    ("j", 0x26),
    ("k", 0x28),
    ("l", 0x25),
    ("m", 0x2E),
    ("n", 0x2D),
    ("o", 0x1F),
    ("p", 0x23),
    ("q", 0x0C),
    ("r", 0x0F),
    ("s", 0x01),
    ("t", 0x11),
    ("u", 0x20),
    ("v", 0x09),
    ("w", 0x0D),
    ("x", 0x07),
    ("y", 0x10),
    ("z", 0x06),
    // Digits 0–9 (top row).
    ("0", 0x1D),
    ("1", 0x12),
    ("2", 0x13),
    ("3", 0x14),
    ("4", 0x15),
    ("5", 0x17),
    ("6", 0x16),
    ("7", 0x1A),
    ("8", 0x1C),
    ("9", 0x19),
    // Whitespace / control.
    ("space", 0x31),
    ("tab", 0x30),
    ("return", 0x24),
    ("escape", 0x35),
    ("esc", 0x35),
    ("backspace", 0x33),
    ("delete", 0x75),
    // Arrows.
    ("up", 0x7E),
    ("down", 0x7D),
    ("left", 0x7B),
    ("right", 0x7C),
    // Punctuation.
    (";", 0x29),
    (",", 0x2B),
    (".", 0x2F),
    ("/", 0x2C),
    ("\\", 0x2A),
    ("[", 0x21),
    ("]", 0x1E),
    ("'", 0x27),
    ("`", 0x32),
    ("-", 0x1B),
    ("=", 0x18),
];

fn name_to_keycode(name: &str) -> Option<KeyCode> {
    LEXICON
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, code)| *code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eitherside() -> ModMatcher {
        ModMatcher::EitherSide
    }
    fn notpresent() -> ModMatcher {
        ModMatcher::NotPresent
    }
    fn left() -> ModMatcher {
        ModMatcher::Specific(Side::Left)
    }
    fn right() -> ModMatcher {
        ModMatcher::Specific(Side::Right)
    }

    #[test]
    fn pure_function_key() {
        let c = parse("f16").unwrap();
        assert_eq!(c.mods, [notpresent(); 4]);
        assert_eq!(c.key, Some(0x6A));
        assert!(!c.double);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(parse("F16").unwrap(), parse("f16").unwrap());
        assert_eq!(parse("Cmd+SHIFT+r").unwrap(), parse("cmd+shift+r").unwrap());
    }

    #[test]
    fn trimming() {
        assert_eq!(parse("  f16  ").unwrap(), parse("f16").unwrap());
    }

    #[test]
    fn combo_modifier_plus_key() {
        let c = parse("cmd+r").unwrap();
        assert_eq!(c.mods[ModType::Cmd as usize], eitherside());
        assert_eq!(c.mods[ModType::Ctrl as usize], notpresent());
        assert_eq!(c.key, Some(0x0F));
    }

    #[test]
    fn combo_multiple_modifiers() {
        let c = parse("left_cmd+shift+r").unwrap();
        assert_eq!(c.mods[ModType::Cmd as usize], left());
        assert_eq!(c.mods[ModType::Shift as usize], eitherside());
        assert_eq!(c.mods[ModType::Ctrl as usize], notpresent());
        assert_eq!(c.key, Some(0x0F));
    }

    #[test]
    fn modifier_only_single() {
        let c = parse("right_shift").unwrap();
        assert_eq!(c.mods[ModType::Shift as usize], right());
        assert_eq!(c.key, None);
        assert!(c.is_modifier_only());
    }

    #[test]
    fn modifier_only_multiple() {
        let c = parse("cmd+shift").unwrap();
        assert_eq!(c.mods[ModType::Cmd as usize], eitherside());
        assert_eq!(c.mods[ModType::Shift as usize], eitherside());
        assert_eq!(c.key, None);
    }

    #[test]
    fn double_tap_suffix() {
        let c = parse("right_shift:double").unwrap();
        assert!(c.double);
        assert_eq!(c.mods[ModType::Shift as usize], right());
        assert_eq!(c.key, None);

        let c = parse("cmd+;:double").unwrap();
        assert!(c.double);
        assert_eq!(c.mods[ModType::Cmd as usize], eitherside());
        assert_eq!(c.key, Some(0x29));
    }

    #[test]
    fn arrow_key_not_confused_with_modifier_prefix() {
        // `left` is the Left Arrow key.
        let c = parse("cmd+left").unwrap();
        assert_eq!(c.mods[ModType::Cmd as usize], eitherside());
        assert_eq!(c.key, Some(0x7B));
        // `left_cmd` is the modifier, not "left" + "cmd".
        let c = parse("left_cmd").unwrap();
        assert_eq!(c.mods[ModType::Cmd as usize], left());
        assert!(c.is_modifier_only());
    }

    #[test]
    fn punctuation_keys() {
        for &(name, code) in &[
            (";", 0x29u16),
            (",", 0x2B),
            (".", 0x2F),
            ("/", 0x2C),
            ("\\", 0x2A),
            ("[", 0x21),
            ("]", 0x1E),
            ("'", 0x27),
            ("`", 0x32),
            ("-", 0x1B),
            ("=", 0x18),
        ] {
            let c = parse(&format!("cmd+{name}")).unwrap();
            assert_eq!(c.key, Some(code), "punctuation {name:?}");
        }
    }

    #[test]
    fn modifier_aliases() {
        // All aliases must produce the canonical 3-letter form on Display
        // so that a TUI capture round-trip ends in a stable string.
        for (alias, canonical) in [
            ("command+r", "cmd+r"),
            ("control+r", "ctrl+r"),
            ("alt+r", "opt+r"),
            ("option+r", "opt+r"),
            ("left_alt+r", "left_opt+r"),
            ("right_option:double", "right_opt:double"),
        ] {
            let parsed = parse(alias).expect(alias);
            assert_eq!(parsed.to_string(), canonical, "alias {alias:?}");
            // And reparsing the canonical must equal the alias's parse.
            assert_eq!(parsed, parse(canonical).unwrap());
        }
    }

    #[test]
    fn key_aliases() {
        let parsed = parse("esc:double").unwrap();
        assert_eq!(parsed, parse("escape:double").unwrap());
        assert_eq!(parsed.to_string(), "escape:double");
    }

    #[test]
    fn merge_either_and_specific_degenerates_to_specific() {
        let c = parse("cmd+left_cmd+r").unwrap();
        assert_eq!(c.mods[ModType::Cmd as usize], left());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());
    }

    #[test]
    fn rejects_lone_suffix() {
        assert!(parse(":double").is_err());
    }

    #[test]
    fn rejects_double_suffix_twice() {
        assert!(parse("f16:double:double").is_err());
    }

    #[test]
    fn rejects_unknown_suffix() {
        assert!(parse("f16:triple").is_err());
        assert!(parse("f16:dbl").is_err());
    }

    #[test]
    fn rejects_trailing_plus() {
        assert!(parse("cmd+").is_err());
    }

    #[test]
    fn rejects_leading_plus() {
        assert!(parse("+r").is_err());
    }

    #[test]
    fn rejects_double_plus() {
        assert!(parse("cmd++r").is_err());
    }

    #[test]
    fn rejects_key_not_at_end() {
        assert!(parse("r+cmd").is_err());
    }

    #[test]
    fn rejects_duplicate_modifier() {
        assert!(parse("cmd+cmd+r").is_err());
        assert!(parse("left_cmd+left_cmd+r").is_err());
    }

    #[test]
    fn rejects_both_sides_explicit() {
        // Extending Combo to model "both sides held" isn't worth the type
        // complexity until a real user wants it.
        assert!(parse("left_cmd+right_cmd").is_err());
    }

    #[test]
    fn rejects_unknown_token() {
        assert!(parse("f21").is_err());
        assert!(parse("hyper+r").is_err());
        assert!(parse("foo").is_err());
    }

    #[test]
    fn display_round_trips() {
        // Combo::Display is used by the TUI capture path; this property
        // guards round-trip across edits-via-config-file.
        for input in [
            "f16",
            "cmd+r",
            "left_cmd+shift+r",
            "right_shift",
            "right_shift:double",
            "cmd+shift",
            "cmd+;:double",
            "cmd+left", // arrow key, not modifier
        ] {
            let c = parse(input).unwrap();
            let printed = c.to_string();
            let reparsed = parse(&printed).unwrap();
            assert_eq!(c, reparsed, "round trip failed for {input:?} → {printed:?}");
        }
    }

    #[test]
    fn modifier_keycodes_distinguished_by_side() {
        assert_eq!(
            modifier_from_keycode(0x37),
            Some((ModType::Cmd, Side::Left))
        );
        assert_eq!(
            modifier_from_keycode(0x36),
            Some((ModType::Cmd, Side::Right))
        );
        assert_eq!(
            modifier_from_keycode(0x38),
            Some((ModType::Shift, Side::Left))
        );
        assert_eq!(
            modifier_from_keycode(0x3C),
            Some((ModType::Shift, Side::Right))
        );
        assert_eq!(
            modifier_from_keycode(0x3A),
            Some((ModType::Opt, Side::Left))
        );
        assert_eq!(
            modifier_from_keycode(0x3D),
            Some((ModType::Opt, Side::Right))
        );
        assert_eq!(
            modifier_from_keycode(0x3B),
            Some((ModType::Ctrl, Side::Left))
        );
        assert_eq!(
            modifier_from_keycode(0x3E),
            Some((ModType::Ctrl, Side::Right))
        );
        // Non-modifier keys.
        assert_eq!(modifier_from_keycode(0x6A), None); // f16
        assert_eq!(modifier_from_keycode(0x00), None); // a
    }
}
