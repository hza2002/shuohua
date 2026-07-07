use serde::Deserialize;
use toml::value::Table;

use crate::config::asr::LocalVadMode;
use crate::config::schema::{self, SchemaId};
use crate::config::spec::validate_value;

fn default_filter_modal() -> u8 {
    1
}

#[derive(Debug, Clone, Deserialize)]
pub struct TencentConfig {
    #[serde(default)]
    #[serde(rename = "name")]
    pub _name: Option<String>,
    pub app_id: String,
    pub secret_id: String,
    pub secret_key: String,
    #[serde(default = "default_engine_model_type")]
    pub engine_model_type: String,
    #[serde(default)]
    pub need_vad: bool,
    #[serde(default)]
    pub filter_dirty: u8,
    #[serde(default = "default_filter_modal")]
    pub filter_modal: u8,
    #[serde(default)]
    pub filter_punc: bool,
    #[serde(default = "default_convert_num_mode")]
    pub convert_num_mode: u8,
    #[serde(default = "default_vad_silence_time")]
    pub vad_silence_time: u64,
    #[serde(default = "default_max_speak_time")]
    pub max_speak_time: u64,
    #[serde(default)]
    pub noise_threshold: f64,
    #[serde(default = "default_hotword_weight")]
    pub hotword_weight: u8,
    #[serde(default)]
    pub hotword_id: String,
    #[serde(default)]
    pub customization_id: String,
    #[serde(default)]
    pub replace_text_id: String,
    #[serde(default)]
    pub sentence_strategy: u8,
    #[serde(default = "default_local_vad")]
    pub local_vad: LocalVadMode,
    #[serde(default = "default_open_timeout_ms")]
    pub open_timeout_ms: u64,
    #[serde(default = "default_finalize_timeout_ms")]
    pub finalize_timeout_ms: u64,
}

pub(crate) fn default_engine_model_type() -> String {
    "16k_zh".into()
}

pub(crate) fn default_convert_num_mode() -> u8 {
    1
}

pub(crate) fn default_vad_silence_time() -> u64 {
    1000
}

pub(crate) fn default_max_speak_time() -> u64 {
    60_000
}

pub(crate) fn default_hotword_weight() -> u8 {
    10
}

pub(crate) fn default_local_vad() -> LocalVadMode {
    LocalVadMode::Auto
}

pub(crate) fn default_open_timeout_ms() -> u64 {
    12_000
}

pub(crate) fn default_finalize_timeout_ms() -> u64 {
    12_000
}

pub(crate) fn load_config_with_overrides_from_path(
    path: &std::path::Path,
    overrides: Option<&Table>,
) -> anyhow::Result<TencentConfig> {
    let body = std::fs::read_to_string(path).map_err(|e| {
        anyhow::anyhow!(
            "tencent config not found at {}: {e}\n\
             hint: create {} and fill in app_id/secret_id/secret_key",
            path.display(),
            path.display(),
        )
    })?;
    let mut value: toml::Value =
        toml::from_str(&body).map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
    if let Some(overrides) = overrides {
        let table = value.as_table_mut().ok_or_else(|| {
            anyhow::anyhow!("parse {}: expected top-level TOML table", path.display())
        })?;
        for (key, value) in overrides {
            table.insert(key.clone(), value.clone());
        }
    }
    crate::config::main::reject_schema_diagnostics(validate_value(
        &schema::spec_for(SchemaId::AsrTencent),
        &value,
    ))
    .map_err(|e| anyhow::anyhow!("validate {}: {e}", path.display()))?;
    let mut cfg: TencentConfig = value
        .try_into()
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
    cfg.app_id = cfg.app_id.trim().to_string();
    cfg.secret_id = cfg.secret_id.trim().to_string();
    cfg.secret_key = cfg.secret_key.trim().to_string();
    cfg.engine_model_type = cfg.engine_model_type.trim().to_string();
    cfg.hotword_id = cfg.hotword_id.trim().to_string();
    cfg.customization_id = cfg.customization_id.trim().to_string();
    cfg.replace_text_id = cfg.replace_text_id.trim().to_string();
    if cfg.app_id.is_empty() || cfg.secret_id.is_empty() || cfg.secret_key.is_empty() {
        anyhow::bail!(
            "{}: app_id / secret_id / secret_key 为空。从腾讯云 API 密钥管理页面获取后填入",
            path.display()
        );
    }
    if cfg.engine_model_type.is_empty() {
        anyhow::bail!("{}: engine_model_type 不能为空", path.display());
    }
    validate_provider_ranges(&cfg, path)?;
    Ok(cfg)
}

fn validate_provider_ranges(cfg: &TencentConfig, path: &std::path::Path) -> anyhow::Result<()> {
    if !matches!(cfg.convert_num_mode, 0 | 1 | 3) {
        anyhow::bail!("{}: convert_num_mode must be 0, 1, or 3", path.display());
    }
    if !matches!(cfg.hotword_weight, 1..=11 | 100) {
        anyhow::bail!(
            "{}: hotword_weight must be between 1 and 11, or 100",
            path.display()
        );
    }
    Ok(())
}

impl TencentConfig {
    pub(crate) fn from_path_with_overrides(
        path: &std::path::Path,
        overrides: Option<&Table>,
    ) -> anyhow::Result<Self> {
        load_config_with_overrides_from_path(path, overrides)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn defaults_to_free_chinese_engine() {
        let cfg: TencentConfig = toml::from_str(
            r#"
type = "tencent"
app_id = "1250000000"
secret_id = "sid"
secret_key = "key"
"#,
        )
        .unwrap();

        assert_eq!(cfg.engine_model_type, "16k_zh");
        assert!(!cfg.need_vad);
        assert_eq!(cfg.filter_dirty, 0);
        assert_eq!(cfg.filter_modal, 1);
        assert!(!cfg.filter_punc);
        assert_eq!(cfg.convert_num_mode, 1);
        assert_eq!(cfg.vad_silence_time, 1000);
        assert_eq!(cfg.max_speak_time, 60_000);
        assert_eq!(cfg.noise_threshold, 0.0);
        assert_eq!(cfg.hotword_weight, 10);
        assert_eq!(cfg.sentence_strategy, 0);
    }

    #[test]
    fn allows_any_documented_or_new_engine_model_type() {
        let cfg: TencentConfig = toml::from_str(
            r#"
type = "tencent"
app_id = "1250000000"
secret_id = "sid"
secret_key = "key"
engine_model_type = "16k_multi_lang"
"#,
        )
        .unwrap();

        assert_eq!(cfg.engine_model_type, "16k_multi_lang");
    }

    #[test]
    fn rejects_provider_values_outside_safe_ranges() {
        let path = std::env::temp_dir().join(format!("shuohua-tencent-{}.toml", ulid::Ulid::new()));
        fs::write(
            &path,
            r#"
type = "tencent"
app_id = "1250000000"
secret_id = "sid"
secret_key = "key"
hotword_weight = 12
"#,
        )
        .unwrap();

        let error = load_config_with_overrides_from_path(&path, None)
            .unwrap_err()
            .to_string();

        assert!(error.contains("hotword_weight"), "{error}");
        assert!(error.contains("1 and 11"), "{error}");
        let _ = fs::remove_file(path);
    }
}
