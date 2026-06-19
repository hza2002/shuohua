use std::path::{Path, PathBuf};

use crate::config::spec::{Diagnostic, Severity};

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

pub(super) type ConfigDiagnosticReportResult<T> = Result<T, ConfigDiagnosticReport>;

pub(super) fn push_spec_diagnostics(
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

pub(super) fn push_error(
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

pub(super) fn diagnostic_report_error(
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
