//! Top-level config: `~/.config/shuohua/config.toml`.
//!
//! provider 私有配置（app_key、language 等）由对应 provider impl 从
//! `~/.config/shuohua/asr/<provider>.toml` 自己加载，本模块不见。
//!
//! 路径解析遵循 XDG Base Directory：优先 $XDG_CONFIG_HOME，否则 $HOME/.config。

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub hotkey: HotkeyCfg,
    #[serde(default)]
    pub voice: VoiceCfg,
    pub asr: AsrCfg,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HotkeyCfg {
    pub trigger: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoiceCfg {
    #[serde(default = "default_stop_delay_ms")]
    pub stop_delay_ms: u32,
    #[serde(default)]
    pub record_audio: bool,
}

impl Default for VoiceCfg {
    fn default() -> Self {
        Self { stop_delay_ms: default_stop_delay_ms(), record_audio: false }
    }
}

fn default_stop_delay_ms() -> u32 {
    800
}

#[derive(Debug, Clone, Deserialize)]
pub struct AsrCfg {
    pub provider: String,
    #[serde(default)]
    pub hotwords: Vec<String>,
}

/// `$XDG_CONFIG_HOME/shuohua/config.toml` or `~/.config/shuohua/config.toml`.
pub fn default_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("shuohua/config.toml");
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/shuohua/config.toml")
}

pub fn load_from(path: &Path) -> Result<Config> {
    let body = std::fs::read_to_string(path).with_context(|| {
        format!(
            "config not found at {}\nhint: cp examples/config/config.toml \
             ~/.config/shuohua/ and edit; also create ~/.config/shuohua/asr/<provider>.toml",
            path.display()
        )
    })?;
    parse(&body).with_context(|| format!("parse {}", path.display()))
}

pub fn parse(body: &str) -> Result<Config> {
    toml::from_str::<Config>(body).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let body = r#"
[hotkey]
trigger = "f16"

[asr]
provider = "doubao"
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.hotkey.trigger, "f16");
        assert_eq!(cfg.asr.provider, "doubao");
        assert!(cfg.asr.hotwords.is_empty());
        assert_eq!(cfg.voice.stop_delay_ms, 800);
        assert!(!cfg.voice.record_audio);
    }

    #[test]
    fn parses_with_voice_overrides_and_hotwords() {
        let body = r#"
[hotkey]
trigger = "f16"

[voice]
stop_delay_ms = 1200
record_audio  = true

[asr]
provider = "doubao"
hotwords = ["Rust", "tokio"]
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.voice.stop_delay_ms, 1200);
        assert!(cfg.voice.record_audio);
        assert_eq!(cfg.asr.hotwords, vec!["Rust", "tokio"]);
    }

    #[test]
    fn missing_required_section_errors() {
        let body = r#"
[hotkey]
trigger = "f16"
"#;
        // [asr] is required (no Default impl on AsrCfg) — parse must fail.
        assert!(parse(body).is_err());
    }
}
