//! ASR provider 实现。每个 provider 一个子模块。

use std::sync::Arc;

use anyhow::{Context, Result};

use crate::asr::AsrProvider;

pub mod apple;
pub mod doubao;

pub struct ProviderRuntime {
    pub provider: Arc<dyn AsrProvider>,
    pub options: ProviderOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderOptions {
    pub idle_pause: bool,
    pub finalize_timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeCheckNotice {
    pub line: &'static str,
}

pub fn build(name: &str, overrides: &toml::value::Table) -> Result<ProviderRuntime> {
    match name {
        "apple" => {
            let provider = apple::AppleProvider::new_with_overrides(Some(overrides))
                .context("init apple provider")?;
            Ok(ProviderRuntime {
                options: provider.options(),
                provider: Arc::new(provider),
            })
        }
        "doubao" => {
            let provider = doubao::DoubaoProvider::new_with_overrides(Some(overrides))
                .context("init doubao provider")?;
            Ok(ProviderRuntime {
                options: provider.options(),
                provider: Arc::new(provider),
            })
        }
        other => anyhow::bail!("未知 ASR provider {other:?}。支持 \"doubao\" / \"apple\""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_rejects_unknown_provider() {
        match build("missing", &toml::value::Table::new()) {
            Ok(_) => panic!("unknown provider must be rejected"),
            Err(error) => assert!(error.to_string().contains("missing")),
        }
    }
}
