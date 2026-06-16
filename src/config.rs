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
    #[serde(default)]
    pub ui: UiCfg,
    #[serde(default)]
    pub overlay: OverlayCfg,
    #[serde(default)]
    pub post: PostCfg,
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
    pub record_audio: bool,
    #[serde(default)]
    pub vad_trace: bool,
    /// true (默认) = 识别完成后立刻 Cmd+V 上屏；false = 只进剪贴板。
    #[serde(default = "default_auto_paste")]
    pub auto_paste: bool,
    #[serde(default)]
    pub vad: VoiceVadCfg,
}

impl Default for VoiceCfg {
    fn default() -> Self {
        Self {
            stop_delay_ms: default_stop_delay_ms(),
            record_audio: false,
            vad_trace: false,
            auto_paste: default_auto_paste(),
            vad: VoiceVadCfg::default(),
        }
    }
}

fn default_stop_delay_ms() -> u32 {
    800
}
fn default_auto_paste() -> bool {
    true
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
    VoiceVadBackend::Off
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

#[derive(Debug, Clone, Deserialize)]
pub struct PostCfg {
    #[serde(default = "default_post_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for PostCfg {
    fn default() -> Self {
        Self {
            timeout_ms: default_post_timeout_ms(),
        }
    }
}

fn default_post_timeout_ms() -> u64 {
    2000
}

#[derive(Debug, Clone, Deserialize)]
pub struct UiCfg {
    #[serde(default = "default_language")]
    pub language: String,
}

impl Default for UiCfg {
    fn default() -> Self {
        Self {
            language: default_language(),
        }
    }
}

fn default_language() -> String {
    "auto".to_string()
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OverlayCfg {
    #[serde(default)]
    pub position: OverlayPosition,
    #[serde(default = "default_glass_variant")]
    pub glass_variant: i64,
    #[serde(default)]
    pub glass_style: GlassStyle,
    #[serde(default = "default_background_rgb")]
    pub background_rgb: u32,
    #[serde(default = "default_background_alpha")]
    pub background_alpha: f64,
    #[serde(default)]
    pub background_blur_radius: i64,
    #[serde(default = "default_corner_radius")]
    pub corner_radius: f64,
    /// 私有 SPI `set_subduedState:` 入参（0 = 正常，1 = 降亮度饱和度；理论可能有更高状态）。
    #[serde(default)]
    pub subdued: i64,
    #[serde(default = "default_max_text_lines")]
    pub max_text_lines: usize,
    #[serde(default = "default_thinking_delay_ms")]
    pub thinking_delay_ms: u64,
}

impl Default for OverlayCfg {
    fn default() -> Self {
        Self {
            position: OverlayPosition::default(),
            glass_variant: default_glass_variant(),
            glass_style: GlassStyle::default(),
            background_rgb: default_background_rgb(),
            background_alpha: default_background_alpha(),
            background_blur_radius: 0,
            corner_radius: default_corner_radius(),
            subdued: 0,
            max_text_lines: default_max_text_lines(),
            thinking_delay_ms: default_thinking_delay_ms(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum OverlayPosition {
    Top,
    Middle,
    Bottom,
}

impl Default for OverlayPosition {
    fn default() -> Self {
        Self::Bottom
    }
}

/// 液态玻璃的 `setStyle:` 预设。
/// - `Clear` ── 不调 setStyle，只走 variant，最透
/// - `Blur`  ── 调 `NSGlassEffectViewStyle::Clear`，带轻量模糊（不带暗化）
///
/// 命名以"用户视觉效果"为准，AppKit 内部的 `Clear` 常量在我们这里对应 `Blur`。
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum GlassStyle {
    #[default]
    Clear,
    Blur,
}

fn default_glass_variant() -> i64 {
    19
}

fn default_background_rgb() -> u32 {
    0x000000
}

fn default_corner_radius() -> f64 {
    18.0
}

fn default_background_alpha() -> f64 {
    0.18
}

fn default_max_text_lines() -> usize {
    5
}

fn default_thinking_delay_ms() -> u64 {
    1200
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
            "config not found at {}\nhint: create ~/.config/shuohua/config.toml, \
             ~/.config/shuohua/apps/default.toml, and ~/.config/shuohua/asr/<provider>.toml",
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
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.hotkey.trigger, "f16");
        assert_eq!(cfg.hotkey.cancel, "escape");
        assert_eq!(cfg.voice.stop_delay_ms, 800);
        assert!(!cfg.voice.record_audio);
        assert!(cfg.voice.auto_paste);
        assert_eq!(cfg.ui.language, "auto");
        assert_eq!(cfg.post.timeout_ms, 2000);
        assert_eq!(cfg.overlay.position, OverlayPosition::Bottom);
        assert_eq!(cfg.overlay.glass_variant, 19);
        assert_eq!(cfg.overlay.glass_style, GlassStyle::Clear);
        assert_eq!(cfg.overlay.background_rgb, 0x000000);
        assert!((cfg.overlay.background_alpha - 0.18).abs() < 1e-9);
        assert_eq!(cfg.overlay.background_blur_radius, 0);
        assert!((cfg.overlay.corner_radius - 18.0).abs() < 1e-9);
        assert_eq!(cfg.overlay.subdued, 0);
        assert_eq!(cfg.overlay.max_text_lines, 5);
        assert_eq!(cfg.overlay.thinking_delay_ms, 1200);
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
record_audio  = true
auto_paste    = false
vad_trace     = true
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.voice.stop_delay_ms, 1200);
        assert!(cfg.voice.record_audio);
        assert!(!cfg.voice.auto_paste);
        assert!(cfg.voice.vad_trace);
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
        assert!(!cfg.voice.vad_trace);
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
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.ui.language, "zh-CN");
    }

    #[test]
    fn overlay_is_configurable() {
        let body = r#"
[hotkey]
trigger = "f16"

[overlay]
position          = "top"
glass_variant     = 13
glass_style       = "blur"
background_rgb    = 0x111111
background_alpha  = 0.24
background_blur_radius = 24
corner_radius     = 22.0
subdued           = 1
max_text_lines    = 6
thinking_delay_ms = 900
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.overlay.position, OverlayPosition::Top);
        assert_eq!(cfg.overlay.glass_variant, 13);
        assert_eq!(cfg.overlay.glass_style, GlassStyle::Blur);
        assert_eq!(cfg.overlay.background_rgb, 0x111111);
        assert!((cfg.overlay.background_alpha - 0.24).abs() < 1e-9);
        assert_eq!(cfg.overlay.background_blur_radius, 24);
        assert!((cfg.overlay.corner_radius - 22.0).abs() < 1e-9);
        assert_eq!(cfg.overlay.subdued, 1);
        assert_eq!(cfg.overlay.max_text_lines, 6);
        assert_eq!(cfg.overlay.thinking_delay_ms, 900);
    }

    #[test]
    fn voice_vad_defaults_are_disabled() {
        let cfg: Config = toml::from_str(
            r#"
[hotkey]
trigger = "f16"
"#,
        )
        .unwrap();

        assert_eq!(cfg.voice.vad.backend, VoiceVadBackend::Off);
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
}
