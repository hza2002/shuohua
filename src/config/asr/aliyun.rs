use serde::Deserialize;
use toml::value::Table;

use crate::config::asr::LocalVadMode;
use crate::config::schema::{self, SchemaId};
use crate::config::spec::validate_value;

const FUN_LANGUAGES: &[&str] = &[
    "zh", "en", "ja", "ko", "vi", "th", "id", "ms", "tl", "hi", "ar", "fr", "de", "es", "pt", "ru",
    "it", "nl", "sv", "da", "fi", "no", "el", "pl", "cs", "hu", "ro", "bg", "hr", "sk",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AliyunRegion {
    Beijing,
    Singapore,
}

impl AliyunRegion {
    pub fn endpoint(self, workspace_id: &str) -> String {
        let region = match self {
            Self::Beijing => "cn-beijing",
            Self::Singapore => "ap-southeast-1",
        };
        format!("wss://{workspace_id}.{region}.maas.aliyuncs.com/api-ws/v1/inference")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AliyunConfig {
    #[serde(default)]
    #[serde(rename = "name")]
    pub _name: Option<String>,
    pub api_key: String,
    pub workspace_id: String,
    #[serde(default = "default_region")]
    pub region: AliyunRegion,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub vocabulary_id: String,
    #[serde(default = "default_language_hints")]
    pub language_hints: Vec<String>,
    #[serde(default)]
    pub semantic_punctuation_enabled: bool,
    #[serde(default = "default_max_sentence_silence")]
    pub max_sentence_silence: u64,
    #[serde(default)]
    pub multi_threshold_mode_enabled: bool,
    #[serde(default = "default_true")]
    pub heartbeat: bool,
    #[serde(default)]
    pub speech_noise_threshold: Option<f64>,
    #[serde(default = "default_local_vad")]
    pub local_vad: LocalVadMode,
    #[serde(default = "default_open_timeout_ms")]
    pub open_timeout_ms: u64,
    #[serde(default = "default_finalize_timeout_ms")]
    pub finalize_timeout_ms: u64,
}

fn default_region() -> AliyunRegion {
    AliyunRegion::Beijing
}

pub(crate) fn default_model() -> String {
    "fun-asr-realtime".into()
}

fn default_language_hints() -> Vec<String> {
    vec!["zh".into()]
}

fn default_max_sentence_silence() -> u64 {
    1300
}

fn default_true() -> bool {
    true
}

fn default_local_vad() -> LocalVadMode {
    LocalVadMode::Auto
}

fn default_open_timeout_ms() -> u64 {
    12_000
}

fn default_finalize_timeout_ms() -> u64 {
    12_000
}

impl AliyunConfig {
    pub(crate) fn from_path_with_overrides(
        path: &std::path::Path,
        overrides: Option<&Table>,
    ) -> anyhow::Result<Self> {
        let body = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!(
                "aliyun config not found at {}: {e}\nhint: create {} and fill in api_key/workspace_id",
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
            &schema::spec_for(SchemaId::AsrAliyun),
            &value,
        ))
        .map_err(|e| anyhow::anyhow!("validate {}: {e}", path.display()))?;

        let mut cfg: Self = value
            .try_into()
            .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
        cfg.api_key = cfg.api_key.trim().to_string();
        cfg.workspace_id = cfg.workspace_id.trim().to_string();
        cfg.model = cfg.model.trim().to_string();
        cfg.vocabulary_id = cfg.vocabulary_id.trim().to_string();
        cfg.language_hints = cfg
            .language_hints
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();

        anyhow::ensure!(
            !cfg.api_key.is_empty(),
            "{}: api_key 不能为空",
            path.display()
        );
        anyhow::ensure!(
            !cfg.workspace_id.is_empty(),
            "{}: workspace_id 不能为空",
            path.display()
        );
        anyhow::ensure!(
            cfg.workspace_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "{}: workspace_id 只能包含字母、数字和连字符",
            path.display()
        );
        validate_languages(&cfg.model, &cfg.language_hints)?;
        Ok(cfg)
    }

    pub fn supports_profile_context(&self) -> bool {
        // 只有受控预设发 profile context；custom model 保守不发。
        self.model == "fun-asr-realtime"
    }
}

/// 精简的单字段校验：官方只读数组第一项，故恒限制至多一个 hint；仅对受控预设
/// `fun-asr-realtime` 校验取值合法性，custom model 交服务端判定。
fn validate_languages(model: &str, hints: &[String]) -> anyhow::Result<()> {
    anyhow::ensure!(
        hints.len() <= 1,
        "language_hints accepts at most one value; the service ignores additional values"
    );
    let Some(language) = hints.first() else {
        return Ok(());
    };
    if let Some(allowed) = supported_language_hints(model) {
        anyhow::ensure!(
            allowed.contains(&language.as_str()),
            "language hint {language:?} is not supported by model {model:?}"
        );
    }
    Ok(())
}

pub(crate) fn supported_language_hints(model: &str) -> Option<&'static [&'static str]> {
    match model {
        "fun-asr-realtime" => Some(FUN_LANGUAGES),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_language_hint_defaults_to_chinese() {
        let config: AliyunConfig = toml::from_str(
            r#"api_key = "sk-test"
workspace_id = "workspace-test"
"#,
        )
        .unwrap();

        assert_eq!(config.language_hints, vec!["zh"]);
    }

    #[test]
    fn fun_preset_only_accepts_its_language_list() {
        assert_eq!(
            supported_language_hints("fun-asr-realtime"),
            Some(FUN_LANGUAGES)
        );
        assert_eq!(supported_language_hints("paraformer-realtime-v2"), None);
        assert_eq!(supported_language_hints("future-asr-model"), None);

        assert!(validate_languages("fun-asr-realtime", &["zh".into()]).is_ok());
        assert!(validate_languages("fun-asr-realtime", &["klingon".into()]).is_err());
        // custom model：语言不受客户端约束，交服务端判定。
        assert!(validate_languages("future-asr-model", &["klingon".into()]).is_ok());
        // 恒限制至多一个 hint。
        assert!(validate_languages("future-asr-model", &["zh".into(), "en".into()]).is_err());
    }

    #[test]
    fn loads_fun_preset() {
        let dir = std::env::temp_dir().join(format!("shuohua-aliyun-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fun.toml");
        std::fs::write(
            &path,
            r#"type = "aliyun"
api_key = "sk-test"
workspace_id = "workspace-test"
model = "fun-asr-realtime"
language_hints = ["zh"]
"#,
        )
        .unwrap();

        let config = AliyunConfig::from_path_with_overrides(&path, None).unwrap();
        assert_eq!(config.model, "fun-asr-realtime");
        assert_eq!(config.language_hints, ["zh"]);
        assert!(config.supports_profile_context());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn loaded_custom_model_allows_free_language_without_family() {
        let dir = std::env::temp_dir().join(format!("shuohua-aliyun-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("custom.toml");
        std::fs::write(
            &path,
            r#"type = "aliyun"
api_key = "sk-test"
workspace_id = "workspace-test"
model = "future-asr-model"
language_hints = ["custom-language"]
"#,
        )
        .unwrap();

        let config = AliyunConfig::from_path_with_overrides(&path, None).unwrap();
        assert_eq!(config.model, "future-asr-model");
        assert_eq!(config.language_hints, ["custom-language"]);
        assert!(!config.supports_profile_context());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_legacy_paraformer_fields() {
        // Aliyun 未发版、无迁移：含 model_family / Paraformer 字段的旧文件因 schema
        // unknown-field warning 被 reject_schema_diagnostics 拒绝加载（刻意）。
        let dir = std::env::temp_dir().join(format!("shuohua-aliyun-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("legacy.toml");
        std::fs::write(
            &path,
            r#"type = "aliyun"
api_key = "sk-test"
workspace_id = "workspace-test"
model = "paraformer-realtime-v2"
model_family = "paraformer_v2"
disfluency_removal_enabled = false
"#,
        )
        .unwrap();

        assert!(AliyunConfig::from_path_with_overrides(&path, None).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
