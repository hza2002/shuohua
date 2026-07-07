//! ASR provider 实现。每个 provider 一个子模块。

use std::sync::Arc;

use anyhow::{Context, Result};

use crate::asr::AsrProvider;
use crate::config::asr::instance::{resolve_instance_in_root, AsrInstance, AsrKind};
use crate::config::asr::LocalVadMode;

#[cfg(target_os = "macos")]
pub mod apple;
pub mod doubao;
pub mod tencent;

pub struct ProviderRuntime {
    pub provider: Arc<dyn AsrProvider>,
    pub options: ProviderOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderOptions {
    pub local_vad: LocalVadMode,
    pub open_timeout_ms: u64,
    pub finalize_timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeCheckNotice {
    pub line: &'static str,
}

pub fn build_instance(id: &str, overrides: &toml::value::Table) -> Result<ProviderRuntime> {
    build_in_root(&crate::config::paths::root_dir(), id, overrides)
}

pub(crate) fn build_in_root(
    root: &std::path::Path,
    id: &str,
    overrides: &toml::value::Table,
) -> Result<ProviderRuntime> {
    let instance = resolve_instance_in_root(root, id).context("resolve ASR instance")?;
    build_from_instance(&instance, overrides)
}

pub fn build_from_instance(
    instance: &AsrInstance,
    overrides: &toml::value::Table,
) -> Result<ProviderRuntime> {
    match instance.kind {
        AsrKind::Apple => build_apple(&instance.path, overrides),
        AsrKind::Doubao => {
            let provider = doubao::DoubaoProvider::new_from_path_with_overrides(
                &instance.path,
                Some(overrides),
            )
            .with_context(|| format!("init doubao provider from {}", instance.path.display()))?;
            Ok(ProviderRuntime {
                options: provider.options(),
                provider: Arc::new(provider),
            })
        }
        AsrKind::Tencent => {
            let provider = tencent::TencentProvider::new_from_path_with_overrides(
                &instance.path,
                Some(overrides),
            )
            .with_context(|| format!("init tencent provider from {}", instance.path.display()))?;
            Ok(ProviderRuntime {
                options: provider.options(),
                provider: Arc::new(provider),
            })
        }
    }
}

#[cfg(target_os = "macos")]
fn build_apple(path: &std::path::Path, overrides: &toml::value::Table) -> Result<ProviderRuntime> {
    let provider = apple::AppleProvider::new_from_path_with_overrides(path, Some(overrides))
        .with_context(|| format!("init apple provider from {}", path.display()))?;
    Ok(ProviderRuntime {
        options: provider.options(),
        provider: Arc::new(provider),
    })
}

#[cfg(not(target_os = "macos"))]
fn build_apple(
    _path: &std::path::Path,
    _overrides: &toml::value::Table,
) -> Result<ProviderRuntime> {
    anyhow::bail!("Apple ASR provider is only implemented on macOS")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect_err(result: Result<ProviderRuntime>) -> String {
        match result {
            Ok(_) => panic!("expected error but got Ok"),
            Err(e) => format!("{e:#}"),
        }
    }

    #[test]
    fn build_instance_reports_missing_file_path() {
        let root = std::env::temp_dir().join(format!("shuohua-build-{}", ulid::Ulid::new()));
        let error = expect_err(build_in_root(&root, "team", &toml::value::Table::new()));
        assert!(error.contains("asr/team.toml"), "{error}");
    }

    #[test]
    fn build_instance_uses_referenced_file_not_type_named_file() {
        let root = std::env::temp_dir().join(format!("shuohua-build-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(root.join("asr")).unwrap();
        std::fs::write(
            root.join("asr/team.toml"),
            "type = \"doubao\"\napp_key = \"\"\naccess_key = \"\"\n",
        )
        .unwrap();
        let error = expect_err(build_in_root(&root, "team", &toml::value::Table::new()));
        assert!(error.contains("team.toml"), "{error}");
        assert!(!error.contains("doubao.toml"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn build_rejects_unknown_provider() {
        // "missing" is a valid id format but has no backing file — resolver reports missing file
        let root = std::env::temp_dir().join(format!("shuohua-build-{}", ulid::Ulid::new()));
        let error = expect_err(build_in_root(&root, "missing", &toml::value::Table::new()));
        assert!(error.contains("missing"), "{error}");
    }

    #[test]
    fn build_rejects_invalid_provider_file_id() {
        let root = std::env::temp_dir().join(format!("shuohua-build-{}", ulid::Ulid::new()));
        let error = expect_err(build_in_root(&root, "BadName", &toml::value::Table::new()));
        assert!(error.contains("lowercase letter first"), "{error}");
    }
}
