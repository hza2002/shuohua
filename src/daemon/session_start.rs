use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

use crate::overlay::{OverlayCmd, OverlayHandle, TextKind};
use crate::state::StateStore;
use crate::voice::finish::SessionParams;
use crate::voice::SessionControl;

pub(super) struct SessionStart {
    pub(super) provider: Arc<dyn crate::asr::AsrProvider>,
    pub(super) params: SessionParams,
    pub(super) control: SessionControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionStartError {
    Profile,
    PostChainLoad,
    PostChainBuild,
    AsrProvider,
}

impl SessionStartError {
    fn i18n_key(self) -> &'static str {
        match self {
            Self::Profile => "error.profile_load",
            Self::PostChainLoad => "error.post_chain_load",
            Self::PostChainBuild => "error.post_chain_build",
            Self::AsrProvider => "error.asr_provider_init",
        }
    }
}

pub(super) fn prepare(
    runtime_cfg: &crate::reload::Cfg,
    start_app_context: crate::post::AppContext,
    state_store: StateStore,
    overlay: OverlayHandle,
    build_provider: impl Fn(&str, &toml::value::Table) -> Result<crate::asr::providers::ProviderRuntime>,
) -> std::result::Result<SessionStart, SessionStartError> {
    let profile_dir = crate::config::profile::default_dir();
    let post_dir = crate::config::post::default_dir();
    prepare_with_dirs(
        runtime_cfg,
        start_app_context,
        state_store,
        overlay,
        &profile_dir,
        &post_dir,
        build_provider,
    )
}

fn prepare_with_dirs(
    runtime_cfg: &crate::reload::Cfg,
    start_app_context: crate::post::AppContext,
    state_store: StateStore,
    overlay: OverlayHandle,
    profile_dir: &Path,
    post_dir: &Path,
    build_provider: impl Fn(&str, &toml::value::Table) -> Result<crate::asr::providers::ProviderRuntime>,
) -> std::result::Result<SessionStart, SessionStartError> {
    let cfg = &runtime_cfg.config;
    let profile = crate::config::profile::load_for_app(
        profile_dir,
        &cfg.profile,
        start_app_context.bundle_id.as_deref(),
    )
    .map_err(|error| {
        tracing::warn!(error = ?error, "profile load failed");
        SessionStartError::Profile
    })?;

    let post_chain_config = crate::config::post::load_components(
        &profile.post.chain,
        &crate::config::post::PostDirs {
            rule: post_dir.join("rule"),
            llm: post_dir.join("llm"),
        },
        &profile.post.llm,
    )
    .map_err(|error| {
        tracing::warn!(error = ?error, "post chain load failed");
        SessionStartError::PostChainLoad
    })?;

    let post_chain = crate::post::build_chain(post_chain_config).map_err(|error| {
        tracing::warn!(error = ?error, "post chain build failed");
        SessionStartError::PostChainBuild
    })?;

    let runtime =
        build_provider(&profile.asr.provider, &profile.asr.overrides).map_err(|error| {
            tracing::error!(error = ?error, "ASR provider init failed");
            SessionStartError::AsrProvider
        })?;

    Ok(SessionStart {
        provider: runtime.provider,
        params: SessionParams {
            auto_paste: cfg.voice.auto_paste,
            record_audio: cfg.voice.record_audio,
            vad_trace: cfg.dev.vad_trace,
            idle_pause: runtime.options.idle_pause,
            finalize_timeout_ms: runtime.options.finalize_timeout_ms,
            vad: cfg.voice.vad.clone(),
            stop_delay_ms: cfg.voice.stop_delay_ms,
            hotwords: profile.asr.hotwords,
            start_app_context,
            post_chain,
            post_timeout_ms: cfg.post.timeout_ms,
            overlay: Some(overlay),
            state: state_store,
        },
        control: SessionControl::new(),
    })
}

pub(super) fn send_error_overlay(overlay: &OverlayHandle, error: SessionStartError) {
    overlay.send(OverlayCmd::SetText {
        text: crate::i18n::tr(error.i18n_key(), &[]),
        kind: TextKind::Error,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex as StdMutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| StdMutex::new(())).lock().unwrap()
    }

    fn temp_config_home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shuohua-daemon-test-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_minimal_config(root: &Path, profile_body: &str) {
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::write(
            root.join("config.toml"),
            r#"
[hotkey]
trigger = "f16"

[profile]
default = "default"
"#,
        )
        .unwrap();
        fs::write(root.join("profile/default.toml"), profile_body).unwrap();
    }

    fn fake_runtime(
        provider: Arc<dyn crate::asr::AsrProvider>,
    ) -> crate::asr::providers::ProviderRuntime {
        crate::asr::providers::ProviderRuntime {
            provider,
            options: crate::asr::providers::ProviderOptions {
                idle_pause: true,
                finalize_timeout_ms: 1234,
            },
        }
    }

    #[test]
    fn session_start_error_maps_to_i18n_keys() {
        assert_eq!(SessionStartError::Profile.i18n_key(), "error.profile_load");
        assert_eq!(
            SessionStartError::PostChainLoad.i18n_key(),
            "error.post_chain_load"
        );
        assert_eq!(
            SessionStartError::PostChainBuild.i18n_key(),
            "error.post_chain_build"
        );
        assert_eq!(
            SessionStartError::AsrProvider.i18n_key(),
            "error.asr_provider_init"
        );
    }

    #[test]
    fn start_error_overlay_sends_localized_error_text() {
        crate::i18n::init("en-US");
        let (overlay, mut rx) = OverlayHandle::channel();

        send_error_overlay(&overlay, SessionStartError::Profile);

        match rx.try_recv().unwrap() {
            OverlayCmd::SetText { text, kind } => {
                assert_eq!(kind, TextKind::Error);
                assert_eq!(text, "Profile could not be loaded");
            }
            other => panic!("unexpected overlay command: {other:?}"),
        }
    }

    #[test]
    fn prepare_session_start_builds_params_from_profile_and_runtime_options() {
        let _guard = env_lock();
        let config_home = temp_config_home();
        let root = config_home.join("shuohua");
        write_minimal_config(
            &root,
            r#"
name = "default"

[asr]
provider = "fake"
hotwords = ["Rust", "macOS"]

[post]
chain = []
"#,
        );
        let cfg = Arc::new(crate::reload::RuntimeConfig {
            config: crate::config::load_from(&root.join("config.toml")).unwrap(),
            theme: crate::config::theme::EffectiveTheme::default(),
            theme_warning: None,
        });
        let (overlay, _rx) = OverlayHandle::channel();

        let start = prepare_with_dirs(
            &cfg,
            crate::post::AppContext {
                bundle_id: Some("com.example.App".to_string()),
                app_name: Some("Example".to_string()),
            },
            StateStore::new(),
            overlay,
            &root.join("profile"),
            &root.join("post"),
            |name, overrides| {
                assert_eq!(name, "fake");
                assert!(overrides.is_empty());
                Ok(fake_runtime(
                    Arc::new(crate::asr::fake::FakeProvider::new()),
                ))
            },
        )
        .unwrap();

        assert_eq!(start.params.hotwords, ["Rust", "macOS"]);
        assert!(start.params.idle_pause);
        assert_eq!(start.params.finalize_timeout_ms, 1234);
        assert_eq!(start.params.post_timeout_ms, 30_000);
        assert_eq!(
            start.params.start_app_context.app_name.as_deref(),
            Some("Example")
        );

        let _ = fs::remove_dir_all(config_home);
    }

    #[test]
    fn prepare_session_start_classifies_provider_build_failure() {
        let _guard = env_lock();
        let config_home = temp_config_home();
        let root = config_home.join("shuohua");
        write_minimal_config(
            &root,
            r#"
name = "default"

[asr]
provider = "fake"

[post]
chain = []
"#,
        );
        let cfg = Arc::new(crate::reload::RuntimeConfig {
            config: crate::config::load_from(&root.join("config.toml")).unwrap(),
            theme: crate::config::theme::EffectiveTheme::default(),
            theme_warning: None,
        });
        let (overlay, _rx) = OverlayHandle::channel();

        let result = prepare_with_dirs(
            &cfg,
            crate::post::AppContext::default(),
            StateStore::new(),
            overlay,
            &root.join("profile"),
            &root.join("post"),
            |_name, _overrides| anyhow::bail!("provider unavailable"),
        );
        let Err(error) = result else {
            panic!("provider build failure should reject session start");
        };

        assert_eq!(error, SessionStartError::AsrProvider);

        let _ = fs::remove_dir_all(config_home);
    }
}
