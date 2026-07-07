#![cfg_attr(not(test), allow(dead_code))]

use std::path::Path;

use crate::config::spec::{ConfigSpec, FieldSpec, ValueKind};

#[derive(Debug, Clone, PartialEq)]
pub enum ControlKind {
    Toggle,
    Select(Vec<String>),
    Number {
        min: Option<f64>,
        max: Option<f64>,
        float: bool,
    },
    Text,
    MultilineText,
    Array,
    KeyCapture,
    ReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldOrigin {
    Set,
    Default,
    RequiredUnset,
    /// Value present in the profile override but invalid for the resolved
    /// provider/component schema (e.g. a stale `app_key` after switching to
    /// an Apple instance), or a dangling chain member.
    // produced by ProfileComposer (C1 Task 3)
    #[allow(dead_code)]
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldView {
    pub field_path: String,
    pub effective: String,
    pub default_value: String,
    pub origin: FieldOrigin,
    pub control: ControlKind,
    pub secret: bool,
    pub editable: bool,
    /// 是否可「重置为默认」：删掉这个键后文件仍合法。required 且无默认值的字段
    /// 删掉会让文件缺必填项，故不可重置。
    pub can_unset: bool,
    pub description_key: Option<&'static str>,
}

pub fn control_for(field: &FieldSpec, dynamic: Option<Vec<String>>) -> ControlKind {
    if field.name() == "type" {
        return ControlKind::ReadOnly;
    }
    if field.is_keycapture() {
        return ControlKind::KeyCapture;
    }
    if field.is_multiline() {
        return ControlKind::MultilineText;
    }
    match field.kind() {
        ValueKind::Table | ValueKind::FreeTable => ControlKind::ReadOnly,
        ValueKind::Array => ControlKind::Array,
        ValueKind::Bool => ControlKind::Toggle,
        ValueKind::Enum => ControlKind::Select(field.allowed().to_vec()),
        ValueKind::Integer => ControlKind::Number {
            min: field.numeric_min(),
            max: field.numeric_max(),
            float: false,
        },
        ValueKind::Float => ControlKind::Number {
            min: field.numeric_min(),
            max: field.numeric_max(),
            float: true,
        },
        ValueKind::Color => ControlKind::Text,
        ValueKind::String => match dynamic {
            Some(opts) => ControlKind::Select(opts),
            None => ControlKind::Text,
        },
    }
}

/// 运行期可枚举域。
pub fn dynamic_domain(rel_path: &str, field_path: &str, config_root: &Path) -> Option<Vec<String>> {
    if rel_path.starts_with("profile/") && field_path == "asr.instance" {
        return Some(available_file_ids(&config_root.join("asr")));
    }
    if rel_path != "config.toml" {
        return None;
    }
    match field_path {
        "ui.language" => Some(
            [
                "auto", "zh-CN", "en-US", "zh-Hant", "zh-TW", "zh-HK", "pseudo",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        ),
        "ui.theme" => Some(available_theme_ids(config_root)),
        "ui.theme_tui" | "ui.theme_overlay" => {
            let mut opts = vec![String::new()]; // 空 = 跟随 ui.theme
            opts.extend(available_theme_ids(config_root));
            Some(opts)
        }
        _ => None,
    }
}

fn available_theme_ids(config_root: &Path) -> Vec<String> {
    let mut ids: Vec<String> = crate::config::template::theme_presets()
        .iter()
        .map(|preset| preset.id.to_string())
        .collect();
    if let Ok(entries) = std::fs::read_dir(config_root.join("theme")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    ids.push(stem.to_string());
                }
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn available_file_ids(dir: &Path) -> Vec<String> {
    let mut ids = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    ids.push(stem.to_string());
                }
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

pub fn field_views(
    rel_path: &str,
    spec: &ConfigSpec,
    parsed: &toml::Value,
    config_root: &Path,
) -> Vec<FieldView> {
    let mut views = Vec::new();
    for field in spec.fields() {
        if matches!(field.kind(), ValueKind::Table | ValueKind::FreeTable) {
            continue;
        }
        let dynamic = dynamic_domain(rel_path, field.name(), config_root);
        let control = control_for(field, dynamic);
        let present = value_at(parsed, field.name());
        let derived_name = (field.name() == "name").then(|| display_name_from_rel_path(rel_path));
        let (effective, origin) = match present {
            Some(value) => {
                let displayed = if field.kind() == ValueKind::Array {
                    display_array_value(value)
                } else {
                    field.display_value(value)
                };
                if derived_name.as_deref() == Some(displayed.as_str()) || displayed.is_empty() {
                    (
                        derived_name.clone().unwrap_or(displayed),
                        FieldOrigin::Default,
                    )
                } else if derived_name.is_some() {
                    (displayed, FieldOrigin::Set)
                } else {
                    let is_default = field
                        .default_value()
                        .is_some_and(|d| d == displayed.as_str());
                    if is_default {
                        (displayed, FieldOrigin::Default)
                    } else {
                        (displayed, FieldOrigin::Set)
                    }
                }
            }
            None => match field.default_value() {
                Some(default) => (default.to_string(), FieldOrigin::Default),
                None if let Some(derived) = derived_name => (derived, FieldOrigin::Default),
                None if field.required_without_default() => {
                    (String::new(), FieldOrigin::RequiredUnset)
                }
                None => (String::new(), FieldOrigin::Default),
            },
        };
        let editable = control != ControlKind::ReadOnly;
        let default = field.default_value().unwrap_or("").to_string();
        views.push(FieldView {
            field_path: field.name().to_string(),
            effective,
            default_value: default,
            origin,
            control,
            secret: field.is_secret(),
            editable,
            can_unset: !field.required_without_default(),
            description_key: field.description_key_value(),
        });
    }
    views
}

fn value_at<'a>(value: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.as_table()?.get(part)?;
    }
    Some(current)
}

fn display_name_from_rel_path(rel_path: &str) -> String {
    let stem = Path::new(rel_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    display_name_from_stem(stem)
}

/// Title-case a file stem into a display name (`my-profile` → `My Profile`).
/// Shared by field-view rendering and `Profile::display_name`.
pub(crate) fn display_name_from_stem(stem: &str) -> String {
    stem.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn display_array_value(value: &toml::Value) -> String {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(toml::Value::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{spec_for, SchemaId};

    #[test]
    fn views_show_full_schema_with_defaults_and_origin() {
        let spec = spec_for(SchemaId::Main);
        let parsed: toml::Value = toml::toml! {
            [hotkey]
            trigger = "f16"
            [overlay]
            position = "top"
        }
        .into();

        let views = field_views("config.toml", &spec, &parsed, Path::new("/tmp/shuohua"));

        let position = views
            .iter()
            .find(|v| v.field_path == "overlay.position")
            .unwrap();
        assert_eq!(position.origin, FieldOrigin::Set);
        assert_eq!(position.effective, "top");
        assert!(matches!(position.control, ControlKind::Select(_)));
        assert!(position.editable);

        let threshold = views
            .iter()
            .find(|v| v.field_path == "voice.vad.threshold")
            .unwrap();
        assert_eq!(threshold.origin, FieldOrigin::Default);
        assert_eq!(threshold.effective, "0.5");
        assert!(matches!(
            threshold.control,
            ControlKind::Number { float: true, .. }
        ));

        let trigger = views
            .iter()
            .find(|v| v.field_path == "hotkey.trigger")
            .unwrap();
        assert_eq!(trigger.origin, FieldOrigin::Set);
        assert!(matches!(trigger.control, ControlKind::KeyCapture));
    }

    #[test]
    fn language_and_theme_are_dynamic_selects() {
        let spec = spec_for(SchemaId::Main);
        let parsed: toml::Value = toml::toml! { [hotkey] trigger = "f16" }.into();
        let views = field_views("config.toml", &spec, &parsed, Path::new("/tmp/shuohua"));

        let lang = views
            .iter()
            .find(|v| v.field_path == "ui.language")
            .unwrap();
        match &lang.control {
            ControlKind::Select(opts) => assert!(opts.contains(&"auto".to_string())),
            other => panic!("expected dynamic select, got {other:?}"),
        }
        let theme = views.iter().find(|v| v.field_path == "ui.theme").unwrap();
        match &theme.control {
            ControlKind::Select(opts) => {
                assert!(opts.contains(&crate::config::theme::DEFAULT_THEME_NAME.to_string()))
            }
            other => panic!("expected dynamic select, got {other:?}"),
        }
    }

    #[test]
    fn asr_provider_choice_fields_are_schema_selects() {
        let apple_value: toml::Value = toml::toml! { type = "apple" }.into();
        let apple = field_views(
            "asr/apple.toml",
            &spec_for(SchemaId::AsrApple),
            &apple_value,
            Path::new("/tmp/shuohua"),
        );
        let apple_language = apple.iter().find(|v| v.field_path == "language").unwrap();
        match &apple_language.control {
            ControlKind::Select(opts) => assert!(opts.contains(&"zh-CN".to_string())),
            other => panic!("expected Apple language select, got {other:?}"),
        }

        let doubao_value: toml::Value = toml::toml! {
            type = "doubao"
            app_key = "app"
            access_key = "access"
        }
        .into();
        let doubao = field_views(
            "asr/doubao.toml",
            &spec_for(SchemaId::AsrDoubao),
            &doubao_value,
            Path::new("/tmp/shuohua"),
        );
        let doubao_language = doubao.iter().find(|v| v.field_path == "language").unwrap();
        match &doubao_language.control {
            ControlKind::Select(opts) => assert!(opts.contains(&"auto".to_string())),
            other => panic!("expected Doubao language select, got {other:?}"),
        }

        let tencent_value: toml::Value = toml::toml! {
            type = "tencent"
            app_id = "1250000000"
            secret_id = "sid"
            secret_key = "key"
        }
        .into();
        let tencent = field_views(
            "asr/tencent.toml",
            &spec_for(SchemaId::AsrTencent),
            &tencent_value,
            Path::new("/tmp/shuohua"),
        );
        let engine = tencent
            .iter()
            .find(|v| v.field_path == "engine_model_type")
            .unwrap();
        match &engine.control {
            ControlKind::Select(opts) => assert!(opts.contains(&"16k_multi_lang".to_string())),
            other => panic!("expected Tencent engine select, got {other:?}"),
        }
    }

    #[test]
    fn profile_asr_provider_uses_available_asr_files() {
        let root = std::env::temp_dir().join(format!("shuohua-fieldview-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(root.join("asr")).unwrap();
        std::fs::write(root.join("asr/apple.toml"), "").unwrap();
        std::fs::write(root.join("asr/team.toml"), "").unwrap();

        let opts = dynamic_domain("profile/default.toml", "asr.instance", &root).unwrap();

        assert_eq!(opts, vec!["apple".to_string(), "team".to_string()]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn arrays_are_editable_and_type_is_read_only() {
        let profile = spec_for(SchemaId::Profile);
        let hotwords = profile.field_for_path("asr.hotwords").unwrap();
        assert_eq!(control_for(hotwords, None), ControlKind::Array);

        let post = spec_for(SchemaId::PostLlm);
        let type_field = post.field_for_path("type").unwrap();
        assert_eq!(control_for(type_field, None), ControlKind::ReadOnly);
    }

    #[test]
    fn container_fields_are_skipped_scalar_fields_editable() {
        let spec = spec_for(SchemaId::Profile);
        let parsed: toml::Value = toml::toml! {
            name = "default"
            [asr]
            instance = "doubao"
        }
        .into();
        let views = field_views(
            "profile/default.toml",
            &spec,
            &parsed,
            Path::new("/tmp/shuohua"),
        );

        assert!(
            views.iter().all(|v| v.field_path != "asr"),
            "table container not a row"
        );
        let provider = views
            .iter()
            .find(|v| v.field_path == "asr.instance")
            .unwrap();
        assert!(provider.editable, "non-main scalar field is editable");
    }

    #[test]
    fn missing_optional_name_displays_title_from_file_stem_as_default() {
        let spec = spec_for(SchemaId::PostRule);
        let parsed: toml::Value = toml::toml! {
            type = "rule"
            patterns = []
        }
        .into();

        let views = field_views(
            "post/my-profile.toml",
            &spec,
            &parsed,
            Path::new("/tmp/shuohua"),
        );

        let name = views.iter().find(|v| v.field_path == "name").unwrap();
        assert_eq!(name.origin, FieldOrigin::Default);
        assert_eq!(name.effective, "My Profile");
    }

    #[test]
    fn profile_without_name_displays_title_from_file_stem_as_default() {
        let spec = spec_for(SchemaId::Profile);
        let parsed: toml::Value = toml::toml! {
            [asr]
            instance = "apple"
        }
        .into();

        let views = field_views(
            "profile/default.toml",
            &spec,
            &parsed,
            Path::new("/tmp/shuohua"),
        );

        let name = views.iter().find(|v| v.field_path == "name").unwrap();
        assert_eq!(name.origin, FieldOrigin::Default);
        assert_eq!(name.effective, "Default");
    }

    #[test]
    fn multiline_and_keycapture_controls_derive_from_flags() {
        let spec = spec_for(SchemaId::PostLlm);
        let parsed: toml::Value = toml::toml! {
            type = "llm"
            name = "x"
            api_key = "k"
            model = "m"
            prompt = "line1\nline2"
        }
        .into();
        let views = field_views("post/x.toml", &spec, &parsed, Path::new("/tmp/shuohua"));
        let prompt = views.iter().find(|v| v.field_path == "prompt").unwrap();
        assert!(matches!(prompt.control, ControlKind::MultilineText));

        let main = spec_for(SchemaId::Main);
        let parsed2: toml::Value = toml::toml! { [hotkey] trigger = "f16" }.into();
        let mviews = field_views("config.toml", &main, &parsed2, Path::new("/tmp/shuohua"));
        let trigger = mviews
            .iter()
            .find(|v| v.field_path == "hotkey.trigger")
            .unwrap();
        assert!(matches!(trigger.control, ControlKind::KeyCapture));
    }
}
