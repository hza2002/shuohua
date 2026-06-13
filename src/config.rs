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
    /// REQUIREMENTS §3.1 描述里"可选自动 Cmd+V 上屏"对应这个开关。
    #[serde(default = "default_auto_paste")]
    pub auto_paste: bool,
    /// 客户端 VAD 检测静音持续多少毫秒后关 ASR session（"思考不计费"机制）。
    /// 见 DESIGN §2.9。约束：≥ 1800（Doubao end_window 默认 800ms + 1000ms 缓冲）。
    #[serde(default = "default_pause_asr_silence_ms")]
    pub pause_asr_silence_ms: u32,
    /// 静音持续多少毫秒后完全停止录音（防忘按 toggle OFF）。默认 10 分钟。
    #[serde(default = "default_auto_stop_silence_ms")]
    pub auto_stop_silence_ms: u32,
    /// 多段 ASR session 拼接时的分隔符。默认空格。
    #[serde(default = "default_segment_separator")]
    pub segment_separator: String,
    /// 客户端 VAD 多 session（"思考不计费"）。实验性，默认关。
    /// 开启后：静音≥pause_asr_silence_ms 关 ASR、有声重开。
    /// 对安静环境效果好；嘈杂环境可能误判。
    #[serde(default)]
    pub vad_enabled: bool,
}

impl Default for VoiceCfg {
    fn default() -> Self {
        Self {
            stop_delay_ms: default_stop_delay_ms(),
            record_audio: false,
            auto_paste: default_auto_paste(),
            pause_asr_silence_ms: default_pause_asr_silence_ms(),
            auto_stop_silence_ms: default_auto_stop_silence_ms(),
            segment_separator: default_segment_separator(),
            vad_enabled: false,
        }
    }
}

fn default_stop_delay_ms() -> u32 {
    800
}
fn default_auto_paste() -> bool {
    true
}
fn default_pause_asr_silence_ms() -> u32 {
    3000
}
fn default_auto_stop_silence_ms() -> u32 {
    600_000
}
fn default_segment_separator() -> String {
    " ".to_string()
}

/// VAD 关 ASR session 的下限：必须明确大于 Doubao server VAD 触发窗（默认 800ms）
/// 加 1000ms 缓冲，确保客户端先于服务端 VAD 触发，避免双重切段。
const MIN_PAUSE_ASR_SILENCE_MS: u32 = 1800;

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
    let cfg = parse(&body).with_context(|| format!("parse {}", path.display()))?;
    cfg.validate().with_context(|| format!("validate {}", path.display()))?;
    Ok(cfg)
}

pub fn parse(body: &str) -> Result<Config> {
    toml::from_str::<Config>(body).map_err(Into::into)
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.voice.pause_asr_silence_ms < MIN_PAUSE_ASR_SILENCE_MS {
            anyhow::bail!(
                "[voice] pause_asr_silence_ms = {} 过小（最小 {}）；\
                 客户端 VAD 必须先于服务端 VAD（默认 800ms 窗口）触发，\
                 否则会被双重切段。建议 3000。",
                self.voice.pause_asr_silence_ms,
                MIN_PAUSE_ASR_SILENCE_MS
            );
        }
        if self.voice.auto_stop_silence_ms <= self.voice.pause_asr_silence_ms {
            anyhow::bail!(
                "[voice] auto_stop_silence_ms ({}) 必须严格大于 pause_asr_silence_ms ({})，\
                 否则 ASR session 还没机会关录音就先整体 stop 了。",
                self.voice.auto_stop_silence_ms,
                self.voice.pause_asr_silence_ms
            );
        }
        Ok(())
    }
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
    fn missing_required_section_errors() {
        let body = r#"
[hotkey]
trigger = "f16"
"#;
        // [asr] is required (no Default impl on AsrCfg) — parse must fail.
        assert!(parse(body).is_err());
    }

    #[test]
    fn vad_defaults_are_loaded() {
        let body = r#"
[hotkey]
trigger = "f16"

[asr]
provider = "doubao"
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.voice.pause_asr_silence_ms, 3000);
        assert_eq!(cfg.voice.auto_stop_silence_ms, 600_000);
        assert_eq!(cfg.voice.segment_separator, " ");
        assert!(!cfg.voice.vad_enabled);
        cfg.validate().expect("default config should validate");
    }

    #[test]
    fn vad_overrides_apply() {
        let body = r#"
[hotkey]
trigger = "f16"

[voice]
pause_asr_silence_ms = 4000
auto_stop_silence_ms = 900000
segment_separator    = "，"

[asr]
provider = "doubao"
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.voice.pause_asr_silence_ms, 4000);
        assert_eq!(cfg.voice.auto_stop_silence_ms, 900_000);
        assert_eq!(cfg.voice.segment_separator, "，");
        cfg.validate().unwrap();
    }

    #[test]
    fn pause_below_minimum_rejected() {
        let body = r#"
[hotkey]
trigger = "f16"

[voice]
pause_asr_silence_ms = 500

[asr]
provider = "doubao"
"#;
        let cfg = parse(body).unwrap();
        let err = cfg.validate().expect_err("500ms 应被拒绝");
        let msg = format!("{err:#}");
        assert!(msg.contains("pause_asr_silence_ms"), "msg = {msg}");
    }

    #[test]
    fn auto_stop_must_exceed_pause() {
        let body = r#"
[hotkey]
trigger = "f16"

[voice]
pause_asr_silence_ms = 3000
auto_stop_silence_ms = 3000

[asr]
provider = "doubao"
"#;
        let cfg = parse(body).unwrap();
        let err = cfg.validate().expect_err("auto_stop == pause 应被拒绝");
        assert!(format!("{err:#}").contains("auto_stop_silence_ms"));
    }
}
