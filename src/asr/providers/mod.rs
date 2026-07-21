//! ASR provider 实现。每个 provider 一个子模块。

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::asr::types::AsrError;
use crate::asr::AsrProvider;
use crate::config::asr::instance::{resolve_instance_in_root, AsrInstance, AsrKind};
use crate::config::asr::LocalVadMode;

pub mod aliyun;
#[cfg(target_os = "macos")]
pub mod apple;
pub mod doubao;
pub mod tencent;

pub(crate) const SESSION_IO_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) async fn bounded_session_io<T, E>(
    cancel: &tokio_util::sync::CancellationToken,
    operation: &'static str,
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, AsrError>
where
    E: std::fmt::Display,
{
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(AsrError::Canceled),
        result = tokio::time::timeout(SESSION_IO_TIMEOUT, future) => match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(AsrError::Network(format!("{operation}: {error}"))),
            Err(_) => Err(AsrError::TransportTimeout),
        },
    }
}

pub(crate) async fn send_session_command<T: Send>(
    tx: &tokio::sync::mpsc::Sender<T>,
    command: T,
    ended_message: &'static str,
) -> Result<(), AsrError> {
    match tokio::time::timeout(SESSION_IO_TIMEOUT, tx.send(command)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(AsrError::Network(ended_message.into())),
        Err(_) => Err(AsrError::TransportTimeout),
    }
}

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
        AsrKind::Aliyun => {
            let provider = aliyun::AliyunProvider::new_from_path_with_overrides(
                &instance.path,
                Some(overrides),
            )
            .with_context(|| format!("init aliyun provider from {}", instance.path.display()))?;
            Ok(ProviderRuntime {
                options: provider.options(),
                provider: Arc::new(provider),
            })
        }
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
        let root = std::env::temp_dir().join(format!("shuohua-build-{}", ulid::Ulid::generate()));
        let error = expect_err(build_in_root(&root, "team", &toml::value::Table::new()));
        assert!(error.contains("asr/team.toml"), "{error}");
    }

    #[test]
    fn build_instance_uses_referenced_file_not_type_named_file() {
        let root = std::env::temp_dir().join(format!("shuohua-build-{}", ulid::Ulid::generate()));
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
        let root = std::env::temp_dir().join(format!("shuohua-build-{}", ulid::Ulid::generate()));
        let error = expect_err(build_in_root(&root, "missing", &toml::value::Table::new()));
        assert!(error.contains("missing"), "{error}");
    }

    #[test]
    fn build_rejects_invalid_provider_file_id() {
        let root = std::env::temp_dir().join(format!("shuohua-build-{}", ulid::Ulid::generate()));
        let error = expect_err(build_in_root(&root, "BadName", &toml::value::Table::new()));
        assert!(error.contains("lowercase letter first"), "{error}");
    }

    #[tokio::test]
    async fn bounded_session_io_times_out_pending_write() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let error = bounded_session_io(
            &cancel,
            "test write",
            std::future::pending::<Result<(), std::io::Error>>(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AsrError::TransportTimeout));
    }

    #[tokio::test]
    async fn bounded_session_io_cancel_wins() {
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        let error = bounded_session_io(
            &cancel,
            "test write",
            std::future::pending::<Result<(), std::io::Error>>(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AsrError::Canceled));
    }

    #[tokio::test]
    async fn full_session_command_queue_times_out() {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        tx.send(1).await.unwrap();
        let error = send_session_command(&tx, 2, "ended").await.unwrap_err();
        assert!(matches!(error, AsrError::TransportTimeout));
    }
}
