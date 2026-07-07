use serde::Deserialize;

use crate::config::asr::LocalVadMode;
use crate::config::schema::{self, SchemaId};
use crate::config::spec::validate_value;

#[derive(Debug, Clone, Deserialize)]
pub struct AppleConfig {
    #[serde(default)]
    #[serde(rename = "name")]
    pub _name: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "default_true")]
    pub install_assets: bool,
    /// 本 provider 对本地 VAD 的覆盖策略。Apple 默认不走本地 VAD。
    #[serde(default = "default_local_vad")]
    pub local_vad: LocalVadMode,
    /// 打开 ASR session（本地框架初始化/建连）的最长等待时间（毫秒）。
    /// 这是 voice 层消费的 provider runtime option，不是 Apple 协议字段。
    #[serde(default = "default_open_timeout_ms")]
    pub open_timeout_ms: u64,
    /// 已打开 session 后，voice 发出 `is_last=true` 后最多等多久 provider 收口（毫秒）。
    /// 这是 voice 层消费的 provider runtime option，不是 Apple 协议字段。
    #[serde(default = "default_finalize_timeout_ms")]
    pub finalize_timeout_ms: u64,
}

impl Default for AppleConfig {
    fn default() -> Self {
        Self {
            _name: None,
            language: None,
            install_assets: true,
            local_vad: default_local_vad(),
            open_timeout_ms: default_open_timeout_ms(),
            finalize_timeout_ms: default_finalize_timeout_ms(),
        }
    }
}

fn default_true() -> bool {
    true
}

pub(crate) fn default_local_vad() -> LocalVadMode {
    LocalVadMode::Off
}

pub(crate) fn default_open_timeout_ms() -> u64 {
    5000
}

pub(crate) fn default_finalize_timeout_ms() -> u64 {
    5000
}

pub(crate) fn load_config_with_overrides_from_path(
    path: &std::path::Path,
    overrides: Option<&toml::value::Table>,
) -> anyhow::Result<AppleConfig> {
    let mut value = if path.exists() {
        let body = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
        toml::from_str::<toml::Value>(&body)
            .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?
    } else {
        toml::Value::Table(toml::value::Table::new())
    };

    if let Some(overrides) = overrides {
        let table = value.as_table_mut().ok_or_else(|| {
            anyhow::anyhow!("parse {}: expected top-level TOML table", path.display())
        })?;
        for (key, value) in overrides {
            table.insert(key.clone(), value.clone());
        }
    }
    crate::config::main::reject_schema_diagnostics(validate_value(
        &schema::spec_for(SchemaId::AsrApple),
        &value,
    ))
    .map_err(|e| anyhow::anyhow!("validate {}: {e}", path.display()))?;

    let mut cfg: AppleConfig = value
        .try_into()
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
    cfg.language = cfg
        .language
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Ok(cfg)
}

impl AppleConfig {
    pub(crate) fn from_path_with_overrides(
        path: &std::path::Path,
        overrides: Option<&toml::value::Table>,
    ) -> anyhow::Result<Self> {
        load_config_with_overrides_from_path(path, overrides)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn rejects_unknown_override_fields() {
        let path = std::env::temp_dir().join(format!("shuohua-apple-{}.toml", ulid::Ulid::new()));
        fs::write(&path, "type = \"apple\"\nlocal_vad = \"on\"\n").unwrap();
        let overrides = [("idle_paus".to_string(), toml::Value::Boolean(false))]
            .into_iter()
            .collect();

        let error = load_config_with_overrides_from_path(&path, Some(&overrides))
            .unwrap_err()
            .to_string();

        assert!(error.contains("idle_paus"), "{error}");
        assert!(error.contains("unknown field"), "{error}");
        let _ = fs::remove_file(path);
    }
}
