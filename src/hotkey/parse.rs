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

use super::combo::{Combo, ModMatcher, ModType, Side};
use super::key::{key_from_name, Key};

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
    let mut key: Option<Key> = None;

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
        } else if let Some(code) = key_from_name(token) {
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
        assert_eq!(c.key, Some(Key::F(16)));
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
        assert_eq!(c.key, Some(Key::Char('r')));
    }

    #[test]
    fn combo_multiple_modifiers() {
        let c = parse("left_cmd+shift+r").unwrap();
        assert_eq!(c.mods[ModType::Cmd as usize], left());
        assert_eq!(c.mods[ModType::Shift as usize], eitherside());
        assert_eq!(c.mods[ModType::Ctrl as usize], notpresent());
        assert_eq!(c.key, Some(Key::Char('r')));
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
        assert_eq!(c.key, Some(Key::Punct(';')));
    }

    #[test]
    fn arrow_key_not_confused_with_modifier_prefix() {
        // `left` is the Left Arrow key.
        let c = parse("cmd+left").unwrap();
        assert_eq!(c.mods[ModType::Cmd as usize], eitherside());
        assert_eq!(c.key, Some(Key::ArrowLeft));
        // `left_cmd` is the modifier, not "left" + "cmd".
        let c = parse("left_cmd").unwrap();
        assert_eq!(c.mods[ModType::Cmd as usize], left());
        assert!(c.is_modifier_only());
    }

    #[test]
    fn punctuation_keys() {
        for &(name, key) in &[
            (";", Key::Punct(';')),
            (",", Key::Punct(',')),
            (".", Key::Punct('.')),
            ("/", Key::Punct('/')),
            ("\\", Key::Punct('\\')),
            ("[", Key::Punct('[')),
            ("]", Key::Punct(']')),
            ("'", Key::Punct('\'')),
            ("`", Key::Punct('`')),
            ("-", Key::Punct('-')),
            ("=", Key::Punct('=')),
        ] {
            let c = parse(&format!("cmd+{name}")).unwrap();
            assert_eq!(c.key, Some(key), "punctuation {name:?}");
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
}
