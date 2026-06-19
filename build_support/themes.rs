use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeSource {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}

pub fn scan_theme_dir(dir: &Path) -> Result<Vec<ThemeSource>, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("read theme directory {}: {error}", dir.display()))?;
    let mut paths = entries
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| format!("read theme directory entry: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();

    let mut themes = Vec::new();
    let mut names = BTreeMap::<String, PathBuf>::new();
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("theme file name is not UTF-8: {}", path.display()))?;
        if file_name.starts_with('.') {
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            return Err(format!(
                "theme directory contains non-TOML file: {}",
                path.display()
            ));
        }
        let id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("theme file stem is not UTF-8: {}", path.display()))?;
        validate_id(id, &path)?;

        let body = std::fs::read_to_string(&path)
            .map_err(|error| format!("read theme {}: {error}", path.display()))?;
        let value: toml::Value = toml::from_str(&body)
            .map_err(|error| format!("parse theme {}: {error}", path.display()))?;
        let name = validate_theme(&value, &path)?;
        let normalized_name = name.to_lowercase();
        if let Some(existing) = names.insert(normalized_name, path.clone()) {
            return Err(format!(
                "duplicate theme name {name:?} in {} and {}",
                existing.display(),
                path.display()
            ));
        }
        themes.push(ThemeSource {
            id: id.to_string(),
            name,
            path,
        });
    }

    if themes.is_empty() {
        return Err(format!("theme directory is empty: {}", dir.display()));
    }
    Ok(themes)
}

pub fn generate_registry(themes: &[ThemeSource]) -> String {
    let mut output = String::from("const THEME_PRESETS: &[ThemePreset] = &[\n");
    for theme in themes {
        let file_name = theme
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("validated theme file name");
        output.push_str("    ThemePreset {\n");
        output.push_str(&format!("        id: {:?},\n", theme.id));
        output.push_str(&format!(
            "        path: {:?},\n",
            format!("theme/{file_name}")
        ));
        output.push_str(&format!(
            "        body: include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {:?})),\n",
            format!("/assets/themes/{file_name}")
        ));
        output.push_str("    },\n");
    }
    output.push_str("];\n");
    output
}

fn validate_id(id: &str, path: &Path) -> Result<(), String> {
    let valid = !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && id.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && id.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && !id.contains("--");
    if valid {
        Ok(())
    } else {
        Err(format!(
            "theme file name must be lowercase kebab-case: {}",
            path.display()
        ))
    }
}

fn validate_theme(value: &toml::Value, path: &Path) -> Result<String, String> {
    let root = value
        .as_table()
        .ok_or_else(|| format!("theme must be a TOML table: {}", path.display()))?;
    let name = root
        .get("name")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("theme must define string `name`: {}", path.display()))?;
    if name.is_empty() {
        return Err(format!("theme `name` cannot be empty: {}", path.display()));
    }
    if name.trim() != name {
        return Err(format!(
            "theme `name` cannot contain leading or trailing whitespace: {}",
            path.display()
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(format!(
            "theme `name` cannot contain control characters: {}",
            path.display()
        ));
    }

    let palette = required_table(root, "palette", path)?;
    if palette.is_empty() {
        return Err(format!("theme palette cannot be empty: {}", path.display()));
    }
    for (key, value) in palette {
        validate_hex(value, &format!("palette.{key}"), path)?;
    }

    let tui = required_table(root, "tui", path)?;
    for key in [
        "foreground",
        "muted",
        "accent",
        "success",
        "warning",
        "error",
        "info",
        "highlight",
        "border",
        "border_focus",
        "segment",
    ] {
        validate_color(
            required_value(tui, key, "tui", path)?,
            palette,
            &format!("tui.{key}"),
            path,
        )?;
    }

    let overlay = required_table(root, "overlay", path)?;
    let macos = required_table(overlay, "macos", path)?;
    required_integer(macos, "glass_variant", "overlay.macos", path)?;
    required_integer(macos, "subdued", "overlay.macos", path)?;
    required_integer(macos, "background_blur_radius", "overlay.macos", path)?;
    match required_value(macos, "glass_style", "overlay.macos", path)?.as_str() {
        Some("clear" | "blur") => {}
        _ => {
            return Err(format!(
                "overlay.macos.glass_style must be `clear` or `blur`: {}",
                path.display()
            ))
        }
    }

    let surface = required_table(overlay, "surface", path)?;
    validate_color(
        required_value(surface, "background", "overlay.surface", path)?,
        palette,
        "overlay.surface.background",
        path,
    )?;
    required_number(surface, "background_alpha", "overlay.surface", path)?;
    required_number(surface, "corner_radius", "overlay.surface", path)?;

    let text = required_table(overlay, "text", path)?;
    for key in [
        "primary",
        "secondary",
        "tertiary",
        "segment",
        "notice",
        "error",
    ] {
        validate_color(
            required_value(text, key, "overlay.text", path)?,
            palette,
            &format!("overlay.text.{key}"),
            path,
        )?;
    }

    let state = required_table(overlay, "state", path)?;
    for key in [
        "idle",
        "connecting",
        "recording",
        "thinking",
        "stopping",
        "error",
    ] {
        validate_color(
            required_value(state, key, "overlay.state", path)?,
            palette,
            &format!("overlay.state.{key}"),
            path,
        )?;
    }
    Ok(name.to_string())
}

fn required_table<'a>(
    table: &'a toml::value::Table,
    key: &str,
    path: &Path,
) -> Result<&'a toml::value::Table, String> {
    table
        .get(key)
        .and_then(toml::Value::as_table)
        .ok_or_else(|| format!("theme must define table `{key}`: {}", path.display()))
}

fn required_value<'a>(
    table: &'a toml::value::Table,
    key: &str,
    prefix: &str,
    path: &Path,
) -> Result<&'a toml::Value, String> {
    table
        .get(key)
        .ok_or_else(|| format!("theme must define `{prefix}.{key}`: {}", path.display()))
}

fn required_integer(
    table: &toml::value::Table,
    key: &str,
    prefix: &str,
    path: &Path,
) -> Result<(), String> {
    if required_value(table, key, prefix, path)?
        .as_integer()
        .is_some()
    {
        Ok(())
    } else {
        Err(format!(
            "`{prefix}.{key}` must be an integer: {}",
            path.display()
        ))
    }
}

fn required_number(
    table: &toml::value::Table,
    key: &str,
    prefix: &str,
    path: &Path,
) -> Result<(), String> {
    let value = required_value(table, key, prefix, path)?;
    if value.as_float().is_some() || value.as_integer().is_some() {
        Ok(())
    } else {
        Err(format!(
            "`{prefix}.{key}` must be a number: {}",
            path.display()
        ))
    }
}

fn validate_color(
    value: &toml::Value,
    palette: &toml::value::Table,
    field: &str,
    path: &Path,
) -> Result<(), String> {
    if let Some(name) = value.as_str() {
        if palette.contains_key(name) {
            return Ok(());
        }
        return Err(format!(
            "{field} references unknown palette color `{name}`: {}",
            path.display()
        ));
    }
    validate_hex(value, field, path)
}

fn validate_hex(value: &toml::Value, field: &str, path: &Path) -> Result<(), String> {
    match value.as_integer() {
        Some(value) if (0..=0xFF_FFFF).contains(&value) => Ok(()),
        _ => Err(format!(
            "`{field}` must be a 0x000000..=0xFFFFFF color: {}",
            path.display()
        )),
    }
}
