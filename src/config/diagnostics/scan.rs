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
    scan_post(
        &mut report,
        &root,
        &referenced_llm_components(&root, &profiles),
    );
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
        if !validate_file_stem(report, DiagnosticScope::Profile, &path) {
            continue;
        }
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
        if !validate_file_stem(report, DiagnosticScope::AsrProvider, &path) {
            continue;
        }
        let Some(value) = read_parse_validate_value(report, DiagnosticScope::AsrProvider, &path)
        else {
            continue;
        };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        match crate::config::asr::instance::kind_from_value(stem, &path, &value) {
            Ok(kind) => {
                let spec = schema::spec_for(kind.schema_id());
                push_spec_diagnostics(
                    report,
                    DiagnosticScope::AsrProvider,
                    &path,
                    validate_value(&spec, &value),
                );
            }
            Err(error) => push_error(
                report,
                DiagnosticScope::AsrProvider,
                &path,
                "type",
                error.to_string(),
            ),
        }
    }
}

fn scan_post(report: &mut ConfigDiagnosticReport, root: &Path, referenced_llm: &BTreeSet<String>) {
    for path in toml_files(&root.join("post")) {
        if !validate_file_stem(report, DiagnosticScope::PostProcessor, &path) {
            continue;
        }
        let Some(value) = read_parse_validate_value(report, DiagnosticScope::PostProcessor, &path)
        else {
            continue;
        };
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let kind = match crate::config::post::kind_from_value(name, &path, &value) {
            Ok(kind) => kind,
            Err(error) => {
                push_error(
                    report,
                    DiagnosticScope::PostProcessor,
                    &path,
                    "type",
                    error.to_string(),
                );
                continue;
            }
        };
        let spec = schema::spec_for(kind.schema_id());
        let diagnostics = validate_value(&spec, &value)
            .into_iter()
            .map(|mut diagnostic| {
                if kind == crate::config::post::PostKind::Llm
                    && !referenced_llm.contains(name)
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

fn scan_theme(report: &mut ConfigDiagnosticReport, root: &Path) {
    for path in toml_files(&root.join("theme")) {
        if !validate_file_stem(report, DiagnosticScope::Theme, &path) {
            continue;
        }
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

fn validate_file_stem(
    report: &mut ConfigDiagnosticReport,
    scope: DiagnosticScope,
    path: &Path,
) -> bool {
    match crate::config::inventory::validate_config_file_stem(path) {
        Ok(()) => true,
        Err(error) => {
            report.files_checked += 1;
            push_error(report, scope, path, "", error);
            false
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

fn referenced_llm_components(root: &Path, profiles: &[Profile]) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for profile in profiles {
        for id in &profile.post.chain {
            if matches!(
                crate::config::post::resolve_kind_in_root(root, id),
                Some(crate::config::post::PostKind::Llm)
            ) {
                set.insert(id.clone());
            }
        }
    }
    set
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
    if let Err(error) =
        crate::config::asr::instance::resolve_instance_in_root(root, &profile.asr.instance)
    {
        push_error(
            report,
            DiagnosticScope::Profile,
            source,
            "asr.instance",
            error.to_string(),
        );
    }

    let mut has_post_reference_error = false;
    for id in &profile.post.chain {
        let path = root.join("post").join(format!("{id}.toml"));
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

    for key in profile.post.overrides.keys() {
        if !profile.post.chain.iter().any(|id| id == key) {
            push_error(
                report,
                DiagnosticScope::Profile,
                source,
                "post.overrides",
                format!("post.overrides.{key} is not in the chain"),
            );
        } else if !matches!(
            crate::config::post::resolve_kind_in_root(root, key),
            Some(crate::config::post::PostKind::Llm)
        ) {
            push_error(
                report,
                DiagnosticScope::Profile,
                source,
                "post.overrides",
                format!("post.overrides.{key} targets a non-llm component"),
            );
        }
    }

    if !has_post_reference_error {
        if let Err(error) = crate::config::post::load_components(
            &profile.post.chain,
            &crate::config::post::PostDir {
                dir: root.join("post"),
            },
            &profile.post.overrides,
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
