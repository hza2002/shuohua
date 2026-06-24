use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config::diagnostics::report::{
    push_error, push_spec_diagnostics, ConfigDiagnosticReport, DiagnosticScope,
};
use crate::config::profile::Profile;
use crate::config::schema::{self, SchemaId};
use crate::config::spec::{validate_value, ConfigSpec, Severity};
use crate::config::{Config, ProfileRouteCfg};

pub fn run_local() -> ConfigDiagnosticReport {
    run_local_from_config_home(&crate::config::paths::config_home())
}

pub fn run_local_from_config_home(config_home: &Path) -> ConfigDiagnosticReport {
    let root = config_home.join("shuohua");
    let mut report = ConfigDiagnosticReport {
        root: root.clone(),
        diagnostics: Vec::new(),
        files_checked: 0,
    };
    let main = scan_main(&mut report, &root);
    let profiles = scan_profiles(&mut report, &root, main.as_ref());
    scan_asr(&mut report, &root);
    scan_post(&mut report, &root, &referenced_llm_components(&profiles));
    scan_theme(&mut report, &root);
    report
}

fn scan_main(report: &mut ConfigDiagnosticReport, root: &Path) -> Option<Config> {
    let path = root.join("config.toml");
    let value = read_parse_validate(
        report,
        DiagnosticScope::Main,
        &path,
        &schema::spec_for(SchemaId::Main),
    )?;
    match value.try_into::<Config>() {
        Ok(config) => Some(config),
        Err(error) => {
            push_error(
                report,
                DiagnosticScope::Main,
                &path,
                "",
                format!("parse config: {error}"),
            );
            None
        }
    }
}

fn scan_profiles(
    report: &mut ConfigDiagnosticReport,
    root: &Path,
    main: Option<&Config>,
) -> Vec<Profile> {
    let mut profiles = Vec::new();
    for path in toml_files(&root.join("profile")) {
        let Some(value) = read_parse_validate(
            report,
            DiagnosticScope::Profile,
            &path,
            &schema::spec_for(SchemaId::Profile),
        ) else {
            continue;
        };
        match value.try_into::<Profile>() {
            Ok(profile) => {
                validate_profile_references(report, root, &path, &profile);
                profiles.push(profile);
            }
            Err(error) => push_error(
                report,
                DiagnosticScope::Profile,
                &path,
                "",
                format!("parse profile: {error}"),
            ),
        }
    }

    if let Some(main) = main {
        validate_profile_routes(report, root, &main.profile);
    }
    profiles
}

fn scan_asr(report: &mut ConfigDiagnosticReport, root: &Path) {
    for path in toml_files(&root.join("asr")) {
        let spec = match path.file_stem().and_then(|name| name.to_str()) {
            Some("apple") => schema::spec_for(SchemaId::AsrApple),
            Some("doubao") => schema::spec_for(SchemaId::AsrDoubao),
            Some(provider) => {
                report.files_checked += 1;
                push_error(
                    report,
                    DiagnosticScope::AsrProvider,
                    &path,
                    "",
                    format!("unknown ASR provider file {provider:?}"),
                );
                continue;
            }
            None => continue,
        };
        let _ = read_parse_validate(report, DiagnosticScope::AsrProvider, &path, &spec);
    }
}

fn scan_post(report: &mut ConfigDiagnosticReport, root: &Path, referenced_llm: &BTreeSet<String>) {
    for path in toml_files(&root.join("post/rule")) {
        let _ = read_parse_validate(
            report,
            DiagnosticScope::PostProcessor,
            &path,
            &schema::spec_for(SchemaId::PostRule),
        );
    }
    for path in toml_files(&root.join("post/llm")) {
        let spec = schema::spec_for(SchemaId::PostLlm);
        if let Some(value) =
            read_parse_validate_value(report, DiagnosticScope::PostProcessor, &path)
        {
            let name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            let diagnostics = validate_value(&spec, &value)
                .into_iter()
                .map(|mut diagnostic| {
                    if !referenced_llm.contains(name)
                        && diagnostic.path == "api_key"
                        && diagnostic.message.contains("empty")
                    {
                        diagnostic.severity = Severity::Warning;
                        diagnostic.message =
                            "draft LLM component api_key is empty; fill it before adding to a profile chain"
                                .to_string();
                    }
                    diagnostic
                })
                .collect();
            push_spec_diagnostics(report, DiagnosticScope::PostProcessor, &path, diagnostics);
        }
    }
}

fn scan_theme(report: &mut ConfigDiagnosticReport, root: &Path) {
    for path in toml_files(&root.join("theme")) {
        let Some(value) = read_parse_validate(
            report,
            DiagnosticScope::Theme,
            &path,
            &schema::spec_for(SchemaId::Theme),
        ) else {
            continue;
        };
        match value.try_into::<crate::config::theme::ThemeFile>() {
            Ok(theme) => {
                if let Err(error) = crate::config::theme::validate_theme_file(&theme) {
                    push_error(report, DiagnosticScope::Theme, &path, "", error.to_string());
                }
            }
            Err(error) => push_error(
                report,
                DiagnosticScope::Theme,
                &path,
                "",
                format!("parse theme: {error}"),
            ),
        }
    }
}

fn read_parse_validate(
    report: &mut ConfigDiagnosticReport,
    scope: DiagnosticScope,
    path: &Path,
    spec: &ConfigSpec,
) -> Option<toml::Value> {
    if !path.exists() {
        return None;
    }
    report.files_checked += 1;
    let body = match std::fs::read_to_string(path) {
        Ok(body) => body,
        Err(error) => {
            push_error(report, scope, path, "", format!("read: {error}"));
            return None;
        }
    };
    let value = match toml::from_str::<toml::Value>(&body) {
        Ok(value) => value,
        Err(error) => {
            push_error(report, scope, path, "", format!("parse TOML: {error}"));
            return None;
        }
    };
    push_spec_diagnostics(report, scope, path, validate_value(spec, &value));
    Some(value)
}

fn read_parse_validate_value(
    report: &mut ConfigDiagnosticReport,
    scope: DiagnosticScope,
    path: &Path,
) -> Option<toml::Value> {
    if !path.exists() {
        return None;
    }
    report.files_checked += 1;
    let body = match std::fs::read_to_string(path) {
        Ok(body) => body,
        Err(error) => {
            push_error(report, scope, path, "", format!("read: {error}"));
            return None;
        }
    };
    match toml::from_str::<toml::Value>(&body) {
        Ok(value) => Some(value),
        Err(error) => {
            push_error(report, scope, path, "", format!("parse TOML: {error}"));
            None
        }
    }
}

fn referenced_llm_components(profiles: &[Profile]) -> BTreeSet<String> {
    profiles
        .iter()
        .flat_map(|profile| profile.post.chain.iter())
        .filter_map(|item| item.strip_prefix("llm:"))
        .map(str::to_string)
        .collect()
}

fn validate_profile_routes(
    report: &mut ConfigDiagnosticReport,
    root: &Path,
    routes: &ProfileRouteCfg,
) {
    if !root
        .join("profile")
        .join(format!("{}.toml", routes.default))
        .exists()
    {
        push_error(
            report,
            DiagnosticScope::Main,
            &root.join("config.toml"),
            "profile.default",
            format!(
                "missing profile {}",
                root.join("profile")
                    .join(format!("{}.toml", routes.default))
                    .display()
            ),
        );
    }

    for profile in routes.routes.keys() {
        let path = root.join("profile").join(format!("{profile}.toml"));
        if !path.exists() {
            push_error(
                report,
                DiagnosticScope::Main,
                &root.join("config.toml"),
                "profile",
                format!("missing profile {}", path.display()),
            );
        }
    }
}

fn validate_profile_references(
    report: &mut ConfigDiagnosticReport,
    root: &Path,
    source: &Path,
    profile: &Profile,
) {
    match profile.asr.provider.as_str() {
        "apple" => {}
        "doubao" => {
            let path = root.join("asr").join("doubao.toml");
            if !path.exists() {
                push_error(
                    report,
                    DiagnosticScope::Profile,
                    source,
                    "asr.provider",
                    format!("missing ASR provider config {}", path.display()),
                );
            }
        }
        provider => push_error(
            report,
            DiagnosticScope::Profile,
            source,
            "asr.provider",
            format!("unknown ASR provider {provider:?}"),
        ),
    }

    let mut has_post_reference_error = false;
    for item in &profile.post.chain {
        let Some((kind, name)) = item.split_once(':') else {
            has_post_reference_error = true;
            push_error(
                report,
                DiagnosticScope::Profile,
                source,
                "post.chain",
                format!("post chain item {item:?} must be kind:name"),
            );
            continue;
        };
        let path = match kind {
            "rule" => root.join("post").join("rule").join(format!("{name}.toml")),
            "llm" => root.join("post").join("llm").join(format!("{name}.toml")),
            other => {
                has_post_reference_error = true;
                push_error(
                    report,
                    DiagnosticScope::Profile,
                    source,
                    "post.chain",
                    format!("unknown post component kind {other:?}"),
                );
                continue;
            }
        };
        if !path.exists() {
            has_post_reference_error = true;
            push_error(
                report,
                DiagnosticScope::Profile,
                source,
                "post.chain",
                format!("missing post component {}", path.display()),
            );
        }
    }

    if !has_post_reference_error {
        let post_root = root.join("post");
        if let Err(error) = crate::config::post::load_components(
            &profile.post.chain,
            &crate::config::post::PostDirs {
                rule: post_root.join("rule"),
                llm: post_root.join("llm"),
            },
            &profile.post.llm,
        ) {
            push_error(
                report,
                DiagnosticScope::Profile,
                source,
                "post",
                format!("invalid post chain config: {error:#}"),
            );
        }
    }
}

pub(super) fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut paths = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    paths.sort();
    paths
}
