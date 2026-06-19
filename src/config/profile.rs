use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use toml::value::Table;

use crate::config::schema::{self, SchemaId};
use crate::config::spec::validate_value;
use crate::config::ProfileRouteCfg;

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub name: String,
    pub asr: ProfileAsr,
    #[serde(default)]
    pub post: ProfilePost,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfileAsr {
    pub provider: String,
    #[serde(default)]
    pub hotwords: Vec<String>,
    #[serde(flatten)]
    pub overrides: Table,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfilePost {
    #[serde(default)]
    pub chain: Vec<String>,
    #[serde(default)]
    pub llm: Table,
}

impl Default for ProfilePost {
    fn default() -> Self {
        Self {
            chain: Vec::new(),
            llm: Table::new(),
        }
    }
}

pub fn default_dir() -> PathBuf {
    crate::config::paths::profile_dir()
}

pub fn load_for_app(
    profile_dir: &Path,
    routes: &ProfileRouteCfg,
    bundle_id: Option<&str>,
) -> Result<Profile> {
    let path = profile_path_for_routes(profile_dir, routes, bundle_id)?;
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("read profile {}", path.display()))?;
    parse(&body).with_context(|| format!("parse profile {}", path.display()))
}

pub fn parse(body: &str) -> Result<Profile> {
    let value = toml::from_str::<toml::Value>(body)?;
    crate::config::main::reject_schema_diagnostics(validate_value(
        &schema::spec_for(SchemaId::Profile),
        &value,
    ))?;
    value.try_into::<Profile>().map_err(Into::into)
}

fn profile_path_for_routes(
    profile_dir: &Path,
    routes: &ProfileRouteCfg,
    bundle_id: Option<&str>,
) -> Result<PathBuf> {
    let profile_name = resolve_profile_name(routes, bundle_id)?;
    Ok(profile_dir.join(format!("{profile_name}.toml")))
}

fn resolve_profile_name(routes: &ProfileRouteCfg, bundle_id: Option<&str>) -> Result<String> {
    let Some(bundle_id) = bundle_id else {
        return Ok(routes.default.clone());
    };
    let matches = routes
        .routes
        .iter()
        .filter_map(|(profile, apps)| apps.iter().any(|app| app == bundle_id).then_some(profile))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(routes.default.clone()),
        [profile] => Ok((*profile).clone()),
        _ => anyhow::bail!("bundle id {bundle_id:?} matches multiple profiles: {matches:?}"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shuohua-profile-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn profile_routes_fall_back_to_default() {
        let dir = temp_dir();
        fs::write(
            dir.join("default.toml"),
            r#"
name = "default"
[asr]
provider = "doubao"
hotwords = ["Rust"]

[post]
chain = ["rule:zh_filter", "llm:deepseek"]
"#,
        )
        .unwrap();

        let routes = ProfileRouteCfg::default();
        let profile = load_for_app(&dir, &routes, Some("com.example.Missing")).unwrap();

        assert_eq!(profile.name, "default");
        assert_eq!(profile.asr.provider, "doubao");
        assert_eq!(profile.asr.hotwords, vec!["Rust"]);
        assert_eq!(profile.post.chain, vec!["rule:zh_filter", "llm:deepseek"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn profile_routes_select_named_profile() {
        let dir = temp_dir();
        fs::write(
            dir.join("default.toml"),
            r#"
name = "default"
[asr]
provider = "doubao"

[post]
chain = ["rule:zh_filter"]
"#,
        )
        .unwrap();
        fs::write(
            dir.join("agent.toml"),
            r#"
name = "agent"
[asr]
provider = "doubao"

[post]
chain = ["llm:deepseek"]

[post.llm.deepseek]
system_prompt = "app prompt"
"#,
        )
        .unwrap();

        let routes = ProfileRouteCfg {
            default: "default".to_string(),
            routes: [("agent".to_string(), vec!["com.example.App".to_string()])]
                .into_iter()
                .collect(),
        };
        let profile = load_for_app(&dir, &routes, Some("com.example.App")).unwrap();

        assert_eq!(profile.name, "agent");
        assert_eq!(profile.post.chain, vec!["llm:deepseek"]);
        assert_eq!(
            profile
                .post
                .llm
                .get("deepseek")
                .unwrap()
                .get("system_prompt")
                .and_then(toml::Value::as_str),
            Some("app prompt")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn profile_routes_select_named_profile_and_fall_back_to_default() {
        let dir = temp_dir();
        fs::write(
            dir.join("default.toml"),
            r#"
name = "default"
[asr]
provider = "doubao"

[post]
chain = []
"#,
        )
        .unwrap();
        fs::write(
            dir.join("agent.toml"),
            r#"
name = "agent"
[asr]
provider = "apple"

[post]
chain = []
"#,
        )
        .unwrap();
        let routes = ProfileRouteCfg {
            default: "default".to_string(),
            routes: [(
                "agent".to_string(),
                vec!["com.mitchellh.ghostty".to_string()],
            )]
            .into_iter()
            .collect(),
        };

        let agent = load_for_app(&dir, &routes, Some("com.mitchellh.ghostty")).unwrap();
        let default = load_for_app(&dir, &routes, Some("com.example.Other")).unwrap();

        assert_eq!(agent.name, "agent");
        assert_eq!(agent.asr.provider, "apple");
        assert_eq!(default.name, "default");
        assert_eq!(default.asr.provider, "doubao");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn profile_routes_reject_duplicate_app_matches() {
        let dir = temp_dir();
        let routes = ProfileRouteCfg {
            default: "default".to_string(),
            routes: [
                (
                    "agent".to_string(),
                    vec!["com.mitchellh.ghostty".to_string()],
                ),
                (
                    "coding".to_string(),
                    vec!["com.mitchellh.ghostty".to_string()],
                ),
            ]
            .into_iter()
            .collect(),
        };

        let err = load_for_app(&dir, &routes, Some("com.mitchellh.ghostty"))
            .unwrap_err()
            .to_string();

        assert!(err.contains("matches multiple profiles"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parse_rejects_unknown_profile_fields() {
        let err = parse(
            r#"
name = "default"
unknown = true

[asr]
provider = "apple"
"#,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("unknown"), "{err}");
        assert!(err.contains("unknown field"), "{err}");
    }

    #[test]
    fn parse_allows_asr_provider_overrides() {
        let profile = parse(
            r#"
name = "default"

[asr]
provider = "apple"
language = "zh-CN"
idle_pause = true
finalize_timeout_ms = 5000
"#,
        )
        .unwrap();

        assert_eq!(profile.asr.provider, "apple");
        assert_eq!(
            profile
                .asr
                .overrides
                .get("language")
                .and_then(toml::Value::as_str),
            Some("zh-CN")
        );
    }
}
