use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsrKind {
    Apple,
    Doubao,
    Tencent,
}

impl AsrKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AsrKind::Apple => "apple",
            AsrKind::Doubao => "doubao",
            AsrKind::Tencent => "tencent",
        }
    }

    pub fn schema_id(self) -> crate::config::schema::SchemaId {
        match self {
            AsrKind::Apple => crate::config::schema::SchemaId::AsrApple,
            AsrKind::Doubao => crate::config::schema::SchemaId::AsrDoubao,
            AsrKind::Tencent => crate::config::schema::SchemaId::AsrTencent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrInstance {
    pub id: String,
    pub kind: AsrKind,
    pub path: PathBuf,
    pub display_name: Option<String>,
}

pub fn resolve_instance(id: &str) -> anyhow::Result<AsrInstance> {
    resolve_instance_in_root(&crate::config::paths::root_dir(), id)
}

pub fn resolve_instance_in_root(root: &Path, id: &str) -> anyhow::Result<AsrInstance> {
    crate::config::inventory::validate_config_file_id(id)
        .map_err(anyhow::Error::msg)
        .map_err(|e| anyhow::anyhow!("invalid ASR instance id {id:?}: {e}"))?;
    let path = root.join("asr").join(format!("{id}.toml"));
    let body = std::fs::read_to_string(&path).map_err(|e| {
        anyhow::anyhow!(
            "ASR instance {id:?} not found at {}: {e}\nhint: create {} with a `type` field",
            path.display(),
            path.display(),
        )
    })?;
    let value: toml::Value =
        toml::from_str(&body).map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
    let kind = kind_from_value(id, &path, &value)?;
    let display_name = value
        .get("name")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(AsrInstance {
        id: id.to_string(),
        kind,
        path,
        display_name,
    })
}

pub fn kind_from_value(id: &str, path: &Path, value: &toml::Value) -> anyhow::Result<AsrKind> {
    let type_str = value.get("type").and_then(toml::Value::as_str).ok_or_else(|| {
        anyhow::anyhow!(
            "ASR instance {id:?} ({}) is missing required `type`; add `type = \"apple\"`, `type = \"doubao\"`, or `type = \"tencent\"`",
            path.display()
        )
    })?;
    match type_str {
        "apple" => Ok(AsrKind::Apple),
        "doubao" => Ok(AsrKind::Doubao),
        "tencent" => Ok(AsrKind::Tencent),
        other => anyhow::bail!(
            "unknown ASR type {other:?} in {}; expected \"apple\", \"doubao\", or \"tencent\"",
            path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("shuohua-asr-instance-{}", ulid::Ulid::new()))
    }

    #[test]
    fn resolves_typed_custom_doubao_instance() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("asr")).unwrap();
        std::fs::write(
            root.join("asr/doubao_work.toml"),
            "type = \"doubao\"\nname = \"Work\"\napp_key = \"a\"\naccess_key = \"b\"\n",
        )
        .unwrap();

        let instance = resolve_instance_in_root(&root, "doubao_work").unwrap();

        assert_eq!(instance.id, "doubao_work");
        assert_eq!(instance.kind, AsrKind::Doubao);
        assert_eq!(instance.path, root.join("asr/doubao_work.toml"));
        assert_eq!(instance.display_name.as_deref(), Some("Work"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resolves_typed_custom_tencent_instance() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("asr")).unwrap();
        std::fs::write(
            root.join("asr/tencent_work.toml"),
            "type = \"tencent\"\nname = \"Tencent Work\"\napp_id = \"1\"\nsecret_id = \"sid\"\nsecret_key = \"key\"\n",
        )
        .unwrap();

        let instance = resolve_instance_in_root(&root, "tencent_work").unwrap();

        assert_eq!(instance.id, "tencent_work");
        assert_eq!(instance.kind, AsrKind::Tencent);
        assert_eq!(instance.path, root.join("asr/tencent_work.toml"));
        assert_eq!(instance.display_name.as_deref(), Some("Tencent Work"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_speechmatics_type() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("asr")).unwrap();
        std::fs::write(
            root.join("asr/speechmatics_work.toml"),
            "type = \"speechmatics\"\nname = \"Speechmatics Work\"\napi_key = \"key\"\n",
        )
        .unwrap();

        let error = resolve_instance_in_root(&root, "speechmatics_work")
            .unwrap_err()
            .to_string();

        assert!(error.contains("unknown ASR type"), "{error}");
        assert!(error.contains("speechmatics"), "{error}");
        assert!(
            !error.contains("expected \"apple\", \"doubao\", \"speechmatics\", or \"tencent\""),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_apple_instance_without_type() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("asr")).unwrap();
        std::fs::write(root.join("asr/apple.toml"), "name = \"Apple\"\n").unwrap();

        let error = resolve_instance_in_root(&root, "apple")
            .unwrap_err()
            .to_string();

        assert!(error.contains("type"), "{error}");
        assert!(error.contains("apple.toml"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn requires_referenced_instance_file_to_exist() {
        let root = temp_root();

        let error = resolve_instance_in_root(&root, "apple")
            .unwrap_err()
            .to_string();

        assert!(error.contains("asr/apple.toml"), "{error}");
    }

    #[test]
    fn rejects_unknown_type() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("asr")).unwrap();
        std::fs::write(root.join("asr/team.toml"), "type = \"other\"\n").unwrap();

        let error = resolve_instance_in_root(&root, "team")
            .unwrap_err()
            .to_string();

        assert!(error.contains("unknown ASR type"), "{error}");
        assert!(error.contains("other"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_invalid_instance_id_before_reading_file() {
        let root = temp_root();

        let error = resolve_instance_in_root(&root, "BadName")
            .unwrap_err()
            .to_string();

        assert!(error.contains("lowercase letter first"), "{error}");
    }
}
