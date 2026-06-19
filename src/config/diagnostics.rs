use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::config::profile::Profile;
use crate::config::schema::{self, SchemaId};
use crate::config::spec::{validate_value, ConfigSpec, Diagnostic, Severity};
use crate::config::{Config, ProfileRouteCfg};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticScope {
    Main,
    Profile,
    AsrProvider,
    PostProcessor,
    Theme,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDiagnostic {
    pub scope: DiagnosticScope,
    pub source: PathBuf,
    pub severity: Severity,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDiagnosticReport {
    pub root: PathBuf,
    pub diagnostics: Vec<ConfigDiagnostic>,
    pub files_checked: usize,
}

impl ConfigDiagnosticReport {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeCheckPlan {
    pub root: PathBuf,
    pub profiles: Vec<RuntimeProfileCheck>,
}

impl RuntimeCheckPlan {
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    pub fn asr_targets(&self) -> Vec<AsrRuntimeTarget> {
        let mut targets: BTreeMap<String, AsrRuntimeTarget> = BTreeMap::new();
        for profile in &self.profiles {
            let key = asr_runtime_key(
                &profile.asr_provider,
                &profile.hotwords,
                &profile.asr_overrides,
            );
            targets
                .entry(key)
                .and_modify(|target| target.profiles.push(profile.name.clone()))
                .or_insert_with(|| AsrRuntimeTarget {
                    profiles: vec![profile.name.clone()],
                    provider: profile.asr_provider.clone(),
                    hotwords: profile.hotwords.clone(),
                    overrides: profile.asr_overrides.clone(),
                });
        }
        targets.into_values().collect()
    }

    pub fn llm_targets(&self) -> Vec<LlmRuntimeTarget> {
        let mut targets: BTreeMap<String, LlmRuntimeTarget> = BTreeMap::new();
        for profile in &self.profiles {
            for component in &profile.llm_components {
                let key = llm_runtime_key(&component.id, &component.overrides);
                targets
                    .entry(key)
                    .and_modify(|target| target.profiles.extend(component.profiles.clone()))
                    .or_insert_with(|| component.clone());
            }
        }
        targets.into_values().collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeProfileCheck {
    pub name: String,
    pub asr_provider: String,
    pub hotwords: Vec<String>,
    pub asr_overrides: toml::value::Table,
    pub llm_components: Vec<LlmRuntimeTarget>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsrRuntimeTarget {
    pub profiles: Vec<String>,
    pub provider: String,
    pub hotwords: Vec<String>,
    pub overrides: toml::value::Table,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmRuntimeTarget {
    pub profiles: Vec<String>,
    pub id: String,
    pub overrides: toml::value::Table,
}

pub fn run_local() -> ConfigDiagnosticReport {
    run_local_from_config_home(&config_home())
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

pub fn runtime_check_plan() -> ConfigDiagnosticReportResult<RuntimeCheckPlan> {
    runtime_check_plan_from_config_home(&config_home())
}

type ConfigDiagnosticReportResult<T> = Result<T, ConfigDiagnosticReport>;

pub fn runtime_check_plan_from_config_home(
    config_home: &Path,
) -> ConfigDiagnosticReportResult<RuntimeCheckPlan> {
    let report = run_local_from_config_home(config_home);
    if report.has_errors() {
        return Err(report);
    }

    let root = config_home.join("shuohua");
    let mut profiles = Vec::new();
    for path in toml_files(&root.join("profile")) {
        let body = std::fs::read_to_string(&path).map_err(|error| {
            diagnostic_report_error(
                &root,
                DiagnosticScope::Profile,
                &path,
                "",
                format!("read: {error}"),
            )
        })?;
        let profile: Profile = toml::from_str(&body).map_err(|error| {
            diagnostic_report_error(
                &root,
                DiagnosticScope::Profile,
                &path,
                "",
                format!("parse profile: {error}"),
            )
        })?;
        let profile_name = profile.name.clone();
        profiles.push(RuntimeProfileCheck {
            name: profile.name,
            asr_provider: profile.asr.provider,
            hotwords: profile.asr.hotwords,
            asr_overrides: profile.asr.overrides,
            llm_components: profile
                .post
                .chain
                .iter()
                .filter_map(|item| item.strip_prefix("llm:"))
                .map(|name| LlmRuntimeTarget {
                    profiles: vec![profile_name.clone()],
                    id: format!("llm:{name}"),
                    overrides: profile
                        .post
                        .llm
                        .get(name)
                        .and_then(toml::Value::as_table)
                        .cloned()
                        .unwrap_or_default(),
                })
                .collect(),
        });
    }

    Ok(RuntimeCheckPlan { root, profiles })
}

fn asr_runtime_key(provider: &str, hotwords: &[String], overrides: &toml::value::Table) -> String {
    format!(
        "{}|hotwords={:?}|overrides={:?}",
        provider, hotwords, overrides
    )
}

fn llm_runtime_key(id: &str, overrides: &toml::value::Table) -> String {
    format!("{id}|overrides={overrides:?}")
}

fn config_home() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
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
            let path = root.join("asr/doubao.toml");
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

    for item in &profile.post.chain {
        let Some((kind, name)) = item.split_once(':') else {
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
            "rule" => root.join("post/rule").join(format!("{name}.toml")),
            "llm" => root.join("post/llm").join(format!("{name}.toml")),
            other => {
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
            push_error(
                report,
                DiagnosticScope::Profile,
                source,
                "post.chain",
                format!("missing post component {}", path.display()),
            );
        }
    }
}

fn push_spec_diagnostics(
    report: &mut ConfigDiagnosticReport,
    scope: DiagnosticScope,
    source: &Path,
    diagnostics: Vec<Diagnostic>,
) {
    report
        .diagnostics
        .extend(diagnostics.into_iter().map(|diagnostic| ConfigDiagnostic {
            scope,
            source: source.to_path_buf(),
            severity: diagnostic.severity,
            path: diagnostic.path,
            message: diagnostic.message,
        }));
}

fn push_error(
    report: &mut ConfigDiagnosticReport,
    scope: DiagnosticScope,
    source: &Path,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    report.diagnostics.push(ConfigDiagnostic {
        scope,
        source: source.to_path_buf(),
        severity: Severity::Error,
        path: path.into(),
        message: message.into(),
    });
}

fn diagnostic_report_error(
    root: &Path,
    scope: DiagnosticScope,
    source: &Path,
    path: impl Into<String>,
    message: impl Into<String>,
) -> ConfigDiagnosticReport {
    ConfigDiagnosticReport {
        root: root.to_path_buf(),
        files_checked: 1,
        diagnostics: vec![ConfigDiagnostic {
            scope,
            source: source.to_path_buf(),
            severity: Severity::Error,
            path: path.into(),
            message: message.into(),
        }],
    }
}

fn toml_files(dir: &Path) -> Vec<PathBuf> {
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn temp_config_home() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("shuohua-diagnostics-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn local_diagnostics_scans_unreferenced_profile_asr_and_post_files() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("asr")).unwrap();
        fs::create_dir_all(root.join("post/llm")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(root.join("profile/broken.toml"), "[asr\n").unwrap();
        fs::write(
            root.join("asr/apple.toml"),
            "idle_pause = true\nunknown = 1\n",
        )
        .unwrap();
        fs::write(
            root.join("post/llm/broken.toml"),
            "type = \"llm\"\napi_key = \"\"\n",
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.files_checked >= 4);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.source.ends_with("profile/broken.toml")));
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.source.ends_with("asr/apple.toml")));
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.source.ends_with("post/llm/broken.toml")));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn local_diagnostics_reports_profile_reference_errors() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("post/rule")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/default.toml"),
            r#"
name = "default"
[asr]
provider = "doubao"

[post]
chain = ["rule:missing", "llm:missing", "bad-item", "other:name"]
"#,
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.diagnostics.iter().any(|d| {
            d.scope == DiagnosticScope::Profile
                && d.source.ends_with("profile/default.toml")
                && d.path == "asr.provider"
                && d.message.contains("asr/doubao.toml")
        }));
        assert!(report.diagnostics.iter().any(|d| {
            d.scope == DiagnosticScope::Profile
                && d.source.ends_with("profile/default.toml")
                && d.path == "post.chain"
                && d.message.contains("post/rule/missing.toml")
        }));
        assert!(report.diagnostics.iter().any(|d| {
            d.scope == DiagnosticScope::Profile
                && d.source.ends_with("profile/default.toml")
                && d.path == "post.chain"
                && d.message.contains("post chain item")
        }));
        assert!(report.diagnostics.iter().any(|d| {
            d.scope == DiagnosticScope::Profile
                && d.source.ends_with("profile/default.toml")
                && d.path == "post.chain"
                && d.message.contains("unknown post component kind")
        }));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn local_diagnostics_reports_only_user_config_sources() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report
            .diagnostics
            .iter()
            .all(|d| d.source.starts_with(&root)));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn theme_diagnostics_accepts_macos_overlay_fields() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("theme")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("theme/custom.toml"),
            r#"
[overlay.macos]
glass_variant = 11
glass_style = "clear"
subdued = 0
background_blur_radius = 3
"#,
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(!report.diagnostics.iter().any(|d| {
            d.source.ends_with("theme/custom.toml")
                && d.severity == Severity::Warning
                && d.path.starts_with("overlay.macos")
                && d.message.contains("unknown")
        }));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn theme_diagnostics_rejects_legacy_overlay_fields() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("theme")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("theme/legacy.toml"),
            r#"
[overlay.glass]
variant = 11
style = "clear"
subdued = 0

[overlay.surface]
background_blur_radius = 3
"#,
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.diagnostics.iter().any(|d| {
            d.source.ends_with("theme/legacy.toml")
                && d.severity == Severity::Warning
                && d.path == "overlay.glass"
                && d.message.contains("unknown")
        }));
        assert!(report.diagnostics.iter().any(|d| {
            d.source.ends_with("theme/legacy.toml")
                && d.severity == Severity::Warning
                && d.path == "overlay.surface.background_blur_radius"
                && d.message.contains("unknown")
        }));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn unreferenced_llm_draft_empty_api_key_is_warning() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("post/llm")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("post/llm/draft.toml"),
            "type = \"llm\"\nname = \"draft\"\napi_key = \"\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.diagnostics.iter().any(|d| {
            d.source.ends_with("post/llm/draft.toml")
                && d.path == "api_key"
                && d.severity == Severity::Warning
        }));
        assert!(!report.diagnostics.iter().any(|d| {
            d.source.ends_with("post/llm/draft.toml")
                && d.path == "api_key"
                && d.severity == Severity::Error
        }));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn referenced_llm_empty_api_key_is_error() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("post/llm")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/default.toml"),
            "name = \"default\"\n[asr]\nprovider = \"apple\"\n[post]\nchain = [\"llm:draft\"]\n",
        )
        .unwrap();
        fs::write(
            root.join("post/llm/draft.toml"),
            "type = \"llm\"\nname = \"draft\"\napi_key = \"\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();

        let report = run_local_from_config_home(&home);

        assert!(report.diagnostics.iter().any(|d| {
            d.source.ends_with("post/llm/draft.toml")
                && d.path == "api_key"
                && d.severity == Severity::Error
        }));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn runtime_check_plan_lists_configured_asr_and_referenced_llm_targets() {
        let home = temp_config_home();
        let root = home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::create_dir_all(root.join("post/llm")).unwrap();
        fs::write(root.join("config.toml"), "[hotkey]\ntrigger = \"f16\"\n").unwrap();
        fs::write(
            root.join("profile/default.toml"),
            r#"
name = "default"
[asr]
provider = "apple"
language = "zh-CN"
hotwords = ["Rust"]

[post]
chain = ["llm:deepseek"]

[post.llm.deepseek]
model = "deepseek-chat"
"#,
        )
        .unwrap();
        fs::write(
            root.join("post/llm/deepseek.toml"),
            r#"
type = "llm"
format = "openai"
name = "deepseek"
api_key = "sk-test"
model = "deepseek-chat"
prompt = "{{text}}"
"#,
        )
        .unwrap();

        let plan = runtime_check_plan_from_config_home(&home).unwrap();

        assert_eq!(plan.profiles.len(), 1);
        assert_eq!(plan.asr_targets()[0].provider, "apple");
        assert_eq!(plan.asr_targets()[0].hotwords, vec!["Rust"]);
        assert_eq!(plan.llm_targets()[0].id, "llm:deepseek");
        assert_eq!(
            plan.llm_targets()[0]
                .overrides
                .get("model")
                .and_then(toml::Value::as_str),
            Some("deepseek-chat")
        );
        let _ = fs::remove_dir_all(home);
    }
}
