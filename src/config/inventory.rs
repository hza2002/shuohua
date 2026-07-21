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
    push_theme(&mut inventory, &root);
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
    match read_toml(&path).and_then(|value| {
        value
            .as_table()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("expected top-level table"))
    }) {
        Ok(table) => push_table_fields(
            inventory,
            InventoryModule::Main,
            "config",
            None,
            &table,
            &path,
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
        if let Err(error) = validate_config_file_stem(&path) {
            push_entry(
                inventory,
                InventoryModule::Profile,
                file_stem(&path, "profile"),
                error,
                source,
                InventoryStatus::Error,
            );
            continue;
        }
        match read_toml(&path).and_then(|value| {
            value
                .as_table()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("expected top-level table"))
        }) {
            Ok(table) => push_table_fields(
                inventory,
                InventoryModule::Profile,
                &file_stem(&path, "profile"),
                None,
                &table,
                &source,
            ),
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
    for path in toml_files(&root.join("post")) {
        push_toml_summary(inventory, InventoryModule::PostProcessor, &path);
    }
}

fn push_asr(inventory: &mut ConfigInventory, root: &Path) {
    for path in toml_files(&root.join("asr")) {
        push_toml_summary(inventory, InventoryModule::AsrProvider, &path);
    }
}

fn push_theme(inventory: &mut ConfigInventory, root: &Path) {
    let mut found = false;
    for path in toml_files(&root.join("theme")) {
        found = true;
        let source = path.clone();
        if let Err(error) = validate_config_file_stem(&path) {
            push_entry(
                inventory,
                InventoryModule::Theme,
                file_stem(&path, "theme"),
                error,
                source,
                InventoryStatus::Error,
            );
            continue;
        }
        match read_toml(&path).and_then(|value| {
            value
                .as_table()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("expected top-level table"))
        }) {
            Ok(table) => push_table_fields(
                inventory,
                InventoryModule::Theme,
                &file_stem(&path, "theme"),
                None,
                &table,
                &source,
            ),
            Err(e) => push_entry(
                inventory,
                InventoryModule::Theme,
                file_stem(&path, "theme"),
                format!("parse error: {e}"),
                source,
                InventoryStatus::Error,
            ),
        }
    }
    if !found {
        push_entry(
            inventory,
            InventoryModule::Theme,
            crate::config::theme::DEFAULT_THEME_NAME,
            "builtin default",
            root.join("theme/gruvbox-dark.toml"),
            InventoryStatus::Ok,
        );
    }
}

fn push_toml_summary(inventory: &mut ConfigInventory, module: InventoryModule, path: &Path) {
    if let Err(error) = validate_config_file_stem(path) {
        push_entry(
            inventory,
            module,
            file_stem(path, module.label()),
            error,
            path.to_path_buf(),
            InventoryStatus::Error,
        );
        return;
    }
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

fn push_table_fields(
    inventory: &mut ConfigInventory,
    module: InventoryModule,
    key_prefix: &str,
    field_prefix: Option<&str>,
    table: &toml::map::Map<String, toml::Value>,
    path: &Path,
) {
    for (key, value) in table {
        let field_path = match field_prefix {
            Some(prefix) => format!("{prefix}.{key}"),
            None => key.clone(),
        };
        let entry_key = format!("{key_prefix}.{field_path}");
        if let Some(table) = value.as_table() {
            push_table_fields(
                inventory,
                module,
                key_prefix,
                Some(&field_path),
                table,
                path,
            );
        } else {
            push_entry_with_field(
                inventory,
                module,
                entry_key,
                Some(field_path.clone()),
                display_value(key, value),
                path.to_path_buf(),
                InventoryStatus::Ok,
            );
        }
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
        return match value.as_str() {
            Some("") | None => "<empty>".to_string(),
            Some(_) => crate::config::spec::SECRET_MASK.to_string(),
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

pub(crate) fn validate_config_file_stem(path: &Path) -> Result<(), String> {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return Err("invalid file name: expected UTF-8 TOML stem".to_string());
    };
    validate_config_file_id(stem).map_err(|error| format!("invalid file name: {error}"))
}

pub(crate) fn validate_config_file_id(id: &str) -> Result<(), String> {
    if is_valid_config_file_stem(id) {
        Ok(())
    } else {
        Err(format!(
            "invalid file id {id:?}; use a lowercase letter first, then lowercase letters, digits, '-' or '_' (examples: default, zh_filter, team-1). Put display text in name = \"...\"."
        ))
    }
}

pub(crate) fn is_valid_config_file_stem(stem: &str) -> bool {
    let mut chars = stem.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}

fn config_home() -> PathBuf {
    crate::config::paths::config_home()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn temp_config_home() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("shuohua-inventory-test-{}", ulid::Ulid::generate()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn config_file_ids_use_lowercase_letter_first_rule() {
        for valid in ["default", "a1_b-c", "zh_filter", "team-1"] {
            assert!(is_valid_config_file_stem(valid), "{valid}");
            assert!(validate_config_file_id(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "1abc", "Abc", "my profile", "zh.filter"] {
            assert!(!is_valid_config_file_stem(invalid), "{invalid}");
            let error = validate_config_file_id(invalid).unwrap_err();
            assert!(error.contains("lowercase letter first"), "{error}");
        }
    }

    #[test]
    fn inventory_summarizes_main_profile_asr_and_post_files() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("asr")).unwrap();
        fs::create_dir_all(root.join("post")).unwrap();
        fs::create_dir_all(root.join("theme")).unwrap();
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
instance = "apple"
[post]
chain = ["zh_filter", "deepseek"]
"#,
        )
        .unwrap();
        fs::write(
            root.join("asr/apple.toml"),
            "type = \"apple\"\nlocal_vad = \"on\"\n",
        )
        .unwrap();
        fs::write(
            root.join("post/zh_filter.toml"),
            "type = \"rule\"\npatterns = []\n",
        )
        .unwrap();
        fs::write(
            root.join("post/deepseek.toml"),
            "type = \"llm\"\nname = \"deepseek\"\napi_key = \"sk-test\"\nmodel = \"deepseek-chat\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();
        fs::write(
            root.join("theme/gruvbox-dark.toml"),
            "name = \"Gruvbox Dark\"\n[tui]\nhighlight = \"fg0\"\n",
        )
        .unwrap();

        let inventory = load_from_config_home(&home);

        assert_eq!(inventory.modules.len(), 6);
        assert!(inventory.entries().any(|entry| {
            entry.module == InventoryModule::Main
                && entry.key == "config.hotkey.trigger"
                && entry.field_path.as_deref() == Some("hotkey.trigger")
                && entry.status == InventoryStatus::Ok
                && entry.summary == "f16"
        }));
        assert!(inventory.entries().any(|entry| {
            entry.module == InventoryModule::Profile
                && entry.key == "default.asr.instance"
                && entry.field_path.as_deref() == Some("asr.instance")
                && entry.summary.contains("apple")
        }));
        assert!(inventory.entries().any(|entry| {
            entry.module == InventoryModule::Profile
                && entry.key == "default.post.chain"
                && entry.field_path.as_deref() == Some("post.chain")
                && entry.summary.contains("deepseek")
        }));
        assert!(inventory.entries().any(|entry| {
            entry.module == InventoryModule::AsrProvider
                && entry.key == "apple.local_vad"
                && entry.field_path.as_deref() == Some("local_vad")
                && entry.summary == "on"
        }));
        assert!(inventory.entries().any(|entry| {
            entry.module == InventoryModule::PostProcessor
                && entry.key == "deepseek.api_key"
                && entry.summary == crate::config::spec::SECRET_MASK
        }));
        assert!(inventory.entries().any(|entry| {
            entry.module == InventoryModule::Theme
                && entry.key == "gruvbox-dark.tui.highlight"
                && entry.field_path.as_deref() == Some("tui.highlight")
                && entry.summary == "fg0"
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
    fn inventory_rejects_non_identifier_config_file_names() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/My Profile.toml"),
            "name = \"My Profile\"\n[asr]\ninstance = \"apple\"\n",
        )
        .unwrap();

        let inventory = load_from_config_home(&home);

        assert!(inventory.entries().any(|entry| {
            entry.module == InventoryModule::Profile
                && entry.key == "My Profile"
                && entry.status == InventoryStatus::Error
                && entry.summary.contains("invalid file name")
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
            "name = \"default\"\n[asr]\ninstance = \"apple\"\n",
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
