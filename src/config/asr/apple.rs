use std::path::PathBuf;

use serde::Deserialize;

use crate::config::schema::{self, SchemaId};
use crate::config::spec::validate_value;

#[derive(Debug, Clone, Deserialize)]
pub struct AppleConfig {
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "default_true")]
    pub install_assets: bool,
    /// 允许 voice 层用本地 VAD 切分本 provider 的 session。默认关。
    #[serde(default)]
    pub idle_pause: bool,
    /// voice 发出 `is_last=true` 后最多等多久 provider finalize（毫秒）。
    #[serde(default = "default_finalize_timeout_ms")]
    pub finalize_timeout_ms: u64,
}

impl Default for AppleConfig {
    fn default() -> Self {
        Self {
            language: None,
            install_assets: true,
            idle_pause: false,
            finalize_timeout_ms: default_finalize_timeout_ms(),
        }
    }
}

fn default_true() -> bool {
    true
}

pub(crate) fn default_finalize_timeout_ms() -> u64 {
    5000
}

pub fn config_path() -> PathBuf {
    crate::config::paths::asr_provider("apple")
}

pub fn load_config_with_overrides(
    overrides: Option<&toml::value::Table>,
) -> anyhow::Result<AppleConfig> {
    load_config_with_overrides_from_path(&config_path(), overrides)
}

fn load_config_with_overrides_from_path(
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn rejects_unknown_override_fields() {
        let path = std::env::temp_dir().join(format!("shuohua-apple-{}.toml", ulid::Ulid::new()));
        fs::write(&path, "idle_pause = true\n").unwrap();
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
