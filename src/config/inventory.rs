use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryModule {
    Overview,
    Main,
    Profile,
    PostProcessor,
    AsrProvider,
    Theme,
}

impl InventoryModule {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Main => "main",
            Self::Profile => "profile",
            Self::PostProcessor => "post",
            Self::AsrProvider => "asr",
            Self::Theme => "theme",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryStatus {
    Ok,
    #[allow(dead_code)]
    Warning,
    Error,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryEntry {
    pub module: InventoryModule,
    pub key: String,
    pub field_path: Option<String>,
    pub summary: String,
    pub source: PathBuf,
    pub status: InventoryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventorySection {
    pub module: InventoryModule,
    pub entries: Vec<InventoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigInventory {
    pub root: PathBuf,
    pub modules: Vec<InventorySection>,
}

impl ConfigInventory {
    pub fn entries(&self) -> impl Iterator<Item = &InventoryEntry> {
        self.modules
            .iter()
            .flat_map(|section| section.entries.iter())
    }
}

pub fn load() -> ConfigInventory {
    load_from_config_home(&config_home())
}

pub fn load_from_config_home(config_home: &Path) -> ConfigInventory {
    let root = config_home.join("shuohua");
    let mut inventory = ConfigInventory {
        root: root.clone(),
        modules: [
            InventoryModule::Overview,
            InventoryModule::Main,
            InventoryModule::Profile,
            InventoryModule::PostProcessor,
            InventoryModule::AsrProvider,
            InventoryModule::Theme,
        ]
        .into_iter()
        .map(|module| InventorySection {
            module,
            entries: Vec::new(),
        })
        .collect(),
    };

    push_main(&mut inventory, &root);
    push_profiles(&mut inventory, &root);
    push_post(&mut inventory, &root);
    push_asr(&mut inventory, &root);
    push_theme_placeholder(&mut inventory, &root);
    push_overview(&mut inventory);
    inventory
}

fn push_overview(inventory: &mut ConfigInventory) {
    let total_files = inventory
        .entries()
        .filter(|entry| entry.module != InventoryModule::Overview)
        .map(|entry| entry.source.clone())
        .collect::<BTreeSet<_>>()
        .len();
    push_entry(
        inventory,
        InventoryModule::Overview,
        "summary",
        format!("{total_files} config files scanned"),
        inventory.root.clone(),
        InventoryStatus::Ok,
    );
}

fn push_main(inventory: &mut ConfigInventory, root: &Path) {
    let path = root.join("config.toml");
    match crate::config::load_from(&path) {
        Ok(cfg) => push_entry_with_field(
            inventory,
            InventoryModule::Main,
            "config",
            Some("hotkey.trigger".to_string()),
            format!(
                "hotkey.trigger={} | voice.vad.backend={:?} | post.timeout_ms={}",
                cfg.hotkey.trigger, cfg.voice.vad.backend, cfg.post.timeout_ms
            )
            .to_lowercase(),
            path,
            InventoryStatus::Ok,
        ),
        Err(e) => push_entry(
            inventory,
            InventoryModule::Main,
            "config",
            format!("error: {e:#}"),
            path,
            InventoryStatus::Error,
        ),
    }
}

fn push_profiles(inventory: &mut ConfigInventory, root: &Path) {
    for path in toml_files(&root.join("profile")) {
        let source = path.clone();
        match read_toml(&path) {
            Ok(value) => {
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
                push_entry(
                    inventory,
                    InventoryModule::Profile,
                    file_stem(&path, "profile"),
                    format!("{name} | {provider} | {chain}"),
                    source,
                    InventoryStatus::Ok,
                );
            }
            Err(e) => push_entry(
                inventory,
                InventoryModule::Profile,
                file_stem(&path, "profile"),
                format!("parse error: {e}"),
                source,
                InventoryStatus::Error,
            ),
        }
    }
}

fn push_post(inventory: &mut ConfigInventory, root: &Path) {
    let dir = root.join("post");
    for subdir in ["rule", "llm"] {
        for path in toml_files(&dir.join(subdir)) {
            push_toml_summary(inventory, InventoryModule::PostProcessor, &path);
        }
    }
}

fn push_asr(inventory: &mut ConfigInventory, root: &Path) {
    for path in toml_files(&root.join("asr")) {
        push_toml_summary(inventory, InventoryModule::AsrProvider, &path);
    }
}

fn push_theme_placeholder(inventory: &mut ConfigInventory, root: &Path) {
    let path = root.join("theme");
    push_entry(
        inventory,
        InventoryModule::Theme,
        "theme",
        "reserved".to_string(),
        path,
        InventoryStatus::Missing,
    );
}

fn push_toml_summary(inventory: &mut ConfigInventory, module: InventoryModule, path: &Path) {
    match read_toml(path).and_then(|value| {
        value
            .as_table()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("expected top-level table"))
    }) {
        Ok(table) => {
            let name = file_stem(path, module.label());
            for (key, value) in table {
                let summary = if value.is_table() {
                    "{...}".to_string()
                } else {
                    display_value(&key, &value)
                };
                push_entry_with_field(
                    inventory,
                    module,
                    format!("{name}.{key}"),
                    Some(key.clone()),
                    summary,
                    path.to_path_buf(),
                    InventoryStatus::Ok,
                );
            }
        }
        Err(e) => push_entry(
            inventory,
            module,
            file_stem(path, module.label()),
            format!("parse error: {e}"),
            path.to_path_buf(),
            InventoryStatus::Error,
        ),
    }
}

fn push_entry(
    inventory: &mut ConfigInventory,
    module: InventoryModule,
    key: impl Into<String>,
    summary: impl Into<String>,
    source: PathBuf,
    status: InventoryStatus,
) {
    push_entry_with_field(inventory, module, key, None, summary, source, status);
}

fn push_entry_with_field(
    inventory: &mut ConfigInventory,
    module: InventoryModule,
    key: impl Into<String>,
    field_path: Option<String>,
    summary: impl Into<String>,
    source: PathBuf,
    status: InventoryStatus,
) {
    let section = inventory
        .modules
        .iter_mut()
        .find(|section| section.module == module)
        .expect("inventory module exists");
    section.entries.push(InventoryEntry {
        module,
        key: key.into(),
        field_path,
        summary: summary.into(),
        source,
        status,
    });
}

fn read_toml(path: &Path) -> anyhow::Result<toml::Value> {
    let body = fs::read_to_string(path)?;
    Ok(toml::from_str(&body)?)
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

fn file_stem(path: &Path, fallback: &str) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(fallback)
        .to_string()
}

fn config_home() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn temp_config_home() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("shuohua-inventory-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn inventory_summarizes_main_profile_asr_and_post_files() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("asr")).unwrap();
        fs::create_dir_all(root.join("post/rule")).unwrap();
        fs::create_dir_all(root.join("post/llm")).unwrap();
        fs::write(
            root.join("config.toml"),
            r#"
[hotkey]
trigger = "f16"

[voice.vad]
backend = "silero"
"#,
        )
        .unwrap();
        fs::write(
            root.join("profile/default.toml"),
            r#"
name = "default"
[asr]
provider = "apple"
[post]
chain = ["rule:zh_filter", "llm:deepseek"]
"#,
        )
        .unwrap();
        fs::write(root.join("asr/apple.toml"), "idle_pause = true\n").unwrap();
        fs::write(
            root.join("post/rule/zh_filter.toml"),
            "type = \"rule\"\npatterns = []\n",
        )
        .unwrap();
        fs::write(
            root.join("post/llm/deepseek.toml"),
            "type = \"llm\"\nname = \"deepseek\"\napi_key = \"sk-test\"\nmodel = \"deepseek-chat\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();

        let inventory = load_from_config_home(&home);

        assert_eq!(inventory.modules.len(), 6);
        assert!(inventory
            .entries()
            .any(|entry| entry.module == InventoryModule::Main
                && entry.key == "config"
                && entry.status == InventoryStatus::Ok
                && entry.summary.contains("hotkey.trigger=f16")));
        assert!(inventory.entries().any(|entry| {
            entry.module == InventoryModule::Profile
                && entry.key == "default"
                && entry
                    .summary
                    .contains("default | apple | rule:zh_filter -> llm:deepseek")
        }));
        assert!(inventory.entries().any(|entry| {
            entry.module == InventoryModule::AsrProvider
                && entry.key == "apple.idle_pause"
                && entry.field_path.as_deref() == Some("idle_pause")
                && entry.summary == "true"
        }));
        assert!(inventory.entries().any(|entry| {
            entry.module == InventoryModule::PostProcessor
                && entry.key == "deepseek.api_key"
                && entry.summary == "<set>"
        }));

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn inventory_marks_parse_errors_without_dropping_file_entry() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::write(root.join("config.toml"), "not valid =").unwrap();
        fs::write(root.join("profile/broken.toml"), "[asr\n").unwrap();

        let inventory = load_from_config_home(&home);

        assert!(inventory.entries().any(|entry| {
            entry.module == InventoryModule::Main
                && entry.key == "config"
                && entry.status == InventoryStatus::Error
        }));
        assert!(inventory.entries().any(|entry| {
            entry.module == InventoryModule::Profile
                && entry.key == "broken"
                && entry.status == InventoryStatus::Error
        }));

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn overview_summary_counts_scanned_sources_not_zero() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/default.toml"),
            "name = \"default\"\n[asr]\nprovider = \"apple\"\n",
        )
        .unwrap();

        let inventory = load_from_config_home(&home);
        let overview = inventory
            .entries()
            .find(|entry| entry.module == InventoryModule::Overview && entry.key == "summary")
            .unwrap();

        assert!(!overview.summary.starts_with("0 "), "{overview:?}");
        let _ = fs::remove_dir_all(home);
    }
}
