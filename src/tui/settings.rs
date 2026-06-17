use crate::config::inventory::{self, InventoryEntry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsRow {
    pub group: String,
    pub key: String,
    pub value: String,
    pub source: String,
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
    }
}
