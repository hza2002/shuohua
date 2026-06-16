use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsRow {
    pub group: String,
    pub key: String,
    pub value: String,
    pub source: String,
}

pub fn load_rows() -> Vec<SettingsRow> {
    let mut rows = Vec::new();
    push_global_rows(&mut rows);
    push_app_rows(&mut rows);
    push_asr_rows(&mut rows);
    push_post_rows(&mut rows);
    rows
}

fn push_global_rows(rows: &mut Vec<SettingsRow>) {
    let path = crate::config::default_path();
    match crate::config::load_from(&path) {
        Ok(cfg) => {
            let source = path.display().to_string();
            rows.extend([
                row("global", "hotkey.trigger", cfg.hotkey.trigger, &source),
                row(
                    "global",
                    "voice.stop_delay_ms",
                    cfg.voice.stop_delay_ms.to_string(),
                    &source,
                ),
                row(
                    "global",
                    "voice.auto_paste",
                    cfg.voice.auto_paste.to_string(),
                    &source,
                ),
                row(
                    "global",
                    "voice.vad.backend",
                    format!("{:?}", cfg.voice.vad.backend).to_lowercase(),
                    &source,
                ),
                row(
                    "global",
                    "post.timeout_ms",
                    cfg.post.timeout_ms.to_string(),
                    &source,
                ),
                row("global", "ui.language", cfg.ui.language, &source),
            ]);
        }
        Err(e) => rows.push(row(
            "global",
            "config",
            format!("error: {e:#}"),
            &path.display().to_string(),
        )),
    }
}

fn push_app_rows(rows: &mut Vec<SettingsRow>) {
    let dir = crate::app_profile::default_dir();
    for path in toml_files(&dir) {
        let source = path.display().to_string();
        match fs::read_to_string(&path)
            .ok()
            .and_then(|body| toml::from_str::<toml::Value>(&body).ok())
        {
            Some(value) => {
                let name = value
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("-");
                let provider = value
                    .get("asr")
                    .and_then(toml::Value::as_table)
                    .and_then(|asr| asr.get("provider"))
                    .and_then(toml::Value::as_str)
                    .unwrap_or("-");
                let chain = value
                    .get("post")
                    .and_then(toml::Value::as_table)
                    .and_then(|post| post.get("chain"))
                    .and_then(toml::Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(toml::Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" -> ")
                    })
                    .unwrap_or_else(|| "-".to_string());
                let key = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("profile");
                rows.push(row(
                    "app",
                    key,
                    format!("{name} | {provider} | {chain}"),
                    &source,
                ));
            }
            None => rows.push(row("app", "parse", "error", &source)),
        }
    }
}

fn push_asr_rows(rows: &mut Vec<SettingsRow>) {
    let dir = config_home().join("shuohua/asr");
    for path in toml_files(&dir) {
        push_toml_summary(rows, "asr", &path);
    }
}

fn push_post_rows(rows: &mut Vec<SettingsRow>) {
    let dir = crate::post::config::default_dir();
    for subdir in ["rules", "llm"] {
        for path in toml_files(&dir.join(subdir)) {
            push_toml_summary(rows, "post", &path);
        }
    }
}

fn push_toml_summary(rows: &mut Vec<SettingsRow>, group: &str, path: &Path) {
    let source = path.display().to_string();
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("-");
    match fs::read_to_string(path)
        .ok()
        .and_then(|body| toml::from_str::<toml::Value>(&body).ok())
        .and_then(|value| value.as_table().cloned())
    {
        Some(table) => {
            for (key, value) in table {
                if value.is_table() {
                    rows.push(row(group, &format!("{name}.{key}"), "{...}", &source));
                } else {
                    rows.push(row(
                        group,
                        &format!("{name}.{key}"),
                        display_value(&key, &value),
                        &source,
                    ));
                }
            }
        }
        None => rows.push(row(group, name, "parse error", &source)),
    }
}

fn row(group: &str, key: &str, value: impl Into<String>, source: &str) -> SettingsRow {
    SettingsRow {
        group: group.to_string(),
        key: key.to_string(),
        value: value.into(),
        source: source.to_string(),
    }
}

fn display_value(key: &str, value: &toml::Value) -> String {
    if is_secret_key(key) {
        return if value.as_str().is_some_and(str::is_empty) {
            "<empty>".to_string()
        } else {
            "<set>".to_string()
        };
    }
    match value {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_lowercase();
    key.contains("key") || key.contains("token") || key.contains("secret")
}

fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut paths = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    paths.sort();
    paths
}

fn config_home() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_value_redacts_secret_keys() {
        assert_eq!(
            display_value("api_key", &toml::Value::String("sk-test".to_string())),
            "<set>"
        );
        assert_eq!(
            display_value("api_key", &toml::Value::String(String::new())),
            "<empty>"
        );
    }

    #[test]
    fn display_value_keeps_plain_strings() {
        assert_eq!(
            display_value("model", &toml::Value::String("deepseek-chat".to_string())),
            "deepseek-chat"
        );
    }
}
