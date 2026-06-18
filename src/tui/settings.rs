use crate::config::inventory::{self, InventoryEntry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsRow {
    pub group: String,
    pub key: String,
    pub value: String,
    pub source: String,
    pub description_key: Option<&'static str>,
}

pub fn load_rows() -> Vec<SettingsRow> {
    inventory::load().entries().map(row_from_entry).collect()
}

fn row_from_entry(entry: &InventoryEntry) -> SettingsRow {
    SettingsRow {
        group: entry.module.label().to_string(),
        key: entry.key.clone(),
        value: match entry.status {
            inventory::InventoryStatus::Ok => entry.summary.clone(),
            inventory::InventoryStatus::Warning => format!("warning: {}", entry.summary),
            inventory::InventoryStatus::Error => format!("error: {}", entry.summary),
            inventory::InventoryStatus::Missing => format!("missing: {}", entry.summary),
        },
        source: entry.source.display().to_string(),
        description_key: description_key_for_entry(entry),
    }
}

fn description_key_for_entry(entry: &InventoryEntry) -> Option<&'static str> {
    let field_path = entry
        .field_path
        .as_deref()
        .or_else(|| field_path_from_key(&entry.key));
    let rel_path = relative_source_path(&entry.source)?;
    let spec = crate::config::schema::spec_for_path(&rel_path)?;
    spec.field_for_path(field_path?)
        .and_then(|field| field.description_key_value())
}

fn field_path_from_key(key: &str) -> Option<&str> {
    key.split_once('.').map(|(_, field)| field)
}

fn relative_source_path(path: &std::path::Path) -> Option<String> {
    let marker = std::path::Path::new("shuohua");
    let mut found = false;
    let mut parts = Vec::new();
    for component in path.components() {
        let text = component.as_os_str().to_str()?;
        if found {
            parts.push(text);
        } else if text == marker {
            found = true;
        }
    }
    if found && !parts.is_empty() {
        Some(parts.join("/"))
    } else {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn row_description_key_comes_from_config_schema() {
        let entry = InventoryEntry {
            module: inventory::InventoryModule::AsrProvider,
            key: "apple.idle_pause".to_string(),
            field_path: Some("idle_pause".to_string()),
            summary: "false".to_string(),
            source: PathBuf::from("/tmp/shuohua/asr/apple.toml"),
            status: inventory::InventoryStatus::Ok,
        };

        let row = row_from_entry(&entry);

        assert_eq!(
            row.description_key,
            Some("config.field.idle_pause.description")
        );
    }
}
