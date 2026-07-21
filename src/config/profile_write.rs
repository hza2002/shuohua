use std::path::Path;

use anyhow::{Context, Result};
use toml_edit::{value, Array, DocumentMut, Item, Table, Value};

pub(crate) fn bind_profile_route_in_document(
    doc: &mut DocumentMut,
    bundle_id: &str,
    target_profile: &str,
) -> Result<()> {
    if !doc.as_table().contains_key("profile") || !doc["profile"].is_table() {
        doc["profile"] = Item::Table(Table::new());
    }

    let default_profile = doc["profile"]
        .as_table()
        .and_then(|profile| profile.get("default"))
        .and_then(Item::as_str)
        .unwrap_or("default")
        .to_string();
    let profile = doc["profile"]
        .as_table_mut()
        .context("[profile] is not a table")?;

    for (name, item) in profile.iter_mut() {
        if name == "default" {
            continue;
        }
        let Some(array) = item.as_array_mut() else {
            continue;
        };
        remove_bundle(array, bundle_id);
    }

    if target_profile == default_profile {
        return Ok(());
    }

    if profile
        .get(target_profile)
        .and_then(Item::as_array)
        .is_none()
    {
        profile[target_profile] = value(Array::new());
    }
    let array = profile[target_profile]
        .as_array_mut()
        .with_context(|| format!("[profile].{target_profile} is not an array"))?;
    if !array.iter().any(|v| v.as_str() == Some(bundle_id)) {
        array.push(bundle_id);
    }
    Ok(())
}

pub(crate) fn bind_profile_route(
    config_path: &Path,
    bundle_id: &str,
    target_profile: &str,
) -> Result<()> {
    let profile_path = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("profile")
        .join(format!("{target_profile}.toml"));
    if !profile_path.is_file() {
        anyhow::bail!("missing profile {}", profile_path.display());
    }
    let profile_body = std::fs::read_to_string(&profile_path)
        .with_context(|| format!("read profile {}", profile_path.display()))?;
    crate::config::profile::parse(&profile_body)
        .with_context(|| format!("parse profile {}", profile_path.display()))?;

    let body = std::fs::read_to_string(config_path)
        .with_context(|| format!("read config {}", config_path.display()))?;
    let mut doc = body
        .parse::<DocumentMut>()
        .with_context(|| format!("parse config {}", config_path.display()))?;
    bind_profile_route_in_document(&mut doc, bundle_id, target_profile)?;

    let tmp = config_path.with_extension(format!("toml.tmp-{}", ulid::Ulid::generate()));
    std::fs::write(&tmp, doc.to_string())
        .with_context(|| format!("write temp config {}", tmp.display()))?;
    std::fs::rename(&tmp, config_path)
        .with_context(|| format!("replace config {}", config_path.display()))?;
    Ok(())
}

fn remove_bundle(array: &mut Array, bundle_id: &str) {
    let kept = array
        .iter()
        .filter_map(Value::as_str)
        .filter(|value| *value != bundle_id)
        .map(str::to_string)
        .collect::<Vec<_>>();
    array.clear();
    for value in kept {
        array.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(input: &str, bundle: &str, target: &str) -> String {
        let mut doc = input.parse::<DocumentMut>().unwrap();
        bind_profile_route_in_document(&mut doc, bundle, target).unwrap();
        doc.to_string()
    }

    #[test]
    fn moves_default_fallback_app_to_named_profile() {
        let out = edit(
            r#"# keep me
[profile]
default = "default"
"#,
            "com.example.App",
            "coding",
        );

        assert!(out.contains("# keep me"));
        assert!(out.contains("default = \"default\""));
        assert!(out.contains("coding = [\"com.example.App\"]"));
    }

    #[test]
    fn moves_app_between_named_profiles_and_removes_old_owner() {
        let out = edit(
            r#"[profile]
default = "default"
coding = ["com.example.App", "com.example.Other"]
chat = ["com.example.Chat"]
"#,
            "com.example.App",
            "chat",
        );

        assert!(out.contains("coding = [\"com.example.Other\"]"));
        assert!(out.contains("chat = [\"com.example.Chat\", \"com.example.App\"]"));
        assert_eq!(out.matches("com.example.App").count(), 1);
    }

    #[test]
    fn choosing_current_default_removes_explicit_routes() {
        let out = edit(
            r#"[profile]
default = "writing"
coding = ["com.example.App"]
writing = ["com.example.Other"]
"#,
            "com.example.App",
            "writing",
        );

        assert!(!out.contains("com.example.App"));
        assert!(out.contains("default = \"writing\""));
    }

    #[test]
    fn binding_is_idempotent() {
        let out = edit(
            r#"[profile]
default = "default"
coding = ["com.example.App"]
"#,
            "com.example.App",
            "coding",
        );

        assert_eq!(out.matches("com.example.App").count(), 1);
    }

    #[test]
    fn creates_missing_profile_table_and_array() {
        let out = edit(
            r#"[hotkey]
trigger = "right_command"
"#,
            "com.example.App",
            "coding",
        );

        assert!(out.contains("[profile]"));
        assert!(out.contains("coding = [\"com.example.App\"]"));
    }

    #[test]
    fn file_write_rejects_missing_target_profile() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-profile-write-{}", ulid::Ulid::generate()));
        let root = dir.join("shuohua");
        std::fs::create_dir_all(root.join("profile")).unwrap();
        std::fs::write(
            root.join("config.toml"),
            r#"[profile]
default = "default"
"#,
        )
        .unwrap();
        std::fs::write(root.join("profile/default.toml"), "name = \"default\"\n").unwrap();

        let error = bind_profile_route(&root.join("config.toml"), "com.example.App", "missing")
            .unwrap_err();

        assert!(error.to_string().contains("missing profile"), "{error:#}");
        let body = std::fs::read_to_string(root.join("config.toml")).unwrap();
        assert!(!body.contains("com.example.App"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn file_write_rejects_malformed_target_profile() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-profile-write-{}", ulid::Ulid::generate()));
        let root = dir.join("shuohua");
        std::fs::create_dir_all(root.join("profile")).unwrap();
        std::fs::write(
            root.join("config.toml"),
            r#"[profile]
default = "default"
"#,
        )
        .unwrap();
        std::fs::write(root.join("profile/default.toml"), "name = \"default\"\n").unwrap();
        std::fs::write(root.join("profile/broken.toml"), "name =").unwrap();

        let error =
            bind_profile_route(&root.join("config.toml"), "com.example.App", "broken").unwrap_err();

        assert!(error.to_string().contains("parse profile"), "{error:#}");
        let body = std::fs::read_to_string(root.join("config.toml")).unwrap();
        assert!(!body.contains("com.example.App"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
