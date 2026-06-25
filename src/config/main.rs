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

#[derive(Debug, Clone, Deserialize)]
pub struct PostCfg {
    #[serde(default = "default_post_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileRouteCfg {
    #[serde(default = "default_profile_name")]
    pub default: String,
    #[serde(default)]
    pub routes: ProfileRoutes,
}

impl Default for ProfileRouteCfg {
    fn default() -> Self {
        Self {
            default: default_profile_name(),
            routes: ProfileRoutes::default(),
        }
    }
}

impl ProfileRouteCfg {
    pub fn matching_profiles(&self, app: &AppIdentity<'_>) -> Vec<&str> {
        self.routes.matching_profiles(app)
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ProfileRoutes(BTreeMap<String, ProfileRouteMatchers>);

impl ProfileRoutes {
    pub fn get(&self, profile: &str) -> Option<&ProfileRouteMatchers> {
        self.0.get(profile)
    }

    pub fn contains_key(&self, profile: &str) -> bool {
        self.0.contains_key(profile)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.0.keys()
    }

    pub fn matching_profiles(&self, app: &AppIdentity<'_>) -> Vec<&str> {
        self.0
            .iter()
            .filter_map(|(profile, routes)| routes.matches(app).then_some(profile.as_str()))
            .collect()
    }
}

impl FromIterator<(String, ProfileRouteMatchers)> for ProfileRoutes {
    fn from_iter<T: IntoIterator<Item = (String, ProfileRouteMatchers)>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ProfileRouteMatchers {
    pub macos: MacosProfileMatchers,
    pub windows: WindowsProfileMatchers,
    pub linux: LinuxProfileMatchers,
}

impl ProfileRouteMatchers {
    fn matches(&self, app: &AppIdentity<'_>) -> bool {
        match app {
            AppIdentity::Macos { bundle_id } => self.macos.matches(*bundle_id),
            AppIdentity::Windows {
                app_user_model_id,
                exe_name,
            } => self.windows.matches(*app_user_model_id, *exe_name),
            AppIdentity::Linux {
                desktop_id,
                wm_class,
                process_name,
            } => self.linux.matches(*desktop_id, *wm_class, *process_name),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MacosProfileMatchers {
    pub bundle_id: Vec<String>,
}

impl MacosProfileMatchers {
    fn matches(&self, bundle_id: Option<&str>) -> bool {
        bundle_id.is_some_and(|actual| self.bundle_id.iter().any(|expected| expected == actual))
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct WindowsProfileMatchers {
    pub app_user_model_id: Vec<String>,
    pub exe_name: Vec<String>,
}

impl WindowsProfileMatchers {
    fn matches(&self, app_user_model_id: Option<&str>, exe_name: Option<&str>) -> bool {
        app_user_model_id
            .is_some_and(|actual| contains_case_insensitive(&self.app_user_model_id, actual))
            || exe_name.is_some_and(|actual| contains_case_insensitive(&self.exe_name, actual))
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct LinuxProfileMatchers {
    pub desktop_id: Vec<String>,
    pub wm_class: Vec<String>,
    pub process_name: Vec<String>,
}

impl LinuxProfileMatchers {
    fn matches(
        &self,
        desktop_id: Option<&str>,
        wm_class: Option<&str>,
        process_name: Option<&str>,
    ) -> bool {
        desktop_id.is_some_and(|actual| self.desktop_id.iter().any(|expected| expected == actual))
            || wm_class
                .is_some_and(|actual| self.wm_class.iter().any(|expected| expected == actual))
            || process_name
                .is_some_and(|actual| self.process_name.iter().any(|expected| expected == actual))
    }
}

fn contains_case_insensitive(values: &[String], actual: &str) -> bool {
    values
        .iter()
        .any(|expected| expected.eq_ignore_ascii_case(actual))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIdentity<'a> {
    Macos {
        bundle_id: Option<&'a str>,
    },
    Windows {
        app_user_model_id: Option<&'a str>,
        exe_name: Option<&'a str>,
    },
    Linux {
        desktop_id: Option<&'a str>,
        wm_class: Option<&'a str>,
        process_name: Option<&'a str>,
    },
}

impl<'a> AppIdentity<'a> {
    pub fn macos(bundle_id: Option<&'a str>) -> Self {
        Self::Macos { bundle_id }
    }

    pub fn windows(app_user_model_id: Option<&'a str>, exe_name: Option<&'a str>) -> Self {
        Self::Windows {
            app_user_model_id,
            exe_name,
        }
    }

    pub fn linux(
        desktop_id: Option<&'a str>,
        wm_class: Option<&'a str>,
        process_name: Option<&'a str>,
    ) -> Self {
        Self::Linux {
            desktop_id,
            wm_class,
            process_name,
        }
    }

    pub fn current_from_app_context(bundle_id: Option<&'a str>) -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::macos(bundle_id)
        }
        #[cfg(windows)]
        {
            let _ = bundle_id;
            Self::windows(None, None)
        }
        #[cfg(target_os = "linux")]
        {
            let _ = bundle_id;
            Self::linux(None, None, None)
        }
        #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
        {
            let _ = bundle_id;
            Self::linux(None, None, None)
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
    fn profile_routes_are_platform_specific() {
        let body = r#"
[hotkey]
trigger = "f16"

[profile]
default = "default"

[profile.routes.agent.macos]
bundle_id = ["com.mitchellh.ghostty", "com.apple.Terminal"]

[profile.routes.agent.windows]
app_user_model_id = ["Microsoft.WindowsTerminal"]
exe_name = ["WindowsTerminal.exe"]

[profile.routes.coding.linux]
desktop_id = ["code.desktop"]
wm_class = ["Code"]
process_name = ["code"]
"#;
        let cfg = parse(body).unwrap();
        assert_eq!(cfg.profile.default, "default");
        assert_eq!(
            &cfg.profile.routes.get("agent").unwrap().macos.bundle_id,
            &vec![
                "com.mitchellh.ghostty".to_string(),
                "com.apple.Terminal".to_string()
            ]
        );
        assert_eq!(
            &cfg.profile.routes.get("agent").unwrap().windows.exe_name,
            &vec!["WindowsTerminal.exe".to_string()]
        );
        assert_eq!(
            &cfg.profile.routes.get("coding").unwrap().linux.desktop_id,
            &vec!["code.desktop".to_string()]
        );
    }

    #[test]
    fn profile_routes_match_windows_identity_case_insensitively() {
        let routes = ProfileRouteCfg {
            default: "default".to_string(),
            routes: ProfileRoutes::from_iter([(
                "agent".to_string(),
                ProfileRouteMatchers {
                    windows: WindowsProfileMatchers {
                        app_user_model_id: vec!["Microsoft.VisualStudioCode".to_string()],
                        exe_name: vec!["Code.exe".to_string()],
                    },
                    ..Default::default()
                },
            )]),
        };

        assert_eq!(
            routes.matching_profiles(&AppIdentity::windows(
                Some("microsoft.visualstudiocode"),
                None
            )),
            vec!["agent"]
        );
        assert_eq!(
            routes.matching_profiles(&AppIdentity::windows(None, Some("code.EXE"))),
            vec!["agent"]
        );
        assert!(routes
            .matching_profiles(&AppIdentity::macos(Some("Code.exe")))
            .is_empty());
    }

    #[test]
    fn rejects_legacy_top_level_profile_route_arrays() {
        let error = parse(
            r#"
[hotkey]
trigger = "f16"

[profile]
default = "default"
agent = ["com.mitchellh.ghostty"]
"#,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("profile.agent"), "{error}");
        assert!(error.contains("unknown field"), "{error}");
    }
}
