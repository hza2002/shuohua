#[path = "build_support/themes.rs"]
mod themes;

fn main() {
    println!("cargo:rerun-if-changed=src/asr/providers/apple_helper.swift");
    println!("cargo:rerun-if-changed=assets/themes");
    println!("cargo:rerun-if-changed=assets/i18n/zh-CN.toml");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=ApplicationServices");
    println!("cargo:rustc-link-lib=framework=QuartzCore");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let themes = themes::scan_theme_dir(std::path::Path::new("assets/themes"))
        .unwrap_or_else(|error| panic!("invalid built-in themes: {error}"));
    std::fs::write(
        std::path::Path::new(&out_dir).join("embedded_themes.rs"),
        themes::generate_registry(&themes),
    )
    .expect("write embedded theme registry");
    generate_zh_variants(std::path::Path::new(&out_dir));

    let helper_out = std::path::Path::new(&out_dir).join("apple_helper");
    let status = std::process::Command::new("xcrun")
        .args([
            "swiftc",
            "-O",
            "-parse-as-library",
            "-o",
            helper_out.to_str().expect("helper path is utf-8"),
            "src/asr/providers/apple_helper.swift",
        ])
        .status()
        .expect("run xcrun swiftc for apple_helper");
    if !status.success() {
        panic!("swiftc failed building apple_helper");
    }
}

fn generate_zh_variants(out_dir: &std::path::Path) {
    use ferrous_opencc::config::BuiltinConfig;

    let body = std::fs::read_to_string("assets/i18n/zh-CN.toml").expect("read zh-CN i18n asset");
    let value = body
        .parse::<toml::Value>()
        .expect("embedded zh-CN i18n TOML must parse");
    let flattened = flatten_i18n_toml(&value);
    let variants = [
        ("i18n_zh_hant.rs", BuiltinConfig::S2t),
        ("i18n_zh_tw.rs", BuiltinConfig::S2twp),
        ("i18n_zh_hk.rs", BuiltinConfig::S2hk),
    ];

    for (file_name, config) in variants {
        let opencc = ferrous_opencc::OpenCC::from_config(config)
            .unwrap_or_else(|error| panic!("create OpenCC converter {config:?}: {error}"));
        let entries = flattened
            .iter()
            .map(|(key, value)| (key.clone(), convert_preserving_placeholders(&opencc, value)))
            .collect::<Vec<_>>();
        std::fs::write(out_dir.join(file_name), generate_i18n_entries(&entries))
            .unwrap_or_else(|error| panic!("write generated i18n asset {file_name}: {error}"));
    }
}

fn flatten_i18n_toml(value: &toml::Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    flatten_i18n_value(None, value, &mut out);
    out.sort_by(|left, right| left.0.cmp(&right.0));
    out
}

fn flatten_i18n_value(prefix: Option<&str>, value: &toml::Value, out: &mut Vec<(String, String)>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, value) in table {
        let full_key = match prefix {
            Some(prefix) => format!("{prefix}.{key}"),
            None => key.to_string(),
        };
        if let Some(text) = value.as_str() {
            out.push((full_key, text.to_string()));
        } else {
            flatten_i18n_value(Some(&full_key), value, out);
        }
    }
}

fn convert_preserving_placeholders(opencc: &ferrous_opencc::OpenCC, text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        out.push_str(&opencc.convert(&rest[..start]));
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('}') else {
            out.push_str(&opencc.convert(&rest[start..]));
            return out;
        };
        let name = &after_start[..end];
        if is_placeholder_name(name) {
            out.push('{');
            out.push_str(name);
            out.push('}');
        } else {
            out.push_str(&opencc.convert(&rest[start..start + end + 2]));
        }
        rest = &after_start[end + 1..];
    }
    out.push_str(&opencc.convert(rest));
    out
}

fn is_placeholder_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn generate_i18n_entries(entries: &[(String, String)]) -> String {
    let mut out = String::from("&[\n");
    for (key, value) in entries {
        out.push_str("    (");
        out.push_str(&rust_string_literal(key));
        out.push_str(", ");
        out.push_str(&rust_string_literal(value));
        out.push_str("),\n");
    }
    out.push(']');
    out
}

fn rust_string_literal(value: &str) -> String {
    format!("{value:?}")
}
