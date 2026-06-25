use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use toml::value::Table;

use crate::config::schema::{self, SchemaId};
use crate::config::spec::validate_value;
use crate::config::{AppIdentity, ProfileRouteCfg};

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
    app: &AppIdentity<'_>,
) -> Result<Profile> {
    let path = profile_path_for_routes(profile_dir, routes, app)?;
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
    app: &AppIdentity<'_>,
) -> Result<PathBuf> {
    let profile_name = resolve_profile_name(routes, app)?;
    Ok(profile_dir.join(format!("{profile_name}.toml")))
}

fn resolve_profile_name(routes: &ProfileRouteCfg, app: &AppIdentity<'_>) -> Result<String> {
    let matches = routes.matching_profiles(app);
    match matches.as_slice() {
        [] => Ok(routes.default.clone()),
        [profile] => Ok((*profile).to_string()),
        _ => anyhow::bail!("app identity matches multiple profiles: {matches:?}"),
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
        let profile = load_for_app(
            &dir,
            &routes,
            &AppIdentity::macos(Some("com.example.Missing")),
        )
        .unwrap();

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
            routes: crate::config::ProfileRoutes::from_iter([(
                "agent".to_string(),
                crate::config::ProfileRouteMatchers {
                    macos: crate::config::MacosProfileMatchers {
                        bundle_id: vec!["com.example.App".to_string()],
                    },
                    ..Default::default()
                },
            )]),
        };
        let profile =
            load_for_app(&dir, &routes, &AppIdentity::macos(Some("com.example.App"))).unwrap();

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
            routes: crate::config::ProfileRoutes::from_iter([(
                "agent".to_string(),
                crate::config::ProfileRouteMatchers {
                    macos: crate::config::MacosProfileMatchers {
                        bundle_id: vec!["com.mitchellh.ghostty".to_string()],
                    },
                    ..Default::default()
                },
            )]),
        };

        let agent = load_for_app(
            &dir,
            &routes,
            &AppIdentity::macos(Some("com.mitchellh.ghostty")),
        )
        .unwrap();
        let default = load_for_app(
            &dir,
            &routes,
            &AppIdentity::macos(Some("com.example.Other")),
        )
        .unwrap();

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
            routes: crate::config::ProfileRoutes::from_iter([
                (
                    "agent".to_string(),
                    crate::config::ProfileRouteMatchers {
                        macos: crate::config::MacosProfileMatchers {
                            bundle_id: vec!["com.mitchellh.ghostty".to_string()],
                        },
                        ..Default::default()
                    },
                ),
                (
                    "coding".to_string(),
                    crate::config::ProfileRouteMatchers {
                        macos: crate::config::MacosProfileMatchers {
                            bundle_id: vec!["com.mitchellh.ghostty".to_string()],
                        },
                        ..Default::default()
                    },
                ),
            ]),
        };

        let err = load_for_app(
            &dir,
            &routes,
            &AppIdentity::macos(Some("com.mitchellh.ghostty")),
        )
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
