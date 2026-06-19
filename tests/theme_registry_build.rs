#[path = "../build_support/themes.rs"]
mod themes;

use std::path::{Path, PathBuf};

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shuohua-theme-build-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_theme(dir: &Path, file_name: &str, name: &str) {
    std::fs::write(
        dir.join(file_name),
        format!(
            r#"name = {name:?}

[palette]
bg = 0x000000

[tui]
foreground = "bg"
muted = "bg"
accent = "bg"
success = "bg"
warning = "bg"
error = "bg"
info = "bg"
highlight = "bg"
border = "bg"
border_focus = "bg"
segment = "bg"

[overlay.glass]
variant = 11
style = "clear"
subdued = 0

[overlay.surface]
background = "bg"
background_alpha = 0.7
background_blur_radius = 0
corner_radius = 18.0

[overlay.text]
primary = "bg"
secondary = "bg"
tertiary = "bg"
segment = "bg"
notice = "bg"
error = "bg"

[overlay.state]
idle = "bg"
connecting = "bg"
recording = "bg"
thinking = "bg"
stopping = "bg"
error = "bg"
"#
        ),
    )
    .unwrap();
}

#[test]
fn scans_themes_in_stable_file_name_order() {
    let dir = temp_dir();
    write_theme(&dir, "zeta.toml", "Zeta");
    write_theme(&dir, "alpha.toml", "Alpha");

    let themes = themes::scan_theme_dir(&dir).unwrap();

    assert_eq!(
        themes
            .iter()
            .map(|theme| theme.id.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );
    assert_eq!(themes[0].name, "Alpha");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejects_invalid_theme_file_names() {
    let dir = temp_dir();
    write_theme(&dir, "Bad_Name.toml", "Bad Name");

    let error = themes::scan_theme_dir(&dir).unwrap_err().to_string();

    assert!(error.contains("lowercase kebab-case"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejects_duplicate_display_names_case_insensitively() {
    let dir = temp_dir();
    write_theme(&dir, "first.toml", "Shared Name");
    write_theme(&dir, "second.toml", "shared name");

    let error = themes::scan_theme_dir(&dir).unwrap_err().to_string();

    assert!(error.contains("duplicate theme name"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejects_missing_or_padded_theme_names() {
    let dir = temp_dir();
    std::fs::write(dir.join("missing.toml"), "[palette]\nbg = 0\n").unwrap();

    let missing = themes::scan_theme_dir(&dir).unwrap_err().to_string();
    assert!(missing.contains("string `name`"), "{missing}");

    std::fs::remove_file(dir.join("missing.toml")).unwrap();
    write_theme(&dir, "padded.toml", " Padded ");
    let padded = themes::scan_theme_dir(&dir).unwrap_err().to_string();
    assert!(
        padded.contains("leading or trailing whitespace"),
        "{padded}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejects_unknown_palette_references() {
    let dir = temp_dir();
    write_theme(&dir, "broken.toml", "Broken");
    let path = dir.join("broken.toml");
    let body = std::fs::read_to_string(&path)
        .unwrap()
        .replace("foreground = \"bg\"", "foreground = \"missing\"");
    std::fs::write(&path, body).unwrap();

    let error = themes::scan_theme_dir(&dir).unwrap_err().to_string();

    assert!(error.contains("unknown palette color `missing`"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn generated_registry_embeds_every_theme() {
    let dir = temp_dir();
    write_theme(&dir, "alpha.toml", "Alpha");
    let themes = themes::scan_theme_dir(&dir).unwrap();

    let generated = themes::generate_registry(&themes);

    assert!(generated.contains("id: \"alpha\""));
    assert!(generated.contains("path: \"theme/alpha.toml\""));
    assert!(generated.contains("/assets/themes/alpha.toml"));
    let _ = std::fs::remove_dir_all(dir);
}
