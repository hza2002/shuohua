use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::{self, Visitor};
use serde::Deserialize;

use crate::config::OverlayPosition;

pub const DEFAULT_THEME_NAME: &str = "gruvbox-dark";
pub const LEGACY_DEFAULT_THEME_NAME: &str = "default";

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum GlassStyle {
    #[default]
    Clear,
    Blur,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveTheme {
    pub theme: String,
    pub theme_tui: String,
    pub theme_overlay: String,
    pub tui: TuiTheme,
    pub overlay: EffectiveOverlayCfg,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveThemeLoad {
    pub theme: EffectiveTheme,
    pub warning: Option<ThemeLoadWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeLoadWarning {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiTheme {
    pub foreground: u32,
    pub muted: u32,
    pub accent: u32,
    pub success: u32,
    pub warning: u32,
    pub error: u32,
    pub info: u32,
    pub highlight: u32,
    pub border: u32,
    pub border_focus: u32,
    pub segment: u32,
}

/// Overlay runtime configuration after merging main config and theme files.
/// This is consumed directly by overlay renderers; it is not a TOML schema.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EffectiveOverlayCfg {
    pub core: CoreOverlayCfg,
    pub macos: MacosOverlayCfg,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoreOverlayCfg {
    pub position: OverlayPosition,
    pub width: f64,
    pub background_rgb: u32,
    pub background_alpha: f64,
    pub corner_radius: f64,
    pub max_text_lines: usize,
    pub text: OverlayTextTheme,
    pub state: OverlayStateTheme,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MacosOverlayCfg {
    pub glass_variant: i64,
    pub glass_style: GlassStyle,
    pub subdued: i64,
    pub background_blur_radius: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayTextTheme {
    pub primary: u32,
    pub secondary: u32,
    pub tertiary: u32,
    pub segment: u32,
    pub notice: u32,
    pub error: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayStateTheme {
    pub idle: u32,
    pub connecting: u32,
    pub recording: u32,
    pub thinking: u32,
    pub stopping: u32,
    pub error: u32,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ThemeFile {
    #[serde(default)]
    #[allow(dead_code)]
    pub name: Option<String>,
    #[serde(default)]
    pub palette: BTreeMap<String, ColorValue>,
    #[serde(default)]
    pub tui: PartialTuiTheme,
    #[serde(default)]
    pub overlay: PartialOverlayTheme,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PartialTuiTheme {
    pub foreground: Option<ColorValue>,
    pub muted: Option<ColorValue>,
    pub accent: Option<ColorValue>,
    pub success: Option<ColorValue>,
    pub warning: Option<ColorValue>,
    pub error: Option<ColorValue>,
    pub info: Option<ColorValue>,
    pub highlight: Option<ColorValue>,
    pub border: Option<ColorValue>,
    pub border_focus: Option<ColorValue>,
    pub segment: Option<ColorValue>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PartialOverlayTheme {
    #[serde(default)]
    pub macos: PartialOverlayMacos,
    #[serde(default)]
    pub surface: PartialOverlaySurface,
    #[serde(default)]
    pub text: PartialOverlayText,
    #[serde(default)]
    pub state: PartialOverlayState,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PartialOverlayMacos {
    pub glass_variant: Option<i64>,
    pub glass_style: Option<GlassStyle>,
    pub subdued: Option<i64>,
    pub background_blur_radius: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PartialOverlaySurface {
    pub background: Option<ColorValue>,
    pub background_alpha: Option<f64>,
    pub corner_radius: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PartialOverlayText {
    pub primary: Option<ColorValue>,
    pub secondary: Option<ColorValue>,
    pub tertiary: Option<ColorValue>,
    pub segment: Option<ColorValue>,
    pub notice: Option<ColorValue>,
    pub error: Option<ColorValue>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PartialOverlayState {
    pub idle: Option<ColorValue>,
    pub connecting: Option<ColorValue>,
    pub recording: Option<ColorValue>,
    pub thinking: Option<ColorValue>,
    pub stopping: Option<ColorValue>,
    pub error: Option<ColorValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorValue {
    Hex(u32),
    Palette(String),
}

impl<'de> Deserialize<'de> for ColorValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ColorValueVisitor;

        impl Visitor<'_> for ColorValueVisitor {
            type Value = ColorValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a 0xRRGGBB integer or palette color name")
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                if !(0..=0xFF_FFFF).contains(&value) {
                    return Err(E::custom("color must be in 0x000000..=0xFFFFFF"));
                }
                Ok(ColorValue::Hex(value as u32))
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value > 0xFF_FFFF {
                    return Err(E::custom("color must be in 0x000000..=0xFFFFFF"));
                }
                Ok(ColorValue::Hex(value as u32))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                let value = value.trim();
                if value.is_empty() {
                    return Err(E::custom("palette color name cannot be empty"));
                }
                Ok(ColorValue::Palette(value.to_string()))
            }
        }

        deserializer.deserialize_any(ColorValueVisitor)
    }
}

impl Default for TuiTheme {
    fn default() -> Self {
        Self {
            foreground: palette::FG0,
            muted: palette::GRAY,
            accent: palette::BRIGHT_AQUA,
            success: palette::BRIGHT_AQUA,
            warning: palette::BRIGHT_YELLOW,
            error: palette::BRIGHT_RED,
            info: palette::BRIGHT_BLUE,
            highlight: palette::BRIGHT_AQUA,
            border: palette::GRAY,
            border_focus: palette::BRIGHT_AQUA,
            segment: palette::FG1,
        }
    }
}

impl Default for OverlayTextTheme {
    fn default() -> Self {
        Self {
            primary: palette::FG0,
            secondary: palette::FG1,
            tertiary: palette::FG3,
            segment: palette::FG1,
            notice: palette::BRIGHT_YELLOW,
            error: palette::BRIGHT_RED,
        }
    }
}

impl Default for OverlayStateTheme {
    fn default() -> Self {
        Self {
            idle: palette::BRIGHT_AQUA,
            connecting: palette::BRIGHT_ORANGE,
            recording: palette::BRIGHT_RED,
            thinking: palette::BRIGHT_BLUE,
            stopping: palette::BRIGHT_YELLOW,
            error: palette::RED,
        }
    }
}

impl Default for CoreOverlayCfg {
    fn default() -> Self {
        Self {
            position: OverlayPosition::default(),
            width: crate::overlay::layout::constants::WIDTH,
            background_rgb: 0x282828,
            background_alpha: 0.70,
            corner_radius: 18.0,
            max_text_lines: 5,
            text: OverlayTextTheme::default(),
            state: OverlayStateTheme::default(),
        }
    }
}

impl Default for MacosOverlayCfg {
    fn default() -> Self {
        Self {
            glass_variant: 11,
            glass_style: GlassStyle::Clear,
            subdued: 0,
            background_blur_radius: 0,
        }
    }
}

impl Default for EffectiveTheme {
    fn default() -> Self {
        Self {
            theme: DEFAULT_THEME_NAME.to_string(),
            theme_tui: DEFAULT_THEME_NAME.to_string(),
            theme_overlay: DEFAULT_THEME_NAME.to_string(),
            tui: TuiTheme::default(),
            overlay: EffectiveOverlayCfg::default(),
        }
    }
}

pub mod palette {
    pub const FG0: u32 = 0xFBF1C7;
    pub const FG1: u32 = 0xEBDBB2;
    pub const FG2: u32 = 0xD5C4A1;
    pub const FG3: u32 = 0xBDAE93;
    pub const FG4: u32 = 0xA89984;
    pub const GRAY: u32 = 0x928374;
    pub const BRIGHT_RED: u32 = 0xFB4934;
    pub const RED: u32 = 0xCC241D;
    pub const BRIGHT_ORANGE: u32 = 0xFE8019;
    pub const BRIGHT_YELLOW: u32 = 0xFABD2F;
    pub const BRIGHT_BLUE: u32 = 0x83A598;
    pub const BRIGHT_AQUA: u32 = 0x8EC07C;
    pub const BACKGROUND: u32 = 0x000000;
}

pub fn load_effective(config: &crate::config::Config, config_path: &Path) -> EffectiveTheme {
    load_effective_report(config, config_path).theme
}

pub fn load_effective_report(
    config: &crate::config::Config,
    config_path: &Path,
) -> EffectiveThemeLoad {
    let root = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    match load_effective_from_root(config, &root) {
        Ok(theme) => EffectiveThemeLoad {
            theme,
            warning: None,
        },
        Err(error) => {
            let message = format!("{error:#}");
            tracing::warn!(error = ?error, "theme load failed; using builtin default theme");
            EffectiveThemeLoad {
                theme: default_for_config(config),
                warning: Some(ThemeLoadWarning { message }),
            }
        }
    }
}

pub fn load_effective_from_root(
    config: &crate::config::Config,
    root: &Path,
) -> Result<EffectiveTheme> {
    let base = normalized_theme_name(&config.ui.theme);
    let tui_name = normalized_theme_name(non_empty_or(&config.ui.theme_tui, base));
    let overlay_name = normalized_theme_name(non_empty_or(&config.ui.theme_overlay, base));

    let tui_file = load_theme_file(root, tui_name)?;
    let overlay_file = if overlay_name == tui_name {
        tui_file.clone()
    } else {
        load_theme_file(root, overlay_name)?
    };

    let mut effective = default_for_config(config);
    effective.theme = base.to_string();
    effective.theme_tui = tui_name.to_string();
    effective.theme_overlay = overlay_name.to_string();
    apply_tui_theme(&mut effective.tui, &tui_file)?;
    apply_overlay_theme(&mut effective.overlay, &overlay_file)?;
    Ok(effective)
}

pub fn default_for_config(config: &crate::config::Config) -> EffectiveTheme {
    let mut theme = EffectiveTheme::default();
    theme.theme = normalized_theme_name(&config.ui.theme).to_string();
    theme.theme_tui =
        normalized_theme_name(non_empty_or(&config.ui.theme_tui, &theme.theme)).to_string();
    theme.theme_overlay =
        normalized_theme_name(non_empty_or(&config.ui.theme_overlay, &theme.theme)).to_string();
    theme.overlay.core.position = config.overlay.position;
    theme.overlay.core.width = config.overlay.width as f64;
    theme.overlay.core.max_text_lines = config.overlay.max_text_lines;
    theme
}

fn normalized_theme_name(value: &str) -> &str {
    match non_empty_or(value, DEFAULT_THEME_NAME) {
        LEGACY_DEFAULT_THEME_NAME => DEFAULT_THEME_NAME,
        value => value,
    }
}

fn non_empty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    let value = value.trim();
    if value.is_empty() {
        fallback
    } else {
        value
    }
}

fn load_theme_file(root: &Path, name: &str) -> Result<ThemeFile> {
    let name = normalized_theme_name(name);
    crate::config::inventory::validate_config_file_id(name)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("invalid theme id {name:?}"))?;
    let path = theme_path(root, name);
    if !path.exists() {
        if let Some(body) = crate::config::template::theme_preset_body(name) {
            return toml::from_str(body).with_context(|| format!("parse builtin theme {name}"));
        }
        if name == DEFAULT_THEME_NAME {
            return Ok(default_theme_file());
        }
    }
    let body =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    toml::from_str(&body).with_context(|| format!("parse {}", path.display()))
}

pub fn theme_path(root: &Path, name: &str) -> PathBuf {
    root.join("theme").join(format!("{name}.toml"))
}

pub fn default_theme_file() -> ThemeFile {
    toml::from_str(default_theme_template_body()).expect("builtin default theme is valid")
}

pub fn default_theme_template_body() -> &'static str {
    crate::config::template::theme_preset_body(DEFAULT_THEME_NAME)
        .expect("builtin default theme preset exists")
}

pub fn validate_theme_file(file: &ThemeFile) -> Result<()> {
    let mut tui = TuiTheme::default();
    let mut overlay = EffectiveOverlayCfg::default();
    apply_tui_theme(&mut tui, file)?;
    apply_overlay_theme(&mut overlay, file)?;
    Ok(())
}

fn apply_tui_theme(target: &mut TuiTheme, file: &ThemeFile) -> Result<()> {
    let palette = palette_for(file)?;
    set_color(
        &mut target.foreground,
        &file.tui.foreground,
        &palette,
        "tui.foreground",
    )?;
    set_color(&mut target.muted, &file.tui.muted, &palette, "tui.muted")?;
    set_color(&mut target.accent, &file.tui.accent, &palette, "tui.accent")?;
    set_color(
        &mut target.success,
        &file.tui.success,
        &palette,
        "tui.success",
    )?;
    set_color(
        &mut target.warning,
        &file.tui.warning,
        &palette,
        "tui.warning",
    )?;
    set_color(&mut target.error, &file.tui.error, &palette, "tui.error")?;
    set_color(&mut target.info, &file.tui.info, &palette, "tui.info")?;
    set_color(
        &mut target.highlight,
        &file.tui.highlight,
        &palette,
        "tui.highlight",
    )?;
    set_color(&mut target.border, &file.tui.border, &palette, "tui.border")?;
    set_color(
        &mut target.border_focus,
        &file.tui.border_focus,
        &palette,
        "tui.border_focus",
    )?;
    set_color(
        &mut target.segment,
        &file.tui.segment,
        &palette,
        "tui.segment",
    )?;
    Ok(())
}

fn apply_overlay_theme(target: &mut EffectiveOverlayCfg, file: &ThemeFile) -> Result<()> {
    let palette = palette_for(file)?;
    if let Some(value) = file.overlay.macos.glass_variant {
        target.macos.glass_variant = value;
    }
    if let Some(value) = file.overlay.macos.glass_style {
        target.macos.glass_style = value;
    }
    if let Some(value) = file.overlay.macos.subdued {
        target.macos.subdued = value;
    }
    if let Some(value) = file.overlay.macos.background_blur_radius {
        target.macos.background_blur_radius = value;
    }
    set_color(
        &mut target.core.background_rgb,
        &file.overlay.surface.background,
        &palette,
        "overlay.surface.background",
    )?;
    if let Some(value) = file.overlay.surface.background_alpha {
        target.core.background_alpha = value;
    }
    if let Some(value) = file.overlay.surface.corner_radius {
        target.core.corner_radius = value;
    }
    set_color(
        &mut target.core.text.primary,
        &file.overlay.text.primary,
        &palette,
        "overlay.text.primary",
    )?;
    set_color(
        &mut target.core.text.secondary,
        &file.overlay.text.secondary,
        &palette,
        "overlay.text.secondary",
    )?;
    set_color(
        &mut target.core.text.tertiary,
        &file.overlay.text.tertiary,
        &palette,
        "overlay.text.tertiary",
    )?;
    set_color(
        &mut target.core.text.segment,
        &file.overlay.text.segment,
        &palette,
        "overlay.text.segment",
    )?;
    set_color(
        &mut target.core.text.notice,
        &file.overlay.text.notice,
        &palette,
        "overlay.text.notice",
    )?;
    set_color(
        &mut target.core.text.error,
        &file.overlay.text.error,
        &palette,
        "overlay.text.error",
    )?;
    set_color(
        &mut target.core.state.idle,
        &file.overlay.state.idle,
        &palette,
        "overlay.state.idle",
    )?;
    set_color(
        &mut target.core.state.connecting,
        &file.overlay.state.connecting,
        &palette,
        "overlay.state.connecting",
    )?;
    set_color(
        &mut target.core.state.recording,
        &file.overlay.state.recording,
        &palette,
        "overlay.state.recording",
    )?;
    set_color(
        &mut target.core.state.thinking,
        &file.overlay.state.thinking,
        &palette,
        "overlay.state.thinking",
    )?;
    set_color(
        &mut target.core.state.stopping,
        &file.overlay.state.stopping,
        &palette,
        "overlay.state.stopping",
    )?;
    set_color(
        &mut target.core.state.error,
        &file.overlay.state.error,
        &palette,
        "overlay.state.error",
    )?;
    Ok(())
}

fn palette_for(file: &ThemeFile) -> Result<BTreeMap<String, u32>> {
    let mut palette = builtin_palette();
    for (name, value) in &file.palette {
        let ColorValue::Hex(rgb) = value else {
            anyhow::bail!("palette color {name:?} must be a 0xRRGGBB integer");
        };
        palette.insert(name.clone(), *rgb);
    }
    Ok(palette)
}

fn builtin_palette() -> BTreeMap<String, u32> {
    [
        ("fg0", palette::FG0),
        ("fg1", palette::FG1),
        ("fg2", palette::FG2),
        ("fg3", palette::FG3),
        ("fg4", palette::FG4),
        ("muted", palette::GRAY),
        ("red", palette::BRIGHT_RED),
        ("dark_red", palette::RED),
        ("orange", palette::BRIGHT_ORANGE),
        ("yellow", palette::BRIGHT_YELLOW),
        ("blue", palette::BRIGHT_BLUE),
        ("green", palette::BRIGHT_AQUA),
        ("cyan", palette::BRIGHT_AQUA),
        ("background", palette::BACKGROUND),
    ]
    .into_iter()
    .map(|(name, rgb)| (name.to_string(), rgb))
    .collect()
}

fn set_color(
    target: &mut u32,
    value: &Option<ColorValue>,
    palette: &BTreeMap<String, u32>,
    path: &str,
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    *target = resolve_color(value, palette).with_context(|| format!("resolve {path}"))?;
    Ok(())
}

fn resolve_color(value: &ColorValue, palette: &BTreeMap<String, u32>) -> Result<u32> {
    match value {
        ColorValue::Hex(rgb) => Ok(*rgb),
        ColorValue::Palette(name) => palette
            .get(name)
            .copied()
            .with_context(|| format!("unknown palette color {name:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_config() -> crate::config::Config {
        crate::config::main::parse(
            r#"
[hotkey]
trigger = "f16"
"#,
        )
        .unwrap()
    }

    #[test]
    fn default_theme_template_is_valid_and_resolves() {
        let file: ThemeFile = toml::from_str(default_theme_template_body()).unwrap();
        let mut tui = TuiTheme::default();
        let mut overlay = EffectiveOverlayCfg::default();

        apply_tui_theme(&mut tui, &file).unwrap();
        apply_overlay_theme(&mut overlay, &file).unwrap();

        assert_eq!(tui.highlight, palette::BRIGHT_AQUA);
        assert_eq!(overlay.macos.glass_variant, 11);
        assert!((overlay.core.background_alpha - 0.70).abs() < 1e-9);
        assert_eq!(overlay.core.text.primary, palette::FG0);
    }

    #[test]
    fn partial_theme_overrides_only_selected_fields() {
        let file: ThemeFile = toml::from_str(
            r#"
[palette]
accent = 0x112233

[tui]
highlight = "accent"

[overlay.text]
error = 0x445566
"#,
        )
        .unwrap();
        let mut tui = TuiTheme::default();
        let mut overlay = EffectiveOverlayCfg::default();

        apply_tui_theme(&mut tui, &file).unwrap();
        apply_overlay_theme(&mut overlay, &file).unwrap();

        assert_eq!(tui.highlight, 0x112233);
        assert_eq!(tui.foreground, palette::FG0);
        assert_eq!(overlay.core.text.error, 0x445566);
        assert_eq!(overlay.core.text.primary, palette::FG0);
    }

    #[test]
    fn missing_default_file_falls_back_to_builtin_default() {
        let dir = std::env::temp_dir().join(format!("shuohua-theme-test-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();

        let effective = load_effective_from_root(&minimal_config(), &dir).unwrap();

        assert_eq!(effective.theme, DEFAULT_THEME_NAME);
        assert_eq!(effective.overlay.core.text.primary, palette::FG0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn load_effective_report_returns_warning_when_theme_falls_back() {
        let dir = std::env::temp_dir().join(format!("shuohua-theme-test-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = crate::config::main::parse(
            r#"
[hotkey]
trigger = "f16"

[ui]
theme = "missing-theme"
"#,
        )
        .unwrap();
        let config_path = dir.join("config.toml");

        let report = load_effective_report(&cfg, &config_path);

        assert!(report.warning.is_some(), "{report:?}");
        assert_eq!(report.theme.theme, "missing-theme");
        assert_eq!(report.theme.overlay.core.text.primary, palette::FG0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_theme_file_id_returns_warning_before_reading_file() {
        let dir = std::env::temp_dir().join(format!("shuohua-theme-test-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = crate::config::main::parse(
            r#"
[hotkey]
trigger = "f16"

[ui]
theme = "Bad Theme"
"#,
        )
        .unwrap();
        let config_path = dir.join("config.toml");

        let report = load_effective_report(&cfg, &config_path);
        let warning = report.warning.unwrap().message;

        assert!(warning.contains("invalid theme id"), "{warning}");
        assert!(warning.contains("lowercase letter first"), "{warning}");
        assert!(!warning.contains("read "), "{warning}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_palette_reference_fails_theme_load() {
        let file: ThemeFile = toml::from_str(
            r#"
[tui]
highlight = "missing"
"#,
        )
        .unwrap();
        let mut tui = TuiTheme::default();

        let error = apply_tui_theme(&mut tui, &file).unwrap_err().to_string();

        assert!(error.contains("tui.highlight"), "{error}");
    }

    #[test]
    fn palette_entries_must_be_hex_values() {
        let file: ThemeFile = toml::from_str(
            r#"
[palette]
accent = "fg0"
"#,
        )
        .unwrap();

        let error = validate_theme_file(&file).unwrap_err().to_string();

        assert!(error.contains("palette color"), "{error}");
    }
}
