//! Top-level config: `~/.config/shuohua/config.toml`.
//!
//! provider 私有配置（app_key、language 等）由对应 provider impl 从
//! `~/.config/shuohua/asr/<provider>.toml` 自己加载，本模块不见。
//!
//! 路径解析遵循 XDG Base Directory：优先 $XDG_CONFIG_HOME，否则 $HOME/.config。

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::schema::{self, SchemaId};
use crate::config::spec::{validate_value, Diagnostic, Severity};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub hotkey: HotkeyCfg,
    #[serde(default)]
    pub voice: VoiceCfg,
    #[serde(default)]
    pub dev: DevCfg,
    #[serde(default)]
    pub ui: UiCfg,
    #[serde(default)]
    pub overlay: OverlayCfg,
    #[serde(default)]
    pub post: PostCfg,
    #[serde(default)]
    pub profile: ProfileRouteCfg,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HotkeyCfg {
    pub trigger: String,
    #[serde(default = "default_cancel_hotkey")]
    pub cancel: String,
}

fn default_cancel_hotkey() -> String {
    "escape".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoiceCfg {
    #[serde(default = "default_stop_delay_ms")]
    pub stop_delay_ms: u32,
    #[serde(default)]
    pub record_audio: RecordAudioMode,
    /// true (默认) = 识别完成后立刻 Cmd+V 上屏；false = 只进剪贴板。
    #[serde(default = "default_auto_paste")]
    pub auto_paste: bool,
    #[serde(default)]
    pub vad: VoiceVadCfg,
    #[serde(default)]
    pub preprocess: VoicePreprocessCfg,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordAudioMode {
    #[default]
    Off,
    Lossless,
    Compact,
}

impl fmt::Display for RecordAudioMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::Lossless => "lossless",
            Self::Compact => "compact",
        })
    }
}

impl Default for VoiceCfg {
    fn default() -> Self {
        Self {
            stop_delay_ms: default_stop_delay_ms(),
            record_audio: RecordAudioMode::Off,
            auto_paste: default_auto_paste(),
            vad: VoiceVadCfg::default(),
            preprocess: VoicePreprocessCfg::default(),
        }
    }
}

fn default_stop_delay_ms() -> u32 {
    800
}
fn default_auto_paste() -> bool {
    true
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DevCfg {
    #[serde(default)]
    pub vad_trace: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VoiceVadBackend {
    Off,
    Silero,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct VoiceVadCfg {
    #[serde(default = "default_vad_backend")]
    pub backend: VoiceVadBackend,
    #[serde(default = "default_vad_threshold")]
    pub threshold: f32,
    #[serde(default = "default_pause_silence_ms")]
    pub pause_silence_ms: u32,
    #[serde(default = "default_pre_roll_ms")]
    pub pre_roll_ms: u32,
    #[serde(default = "default_max_overlap_ms")]
    pub max_overlap_ms: u32,
    #[serde(default = "default_min_start_voiced_frames")]
    pub min_start_voiced_frames: u32,
}

impl Default for VoiceVadCfg {
    fn default() -> Self {
        Self {
            backend: default_vad_backend(),
            threshold: default_vad_threshold(),
            pause_silence_ms: default_pause_silence_ms(),
            pre_roll_ms: default_pre_roll_ms(),
            max_overlap_ms: default_max_overlap_ms(),
            min_start_voiced_frames: default_min_start_voiced_frames(),
        }
    }
}

fn default_vad_backend() -> VoiceVadBackend {
    VoiceVadBackend::Silero
}
fn default_vad_threshold() -> f32 {
    0.5
}
fn default_pause_silence_ms() -> u32 {
    1500
}
fn default_pre_roll_ms() -> u32 {
    300
}
fn default_max_overlap_ms() -> u32 {
    200
}
fn default_min_start_voiced_frames() -> u32 {
    2
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VoicePreprocessBackend {
    Off,
    #[default]
    Apple,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct VoicePreprocessCfg {
    #[serde(default)]
    pub backend: VoicePreprocessBackend,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PostCfg {
    #[serde(default = "default_post_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProfileRouteCfg {
    #[serde(default = "default_profile_name")]
    pub default: String,
    #[serde(flatten)]
    pub routes: BTreeMap<String, Vec<String>>,
}

impl Default for ProfileRouteCfg {
    fn default() -> Self {
        Self {
            default: default_profile_name(),
            routes: BTreeMap::new(),
        }
    }
}

fn default_profile_name() -> String {
    "default".to_string()
}

impl Default for PostCfg {
    fn default() -> Self {
        Self {
            timeout_ms: default_post_timeout_ms(),
        }
    }
}

fn default_post_timeout_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, Deserialize)]
pub struct UiCfg {
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub theme_tui: String,
    #[serde(default)]
    pub theme_overlay: String,
}

impl Default for UiCfg {
    fn default() -> Self {
        Self {
            language: default_language(),
            theme: default_theme(),
            theme_tui: String::new(),
            theme_overlay: String::new(),
        }
    }
}

fn default_language() -> String {
    "auto".to_string()
}
fn default_theme() -> String {
    crate::config::theme::DEFAULT_THEME_NAME.to_string()
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OverlayCfg {
    #[serde(default)]
    pub position: OverlayPosition,
    #[serde(default = "default_max_text_lines")]
    pub max_text_lines: usize,
}

impl Default for OverlayCfg {
    fn default() -> Self {
        Self {
            position: OverlayPosition::default(),
            max_text_lines: default_max_text_lines(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum OverlayPosition {
    Top,
    Middle,
    #[default]
    Bottom,
}

fn default_max_text_lines() -> usize {
    5
}

/// `$XDG_CONFIG_HOME/shuohua/config.toml` or `~/.config/shuohua/config.toml`.
pub fn default_path() -> PathBuf {
    crate::config::paths::main_config()
}

pub fn load_from(path: &Path) -> Result<Config> {
    let body = std::fs::read_to_string(path).with_context(|| {
        format!(
            "config not found at {}\nhint: create ~/.config/shuohua/config.toml, \
             ~/.config/shuohua/profile/default.toml, and ~/.config/shuohua/asr/<provider>.toml",
            path.display()
        )
    })?;
    parse(&body).with_context(|| format!("parse {}", path.display()))
}

pub fn parse(body: &str) -> Result<Config> {
    let value = toml::from_str::<toml::Value>(body)?;
    reject_schema_diagnostics(validate_value(&schema::spec_for(SchemaId::Main), &value))?;
    value.try_into::<Config>().map_err(Into::into)
}

pub(crate) fn reject_schema_diagnostics(diagnostics: Vec<Diagnostic>) -> Result<()> {
    if diagnostics.is_empty() {
        return Ok(());
    }
    let messages = diagnostics
        .into_iter()
        .filter(|diagnostic| matches!(diagnostic.severity, Severity::Error | Severity::Warning))
        .map(|diagnostic| format!("{}: {}", diagnostic.path, diagnostic.message))
        .collect::<Vec<_>>();
    if messages.is_empty() {
        return Ok(());
    }
    anyhow::bail!("invalid config:\n{}", messages.join("\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let body = r#"
[hotkey]
trigger = "f16"
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.hotkey.trigger, "f16");
        assert_eq!(cfg.hotkey.cancel, "escape");
        assert_eq!(cfg.voice.stop_delay_ms, 800);
        assert_eq!(cfg.voice.record_audio, RecordAudioMode::Off);
        assert!(cfg.voice.auto_paste);
        assert_eq!(cfg.voice.preprocess.backend, VoicePreprocessBackend::Apple);
        assert_eq!(cfg.ui.language, "auto");
        assert_eq!(cfg.ui.theme, "gruvbox-dark");
        assert_eq!(cfg.ui.theme_tui, "");
        assert_eq!(cfg.ui.theme_overlay, "");
        assert_eq!(cfg.post.timeout_ms, 30_000);
        assert_eq!(cfg.profile.default, "default");
        assert!(cfg.profile.routes.is_empty());
        assert_eq!(cfg.overlay.position, OverlayPosition::Bottom);
        assert_eq!(cfg.overlay.max_text_lines, 5);
    }

    #[test]
    fn parses_cancel_hotkey_override() {
        let body = r#"
[hotkey]
trigger = "f16"
cancel = "escape:double"
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.hotkey.cancel, "escape:double");
    }

    #[test]
    fn parses_with_voice_overrides() {
        let body = r#"
[hotkey]
trigger = "f16"

[voice]
stop_delay_ms = 1200
record_audio  = "compact"
auto_paste    = false
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.voice.stop_delay_ms, 1200);
        assert_eq!(cfg.voice.record_audio, RecordAudioMode::Compact);
        assert!(!cfg.voice.auto_paste);
    }

    #[test]
    fn parses_all_record_audio_modes() {
        for (value, expected) in [
            ("off", RecordAudioMode::Off),
            ("lossless", RecordAudioMode::Lossless),
            ("compact", RecordAudioMode::Compact),
        ] {
            let body =
                format!("[hotkey]\ntrigger = \"f16\"\n[voice]\nrecord_audio = \"{value}\"\n");
            assert_eq!(parse(&body).unwrap().voice.record_audio, expected);
        }
    }

    #[test]
    fn rejects_boolean_record_audio_value() {
        let error =
            parse("[hotkey]\ntrigger = \"f16\"\n[voice]\nrecord_audio = true\n").unwrap_err();

        assert!(error.to_string().contains("record_audio"));
    }

    #[test]
    fn auto_paste_defaults_to_true() {
        let body = r#"
[hotkey]
trigger = "f16"
"#;
        let cfg = parse(body).unwrap();
        assert!(cfg.voice.auto_paste, "auto_paste 默认应为 true");
    }

    #[test]
    fn vad_trace_defaults_to_false() {
        let body = r#"
[hotkey]
trigger = "f16"
"#;
        let cfg = parse(body).unwrap();
        assert!(!cfg.dev.vad_trace);
    }

    #[test]
    fn dev_vad_trace_is_configurable() {
        let body = r#"
[hotkey]
trigger = "f16"

[dev]
vad_trace = true
"#;
        let cfg = parse(body).unwrap();
        assert!(cfg.dev.vad_trace);
    }

    #[test]
    fn missing_required_section_errors() {
        let body = r#"
[voice]
stop_delay_ms = 800
"#;
        assert!(parse(body).is_err());
    }

    #[test]
    fn ui_language_is_configurable() {
        let body = r#"
[hotkey]
trigger = "f16"

[ui]
language = "zh-CN"
theme = "catppuccin-latte"
theme_tui = "terminal-dark"
theme_overlay = "nord"
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.ui.language, "zh-CN");
        assert_eq!(cfg.ui.theme, "catppuccin-latte");
        assert_eq!(cfg.ui.theme_tui, "terminal-dark");
        assert_eq!(cfg.ui.theme_overlay, "nord");
    }

    #[test]
    fn overlay_is_configurable() {
        let body = r#"
[hotkey]
trigger = "f16"

[overlay]
position          = "top"
max_text_lines    = 6
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.overlay.position, OverlayPosition::Top);
        assert_eq!(cfg.overlay.max_text_lines, 6);
    }

    #[test]
    fn rejects_removed_overlay_thinking_delay_ms() {
        let error = parse(
            r#"
[hotkey]
trigger = "f16"

[overlay]
thinking_delay_ms = 900
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("overlay.thinking_delay_ms"));
    }

    #[test]
    fn voice_vad_defaults_use_silero() {
        let cfg: Config = toml::from_str(
            r#"
[hotkey]
trigger = "f16"
"#,
        )
        .unwrap();

        assert_eq!(cfg.voice.vad.backend, VoiceVadBackend::Silero);
        assert!((cfg.voice.vad.threshold - 0.5).abs() < 1e-6);
        assert_eq!(cfg.voice.vad.pause_silence_ms, 1500);
        assert_eq!(cfg.voice.vad.pre_roll_ms, 300);
        assert_eq!(cfg.voice.vad.max_overlap_ms, 200);
        assert_eq!(cfg.voice.vad.min_start_voiced_frames, 2);
    }

    #[test]
    fn voice_vad_can_parse_silero_settings() {
        let cfg: Config = toml::from_str(
            r#"
[hotkey]
trigger = "f16"

[voice.vad]
backend = "silero"
threshold = 0.42
pause_silence_ms = 1200
pre_roll_ms = 250
max_overlap_ms = 180
min_start_voiced_frames = 3
"#,
        )
        .unwrap();

        assert_eq!(cfg.voice.vad.backend, VoiceVadBackend::Silero);
        assert!((cfg.voice.vad.threshold - 0.42).abs() < 1e-6);
        assert_eq!(cfg.voice.vad.pause_silence_ms, 1200);
        assert_eq!(cfg.voice.vad.pre_roll_ms, 250);
        assert_eq!(cfg.voice.vad.max_overlap_ms, 180);
        assert_eq!(cfg.voice.vad.min_start_voiced_frames, 3);
    }

    #[test]
    fn voice_preprocess_defaults_to_apple() {
        let cfg: Config = toml::from_str(
            r#"
[hotkey]
trigger = "f16"
"#,
        )
        .unwrap();

        assert_eq!(cfg.voice.preprocess.backend, VoicePreprocessBackend::Apple);
    }

    #[test]
    fn voice_preprocess_can_parse_apple_backend() {
        let cfg: Config = toml::from_str(
            r#"
[hotkey]
trigger = "f16"

[voice.preprocess]
backend = "apple"
"#,
        )
        .unwrap();

        assert_eq!(cfg.voice.preprocess.backend, VoicePreprocessBackend::Apple);
    }

    #[test]
    fn voice_preprocess_rejects_unknown_backend() {
        let error = parse(
            r#"
[hotkey]
trigger = "f16"

[voice.preprocess]
backend = "missing"
"#,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("voice.preprocess.backend"), "{error}");
    }

    #[test]
    fn post_is_configurable() {
        let body = r#"
[hotkey]
trigger = "f16"

[post]
timeout_ms = 3500
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.post.timeout_ms, 3500);
    }

    #[test]
    fn default_post_timeout_is_long_enough_for_llm_cleanup() {
        let cfg = parse(
            r#"
[hotkey]
trigger = "f16"
"#,
        )
        .unwrap();

        assert_eq!(cfg.post.timeout_ms, 30_000);
    }

    #[test]
    fn parse_rejects_unknown_fields() {
        let error = parse(
            r#"
[hotkey]
trigger = "f16"

[voice]
stop_delay_mss = 1200
"#,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("voice.stop_delay_mss"), "{error}");
        assert!(error.contains("unknown field"), "{error}");
    }

    #[test]
    fn parse_rejects_values_outside_safe_ranges() {
        let error = parse(
            r#"
[hotkey]
trigger = "f16"

[voice.vad]
threshold = 1.5

[overlay]
max_text_lines = 0
"#,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("voice.vad.threshold"), "{error}");
        assert!(error.contains("overlay.max_text_lines"), "{error}");
    }

    #[test]
    fn profile_routes_are_configurable() {
        let body = r#"
[hotkey]
trigger = "f16"

[profile]
default = "default"
agent = ["com.mitchellh.ghostty", "com.apple.Terminal"]
coding = ["com.apple.dt.Xcode"]
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.profile.default, "default");
        assert_eq!(
            cfg.profile.routes.get("agent").unwrap(),
            &vec![
                "com.mitchellh.ghostty".to_string(),
                "com.apple.Terminal".to_string()
            ]
        );
        assert_eq!(
            cfg.profile.routes.get("coding").unwrap(),
            &vec!["com.apple.dt.Xcode".to_string()]
        );
    }
}
