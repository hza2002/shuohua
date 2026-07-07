use std::path::{Path, PathBuf};

use crate::config::field_view::{self, ControlKind, FieldOrigin, FieldView};
use crate::config::inventory::InventoryModule;
use crate::config::{paths, schema};

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsRow {
    pub group: String,
    pub field_path: String,
    pub display_key: String,
    pub value: String,
    pub default_value: String,
    pub origin: FieldOrigin,
    pub control: ControlKind,
    pub editable: bool,
    pub secret: bool,
    /// 该字段能否「重置为默认」（删键后文件仍合法）。见 FieldView::can_unset。
    pub can_unset: bool,
    pub source: String,
    pub description_key: Option<&'static str>,
}

pub fn load_rows() -> Vec<SettingsRow> {
    let root = paths::config_home().join("shuohua");
    let mut rows = Vec::new();
    rows.extend(rows_for_path(
        &root,
        &root.join("config.toml"),
        InventoryModule::Main,
    ));
    for path in toml_files(&root.join("profile")) {
        rows.extend(rows_for_path(&root, &path, InventoryModule::Profile));
    }
    for path in toml_files(&root.join("asr")) {
        rows.extend(rows_for_path(&root, &path, InventoryModule::AsrProvider));
    }
    for path in toml_files(&root.join("post")) {
        rows.extend(rows_for_path(&root, &path, InventoryModule::PostProcessor));
    }
    for path in toml_files(&root.join("theme")) {
        rows.extend(rows_for_path(&root, &path, InventoryModule::Theme));
    }
    rows
}

fn rows_for_path(root: &Path, path: &Path, module: InventoryModule) -> Vec<SettingsRow> {
    let rel = relative_source_path(path).unwrap_or_default();
    let spec = match schema::spec_for_config_file(path, &rel) {
        Some(Ok(spec)) => spec,
        Some(Err(e)) => {
            // Broken ASR file (e.g. missing/invalid `type`): surface a non-editable
            // error row so the file is visible in the TUI rather than silently absent.
            let display_key = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("type")
                .to_string();
            return vec![SettingsRow {
                group: module.label().to_string(),
                field_path: display_key.clone(),
                display_key,
                value: format!("{e:#}"),
                default_value: String::new(),
                origin: crate::config::field_view::FieldOrigin::Default,
                control: crate::config::field_view::ControlKind::Text,
                editable: false,
                secret: false,
                can_unset: false,
                source: path.display().to_string(),
                description_key: None,
            }];
        }
        None => return Vec::new(),
    };
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|body| toml::from_str::<toml::Value>(&body).ok())
        .unwrap_or_else(|| toml::Value::Table(Default::default()));
    field_view::field_views(&rel, &spec, &parsed, root)
        .into_iter()
        .map(|view| row_from_view(module, path, view))
        .collect()
}

fn row_from_view(module: InventoryModule, path: &Path, view: FieldView) -> SettingsRow {
    SettingsRow {
        group: module.label().to_string(),
        display_key: view.field_path.clone(),
        field_path: view.field_path,
        value: view.effective,
        default_value: view.default_value,
        origin: view.origin,
        control: view.control,
        editable: view.editable,
        secret: view.secret,
        can_unset: view.can_unset,
        source: path.display().to_string(),
        description_key: view.description_key,
    }
}

fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut paths = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    paths.sort();
    paths
}

fn relative_source_path(path: &Path) -> Option<String> {
    let marker = Path::new("shuohua");
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
            .and_then(|n| n.to_str())
            .map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_extracts_after_marker() {
        let p = Path::new("/home/u/.config/shuohua/asr/doubao.toml");
        assert_eq!(relative_source_path(p).as_deref(), Some("asr/doubao.toml"));
    }
}
