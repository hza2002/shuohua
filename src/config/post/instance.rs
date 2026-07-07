use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostKind {
    Rule,
    Llm,
}

impl PostKind {
    pub fn schema_id(self) -> crate::config::schema::SchemaId {
        match self {
            PostKind::Rule => crate::config::schema::SchemaId::PostRule,
            PostKind::Llm => crate::config::schema::SchemaId::PostLlm,
        }
    }
}

/// Resolve a bare post component id to its kind under `root/post/<id>.toml`.
/// Returns `None` on missing file, unreadable, unparseable, or missing/invalid `type`
/// — callers that only need "is this an llm component?" treat all failures as "not llm".
pub fn resolve_kind_in_root(root: &std::path::Path, id: &str) -> Option<PostKind> {
    let path = root.join("post").join(format!("{id}.toml"));
    let body = std::fs::read_to_string(&path).ok()?;
    let value: toml::Value = body.parse().ok()?;
    kind_from_value(id, &path, &value).ok()
}

pub fn kind_from_value(id: &str, path: &Path, value: &toml::Value) -> anyhow::Result<PostKind> {
    let type_str = value.get("type").and_then(toml::Value::as_str).ok_or_else(|| {
        anyhow::anyhow!(
            "post component {id:?} ({}) is missing required `type`; add `type = \"rule\"` or `type = \"llm\"`",
            path.display()
        )
    })?;
    match type_str {
        "rule" => Ok(PostKind::Rule),
        "llm" => Ok(PostKind::Llm),
        other => anyhow::bail!(
            "unknown post type {other:?} in {}; expected \"rule\" or \"llm\"",
            path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn reads_rule_and_llm_type() {
        let path = PathBuf::from("post/zh_filter.toml");
        let rule = "type = \"rule\"\npatterns = []\n"
            .parse::<toml::Value>()
            .unwrap();
        assert_eq!(
            kind_from_value("zh_filter", &path, &rule).unwrap(),
            PostKind::Rule
        );

        let llm = "type = \"llm\"\n".parse::<toml::Value>().unwrap();
        assert_eq!(
            kind_from_value("deepseek", &path, &llm).unwrap(),
            PostKind::Llm
        );
    }

    #[test]
    fn rejects_missing_type() {
        let path = PathBuf::from("post/zh_filter.toml");
        let value = "patterns = []\n".parse::<toml::Value>().unwrap();

        let error = kind_from_value("zh_filter", &path, &value)
            .unwrap_err()
            .to_string();

        assert!(error.contains("type"), "{error}");
        assert!(error.contains("zh_filter.toml"), "{error}");
    }

    #[test]
    fn rejects_unknown_type() {
        let path = PathBuf::from("post/team.toml");
        let value = "type = \"other\"\n".parse::<toml::Value>().unwrap();

        let error = kind_from_value("team", &path, &value)
            .unwrap_err()
            .to_string();

        assert!(error.contains("unknown post type"), "{error}");
        assert!(error.contains("other"), "{error}");
    }
}
