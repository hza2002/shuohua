use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::diagnostics::report::{
    diagnostic_report_error, ConfigDiagnosticReportResult, DiagnosticScope,
};
use crate::config::diagnostics::scan::{run_local_from_config_home, toml_files};
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
                &profile.asr_instance.id,
                &profile.hotwords,
                &profile.asr_overrides,
            );
            targets
                .entry(key)
                .and_modify(|target| target.profiles.push(profile.name.clone()))
                .or_insert_with(|| AsrRuntimeTarget {
                    profiles: vec![profile.name.clone()],
                    instance: profile.asr_instance.clone(),
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
    pub asr_instance: crate::config::asr::instance::AsrInstance,
    pub hotwords: Vec<String>,
    pub asr_overrides: toml::value::Table,
    pub llm_components: Vec<LlmRuntimeTarget>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsrRuntimeTarget {
    pub profiles: Vec<String>,
    pub instance: crate::config::asr::instance::AsrInstance,
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
    runtime_check_plan_from_config_home(&crate::config::paths::config_home())
}

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
        let mut profile: Profile = toml::from_str(&body).map_err(|error| {
            diagnostic_report_error(
                &root,
                DiagnosticScope::Profile,
                &path,
                "",
                format!("parse profile: {error}"),
            )
        })?;
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            profile.id = stem.to_string();
        }
        let instance =
            crate::config::asr::instance::resolve_instance_in_root(&root, &profile.asr.instance)
                .map_err(|error| {
                    diagnostic_report_error(
                        &root,
                        DiagnosticScope::Profile,
                        &path,
                        "asr.instance",
                        error.to_string(),
                    )
                })?;
        let profile_name = profile.display_name();
        profiles.push(RuntimeProfileCheck {
            name: profile_name.clone(),
            asr_instance: instance,
            hotwords: profile.asr.hotwords,
            asr_overrides: profile.asr.overrides,
            llm_components: profile
                .post
                .chain
                .iter()
                .filter(|id| {
                    matches!(
                        crate::config::post::resolve_kind_in_root(&root, id),
                        Some(crate::config::post::PostKind::Llm)
                    )
                })
                .map(|id| LlmRuntimeTarget {
                    profiles: vec![profile_name.clone()],
                    id: id.clone(),
                    overrides: profile
                        .post
                        .overrides
                        .get(id)
                        .and_then(toml::Value::as_table)
                        .cloned()
                        .unwrap_or_default(),
                })
                .collect(),
        });
    }

    Ok(RuntimeCheckPlan { root, profiles })
}

fn asr_runtime_key(
    instance_id: &str,
    hotwords: &[String],
    overrides: &toml::value::Table,
) -> String {
    format!(
        "{}|hotwords={:?}|overrides={:?}",
        instance_id, hotwords, overrides
    )
}

fn llm_runtime_key(id: &str, overrides: &toml::value::Table) -> String {
    format!("{id}|overrides={overrides:?}")
}
