use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use toml::value::Table;

use crate::config::schema::{self, SchemaId};
use crate::config::spec::validate_value;
use crate::config::ProfileRouteCfg;

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    /// File stem of the profile (e.g. `default` for `profile/default.toml`).
    /// Not part of the TOML; populated by path-aware loaders and used for the
    /// stem-derived display-name fallback when `name` is unset.
    #[serde(skip)]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub asr: ProfileAsr,
    #[serde(default)]
    pub post: ProfilePost,
}

impl Profile {
    /// Human-readable name: explicit `name`, else a title-cased stem of `id`.
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| crate::config::field_view::display_name_from_stem(&self.id))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfileAsr {
    pub instance: String,
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
    pub overrides: Table,
}

impl Default for ProfilePost {
    fn default() -> Self {
        Self {
            chain: Vec::new(),
            overrides: Table::new(),
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
    let mut profile = parse(&body).with_context(|| format!("parse profile {}", path.display()))?;
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        profile.id = stem.to_string();
    }
    Ok(profile)
}

pub fn parse(body: &str) -> Result<Profile> {
    let value = toml::from_str::<toml::Value>(body)?;
    crate::config::main::reject_schema_diagnostics(validate_value(
        &schema::spec_for(SchemaId::Profile),
        &value,
    ))?;
    value.try_into::<Profile>().map_err(Into::into)
}

pub(crate) fn create_profile_file(
    config_root: &Path,
    profile_id: &str,
    display_name: &str,
    asr_instance: &str,
) -> Result<PathBuf> {
    crate::config::inventory::validate_config_file_id(profile_id).map_err(anyhow::Error::msg)?;
    let profile_dir = config_root.join("profile");
    std::fs::create_dir_all(&profile_dir)
        .with_context(|| format!("create profile dir {}", profile_dir.display()))?;
    let path = profile_dir.join(format!("{profile_id}.toml"));
    // Empty name, or a name equal to the id, falls back to the stem-derived
    // display name — no explicit `name` line is written in that case.
    let display_name = display_name.trim();
    let name_line = if display_name.is_empty() || display_name == profile_id {
        String::new()
    } else {
        format!("name = {display_name:?}\n\n")
    };
    let body = format!("{name_line}[asr]\ninstance = {asr_instance:?}\n\n[post]\nchain = []\n");
    parse(&body).context("render new profile")?;

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("create profile {}; file already exists", path.display()))?;
    file.write_all(body.as_bytes())
        .with_context(|| format!("write profile {}", path.display()))?;
    Ok(path)
}

/// Read all profiles and return the file stems (sorted) of those matching `pred`.
/// Profiles that cannot be parsed are skipped — bad profiles are handled by diagnostics,
/// not by blocking delete operations.
fn profiles_matching(config_root: &Path, pred: impl Fn(&Profile) -> bool) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(config_root.join("profile")) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(profile) = toml::from_str::<Profile>(&body) else {
            continue;
        };
        if pred(&profile) {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    names
}

pub(crate) fn delete_asr_instance_file(config_root: &Path, id: &str) -> Result<PathBuf> {
    crate::config::inventory::validate_config_file_id(id).map_err(anyhow::Error::msg)?;
    let refs = profiles_matching(config_root, |p| p.asr.instance == id);
    anyhow::ensure!(
        refs.is_empty(),
        "cannot delete ASR instance {id:?}; referenced by profile(s): {}",
        refs.join(", ")
    );
    let path = config_root.join("asr").join(format!("{id}.toml"));
    std::fs::remove_file(&path)
        .with_context(|| format!("delete ASR instance {}", path.display()))?;
    Ok(path)
}

pub(crate) fn delete_post_component_file(config_root: &Path, id: &str) -> Result<PathBuf> {
    crate::config::inventory::validate_config_file_id(id).map_err(anyhow::Error::msg)?;
    let refs = profiles_matching(config_root, |p| p.post.chain.iter().any(|c| c == id));
    anyhow::ensure!(
        refs.is_empty(),
        "cannot delete post component {id:?}; referenced by profile(s): {}",
        refs.join(", ")
    );
    let path = config_root.join("post").join(format!("{id}.toml"));
    std::fs::remove_file(&path)
        .with_context(|| format!("delete post component {}", path.display()))?;
    Ok(path)
}

pub(crate) fn delete_profile_file(config_root: &Path, profile_id: &str) -> Result<PathBuf> {
    crate::config::inventory::validate_config_file_id(profile_id).map_err(anyhow::Error::msg)?;
    ensure_profile_not_referenced(config_root, profile_id)?;
    let path = config_root
        .join("profile")
        .join(format!("{profile_id}.toml"));
    std::fs::remove_file(&path).with_context(|| format!("delete profile {}", path.display()))?;
    Ok(path)
}

fn ensure_profile_not_referenced(config_root: &Path, profile_id: &str) -> Result<()> {
    let config_path = config_root.join("config.toml");
    let routes = match std::fs::read_to_string(&config_path) {
        Ok(body) => profile_routes_from_main_body(&body)
            .with_context(|| format!("parse [profile] in {}", config_path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            crate::config::ProfileRouteCfg::default()
        }
        Err(e) => {
            return Err(anyhow::Error::new(e))
                .with_context(|| format!("read config {}", config_path.display()))
        }
    };

    if routes.default == profile_id {
        anyhow::bail!(
            "cannot delete default profile {profile_id:?}; change [profile].default first"
        );
    }
    if routes.routes.contains_key(profile_id) {
        anyhow::bail!(
            "cannot delete profile {profile_id:?}; it is referenced by [profile] route/app bindings"
        );
    }
    Ok(())
}

fn profile_routes_from_main_body(body: &str) -> Result<crate::config::ProfileRouteCfg> {
    let value = toml::from_str::<toml::Value>(body)?;
    let Some(profile) = value.get("profile") else {
        return Ok(crate::config::ProfileRouteCfg::default());
    };
    profile.clone().try_into().map_err(anyhow::Error::new)
}

#[cfg(test)]
mod profile_file_tests {
    use std::fs;

    use super::*;

    fn temp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shuohua-profile-file-{}", ulid::Ulid::new()));
        fs::create_dir_all(dir.join("profile")).unwrap();
        dir
    }

    #[test]
    fn create_profile_writes_new_valid_file() {
        let root = temp_root();

        let path = create_profile_file(&root, "meeting", "Meeting", "apple").unwrap();

        assert_eq!(path, root.join("profile/meeting.toml"));
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("name = \"Meeting\""), "{body}");
        assert!(body.contains("instance = \"apple\""), "{body}");
        parse(&body).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn create_profile_rejects_existing_file_without_overwrite() {
        let root = temp_root();
        let path = root.join("profile/default.toml");
        fs::write(&path, "keep me").unwrap();

        let error = create_profile_file(&root, "default", "Default", "apple").unwrap_err();

        assert!(error.to_string().contains("already exists"), "{error:#}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn create_profile_rejects_invalid_file_id() {
        let root = temp_root();

        let error = create_profile_file(&root, "My Profile", "My Profile", "apple").unwrap_err();

        assert!(
            error.to_string().contains("lowercase letter first"),
            "{error:#}"
        );
        assert!(!root.join("profile/My Profile.toml").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_unreferenced_profile_removes_file() {
        let root = temp_root();
        let path = root.join("profile/meeting.toml");
        fs::write(&path, "name = \"Meeting\"\n[asr]\ninstance = \"apple\"\n").unwrap();
        fs::write(
            root.join("config.toml"),
            "[profile]\ndefault = \"default\"\n",
        )
        .unwrap();

        delete_profile_file(&root, "meeting").unwrap();

        assert!(!path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_rejects_default_and_routed_profiles() {
        let root = temp_root();
        fs::write(root.join("profile/default.toml"), "").unwrap();
        fs::write(root.join("profile/coding.toml"), "").unwrap();
        fs::write(
            root.join("config.toml"),
            r#"[profile]
default = "default"
coding = ["com.example.App"]
"#,
        )
        .unwrap();

        let default_error = delete_profile_file(&root, "default").unwrap_err();
        let routed_error = delete_profile_file(&root, "coding").unwrap_err();

        assert!(
            default_error.to_string().contains("default profile"),
            "{default_error:#}"
        );
        assert!(
            routed_error.to_string().contains("route"),
            "{routed_error:#}"
        );
        assert!(root.join("profile/default.toml").exists());
        assert!(root.join("profile/coding.toml").exists());
        let _ = fs::remove_dir_all(root);
    }

    fn write_profile_fixture(root: &std::path::Path, stem: &str, asr: &str, chain: &[&str]) {
        let chain_toml = chain
            .iter()
            .map(|c| format!("{c:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let body =
            format!("name = {stem:?}\n[asr]\ninstance = {asr:?}\n[post]\nchain = [{chain_toml}]\n");
        fs::write(root.join("profile").join(format!("{stem}.toml")), body).unwrap();
    }

    #[test]
    fn delete_asr_instance_unreferenced_removes_file() {
        let root = temp_root();
        // Profile references a DIFFERENT instance ("apple"), not "team".
        write_profile_fixture(&root, "default", "apple", &[]);
        fs::create_dir_all(root.join("asr")).unwrap();
        let asr_path = root.join("asr/team.toml");
        fs::write(&asr_path, "type = \"doubao\"\n").unwrap();

        let result = delete_asr_instance_file(&root, "team");

        assert!(result.is_ok(), "{result:#?}");
        assert!(!asr_path.exists(), "file should be removed");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_asr_instance_referenced_is_blocked() {
        let root = temp_root();
        // Profile references "team".
        write_profile_fixture(&root, "default", "team", &[]);
        fs::create_dir_all(root.join("asr")).unwrap();
        let asr_path = root.join("asr/team.toml");
        fs::write(&asr_path, "type = \"doubao\"\n").unwrap();

        let err = delete_asr_instance_file(&root, "team").unwrap_err();
        let msg = format!("{err:#}");

        assert!(
            msg.contains("team"),
            "error should mention instance id: {msg}"
        );
        assert!(
            msg.contains("default"),
            "error should mention referencing profile: {msg}"
        );
        assert!(
            asr_path.exists(),
            "file must still exist after blocked delete"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_post_component_unreferenced_removes_file() {
        let root = temp_root();
        // Profile chain has a different component, not "deepseek".
        write_profile_fixture(&root, "default", "apple", &["other"]);
        let post_path = root.join("post/deepseek.toml");
        fs::create_dir_all(post_path.parent().unwrap()).unwrap();
        fs::write(
            &post_path,
            "type = \"llm\"\nname = \"deepseek\"\napi_key = \"k\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();

        let result = delete_post_component_file(&root, "deepseek");

        assert!(result.is_ok(), "{result:#?}");
        assert!(!post_path.exists(), "file should be removed");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_post_component_referenced_is_blocked() {
        let root = temp_root();
        write_profile_fixture(&root, "default", "apple", &["deepseek"]);
        let post_path = root.join("post/deepseek.toml");
        fs::create_dir_all(post_path.parent().unwrap()).unwrap();
        fs::write(
            &post_path,
            "type = \"llm\"\nname = \"deepseek\"\napi_key = \"k\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();

        let err = delete_post_component_file(&root, "deepseek").unwrap_err();
        let msg = format!("{err:#}");

        assert!(
            msg.contains("deepseek"),
            "error should mention component id: {msg}"
        );
        assert!(
            msg.contains("default"),
            "error should mention referencing profile: {msg}"
        );
        assert!(
            post_path.exists(),
            "file must still exist after blocked delete"
        );
        let _ = fs::remove_dir_all(root);
    }
}

fn profile_path_for_routes(
    profile_dir: &Path,
    routes: &ProfileRouteCfg,
    bundle_id: Option<&str>,
) -> Result<PathBuf> {
    let profile_name = resolve_profile_name(routes, bundle_id)?;
    crate::config::inventory::validate_config_file_id(&profile_name)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("invalid profile id {profile_name:?}"))?;
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
instance = "doubao"
hotwords = ["Rust"]

[post]
chain = ["zh_filter", "deepseek"]
"#,
        )
        .unwrap();

        let routes = ProfileRouteCfg::default();
        let profile = load_for_app(&dir, &routes, Some("com.example.Missing")).unwrap();

        assert_eq!(profile.name.as_deref(), Some("default"));
        assert_eq!(profile.id, "default");
        assert_eq!(profile.display_name(), "default");
        assert_eq!(profile.asr.instance, "doubao");
        assert_eq!(profile.asr.hotwords, vec!["Rust"]);
        assert_eq!(profile.post.chain, vec!["zh_filter", "deepseek"]);
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
instance = "doubao"

[post]
chain = ["zh_filter"]
"#,
        )
        .unwrap();
        fs::write(
            dir.join("agent.toml"),
            r#"
name = "agent"
[asr]
instance = "doubao"

[post]
chain = ["deepseek"]

[post.overrides.deepseek]
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

        assert_eq!(profile.name.as_deref(), Some("agent"));
        assert_eq!(profile.post.chain, vec!["deepseek"]);
        assert_eq!(
            profile
                .post
                .overrides
                .get("deepseek")
                .unwrap()
                .get("system_prompt")
                .and_then(toml::Value::as_str),
            Some("app prompt")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn profile_routes_reject_invalid_profile_id_before_reading_file() {
        let dir = temp_dir();
        let routes = ProfileRouteCfg {
            default: "1default".to_string(),
            routes: Default::default(),
        };

        let error = load_for_app(&dir, &routes, None).unwrap_err();
        let error = format!("{error:#}");

        assert!(error.contains("invalid profile id"), "{error}");
        assert!(error.contains("lowercase letter first"), "{error}");
        assert!(!error.contains("read profile"), "{error}");
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
instance = "doubao"

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
instance = "apple"

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

        assert_eq!(agent.name.as_deref(), Some("agent"));
        assert_eq!(agent.asr.instance, "apple");
        assert_eq!(default.name.as_deref(), Some("default"));
        assert_eq!(default.asr.instance, "doubao");
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
instance = "apple"
"#,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("unknown"), "{err}");
        assert!(err.contains("unknown field"), "{err}");
    }

    #[test]
    fn parse_allows_asr_instance_overrides() {
        let profile = parse(
            r#"
name = "default"

[asr]
instance = "apple"
language = "zh-CN"
local_vad = "on"
open_timeout_ms = 5000
finalize_timeout_ms = 5000
"#,
        )
        .unwrap();

        assert_eq!(profile.asr.instance, "apple");
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
