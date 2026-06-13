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
    /// true (默认) = 识别完成后立刻 Cmd+V 上屏；false = 只进剪贴板。
    #[serde(default = "default_auto_paste")]
    pub auto_paste: bool,
    /// 多段 ASR segment 拼接时的分隔符。默认空格。
    /// 目前只有 Doubao server VAD 切段（单 session 内），未来加客户端 VAD
    /// 多 session 后也用它拼 session 间文本。
    #[serde(default = "default_segment_separator")]
    pub segment_separator: String,
}

impl Default for VoiceCfg {
    fn default() -> Self {
        Self {
            stop_delay_ms: default_stop_delay_ms(),
            record_audio: false,
            auto_paste: default_auto_paste(),
            segment_separator: default_segment_separator(),
        }
    }
}

fn default_stop_delay_ms() -> u32 {
    800
}
fn default_auto_paste() -> bool {
    true
}
fn default_segment_separator() -> String {
    " ".to_string()
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
        assert!(cfg.voice.auto_paste);
        assert_eq!(cfg.voice.segment_separator, " ");
    }

    #[test]
    fn parses_with_voice_overrides_and_hotwords() {
        let body = r#"
[hotkey]
trigger = "f16"

[voice]
stop_delay_ms = 1200
record_audio  = true
auto_paste    = false

[asr]
provider = "doubao"
hotwords = ["Rust", "tokio"]
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.voice.stop_delay_ms, 1200);
        assert!(cfg.voice.record_audio);
        assert!(!cfg.voice.auto_paste);
        assert_eq!(cfg.asr.hotwords, vec!["Rust", "tokio"]);
    }

    #[test]
    fn auto_paste_defaults_to_true() {
        let body = r#"
[hotkey]
trigger = "f16"

[asr]
provider = "doubao"
"#;
        let cfg = parse(body).unwrap();
        assert!(cfg.voice.auto_paste, "auto_paste 默认应为 true (REQUIREMENTS §3.1)");
    }

    #[test]
    fn segment_separator_overridable() {
        let body = r#"
[hotkey]
trigger = "f16"

[voice]
segment_separator = "，"

[asr]
provider = "doubao"
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.voice.segment_separator, "，");
    }

    #[test]
    fn missing_required_section_errors() {
        let body = r#"
[hotkey]
trigger = "f16"
"#;
        assert!(parse(body).is_err());
    }
}
