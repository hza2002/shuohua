use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppProfile {
    pub name: String,
    pub asr: String,
    #[serde(default)]
    pub hotwords: Vec<String>,
    #[serde(default)]
    pub post_chain: Vec<String>,
}

pub fn default_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("shuohua/apps");
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/shuohua/apps")
}

pub fn load_for_app(apps_dir: &Path, bundle_id: Option<&str>) -> Result<AppProfile> {
    let path = profile_path(apps_dir, bundle_id);
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("read app profile {}", path.display()))?;
    toml::from_str(&body).with_context(|| format!("parse app profile {}", path.display()))
}

fn profile_path(apps_dir: &Path, bundle_id: Option<&str>) -> PathBuf {
    if let Some(bundle_id) = bundle_id {
        let app_path = apps_dir.join(format!("{bundle_id}.toml"));
        if app_path.exists() {
            return app_path;
        }
    }
    apps_dir.join("default.toml")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shuohua-apps-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn app_profile_falls_back_to_default() {
        let dir = temp_dir();
        fs::write(
            dir.join("default.toml"),
            r#"
name = "default"
asr = "doubao"
hotwords = ["Rust"]
post_chain = ["rule:filler", "llm:deepseek"]
"#,
        )
        .unwrap();

        let profile = load_for_app(&dir, Some("com.example.Missing")).unwrap();

        assert_eq!(profile.name, "default");
        assert_eq!(profile.asr, "doubao");
        assert_eq!(profile.hotwords, vec!["Rust"]);
        assert_eq!(profile.post_chain, vec!["rule:filler", "llm:deepseek"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn per_app_profile_wins_over_default() {
        let dir = temp_dir();
        fs::write(
            dir.join("default.toml"),
            r#"
name = "default"
asr = "doubao"
post_chain = ["rule:filler"]
"#,
        )
        .unwrap();
        fs::write(
            dir.join("com.example.App.toml"),
            r#"
name = "app"
asr = "doubao"
post_chain = ["llm:deepseek"]
"#,
        )
        .unwrap();

        let profile = load_for_app(&dir, Some("com.example.App")).unwrap();

        assert_eq!(profile.name, "app");
        assert_eq!(profile.post_chain, vec!["llm:deepseek"]);
        let _ = fs::remove_dir_all(dir);
    }
}
