//! Hotkey 字符串 → macOS 物理虚拟键码（u16）。
//!
//! M2 scope: 只支持无修饰键的功能键（F1–F20）。普通字符键作为语音热键被
//! DESIGN §5 不变量 6 禁掉；修饰键组合（Cmd+Space 等）需要 tracker 支持
//! modifier mask，留给 M6 跟 suppress 一起做。
//!
//! 虚拟键码来自 HIToolbox/Events.h，跨键盘布局稳定（基于物理位置）。

use anyhow::{bail, Result};

pub fn parse(s: &str) -> Result<u16> {
    let s = s.trim();
    if s.is_empty() {
        bail!("empty hotkey string");
    }
    if s.contains('+') {
        bail!(
            "hotkey {s:?} 含修饰键；M2 仅支持单键（F1–F20）。\
             组合键支持在 M6 跟热键 suppress 一起做"
        );
    }
    match s.to_ascii_lowercase().as_str() {
        "f1" => Ok(0x7A),
        "f2" => Ok(0x78),
        "f3" => Ok(0x63),
        "f4" => Ok(0x76),
        "f5" => Ok(0x60),
        "f6" => Ok(0x61),
        "f7" => Ok(0x62),
        "f8" => Ok(0x64),
        "f9" => Ok(0x65),
        "f10" => Ok(0x6D),
        "f11" => Ok(0x67),
        "f12" => Ok(0x6F),
        "f13" => Ok(0x69),
        "f14" => Ok(0x6B),
        "f15" => Ok(0x71),
        "f16" => Ok(0x6A),
        "f17" => Ok(0x40),
        "f18" => Ok(0x4F),
        "f19" => Ok(0x50),
        "f20" => Ok(0x5A),
        _ => bail!("unknown or unsupported hotkey {s:?}; M2 supports F1–F20"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f16_is_0x6a() {
        assert_eq!(parse("f16").unwrap(), 0x6A);
        assert_eq!(parse("F16").unwrap(), 0x6A);
        assert_eq!(parse("  F16  ").unwrap(), 0x6A);
    }

    #[test]
    fn f_key_range() {
        // spot-check both ends and a middle value
        assert_eq!(parse("f1").unwrap(), 0x7A);
        assert_eq!(parse("f9").unwrap(), 0x65);
        assert_eq!(parse("f20").unwrap(), 0x5A);
    }

    #[test]
    fn rejects_combo() {
        let e = parse("cmd+space").unwrap_err().to_string();
        assert!(e.contains("M6"), "should explain M6 deferral, got: {e}");
    }

    #[test]
    fn rejects_unknown_key() {
        assert!(parse("space").is_err());
        assert!(parse("a").is_err());
        assert!(parse("f21").is_err()); // out of range
    }

    #[test]
    fn rejects_empty() {
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());
    }
}
