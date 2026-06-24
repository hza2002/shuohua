use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::diagnostics::report::{
    diagnostic_report_error, ConfigDiagnosticReportResult, DiagnosticScope,
};
use crate::config::diagnostics::scan::{
    run_local_from_config_home, run_local_from_config_root, toml_files,
};
use crate::config::profile::Profile;

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

pub fn runtime_check_plan() -> ConfigDiagnosticReportResult<RuntimeCheckPlan> {
    runtime_check_plan_from_config_root(&crate::config::paths::config_root())
}

pub fn runtime_check_plan_from_config_home(
    config_home: &Path,
) -> ConfigDiagnosticReportResult<RuntimeCheckPlan> {
    let report = run_local_from_config_home(config_home);
    if report.has_errors() {
        return Err(report);
    }

    let root = config_home.join("shuohua");
    runtime_check_plan_from_checked_root(root)
}

pub fn runtime_check_plan_from_config_root(
    root: &Path,
) -> ConfigDiagnosticReportResult<RuntimeCheckPlan> {
    let report = run_local_from_config_root(root);
    if report.has_errors() {
        return Err(report);
    }
    runtime_check_plan_from_checked_root(root.to_path_buf())
}

fn runtime_check_plan_from_checked_root(
    root: PathBuf,
) -> ConfigDiagnosticReportResult<RuntimeCheckPlan> {
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
