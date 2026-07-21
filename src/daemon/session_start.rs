use anyhow::{Context, Result};
use std::sync::Arc;

use crate::overlay::{OverlayCmd, OverlayHandle, ProfileChoice, TextKind};
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
    start: crate::voice::resume::RecordingStart,
    build_provider: impl Fn(&str, &toml::value::Table) -> Result<crate::asr::providers::ProviderRuntime>,
) -> std::result::Result<SessionStart, SessionStartError> {
    let cfg = &runtime_cfg.config;
    let profile = crate::config::profile::load_for_app(
        &crate::config::profile::default_dir(),
        &cfg.profile,
        start_app_context.bundle_id.as_deref(),
    )
    .map_err(|error| {
        tracing::warn!(error = ?error, "profile load failed");
        SessionStartError::Profile
    })?;

    let post_chain_config = crate::config::post::load_components(
        &profile.post.chain,
        &crate::config::post::PostDir {
            dir: crate::config::post::default_dir(),
        },
        &profile.post.overrides,
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
        build_provider(&profile.asr.instance, &profile.asr.overrides).map_err(|error| {
            tracing::error!(error = ?error, "ASR provider init failed");
            SessionStartError::AsrProvider
        })?;

    let profile_name = profile.display_name();
    let mut vad = cfg.voice.vad.clone();
    let idle_pause = match runtime.options.local_vad {
        crate::config::asr::LocalVadMode::Auto => {
            matches!(vad.backend, crate::config::VoiceVadBackend::Silero)
        }
        crate::config::asr::LocalVadMode::On => {
            vad.backend = crate::config::VoiceVadBackend::Silero;
            true
        }
        crate::config::asr::LocalVadMode::Off => false,
    };
    Ok(SessionStart {
        provider: runtime.provider,
        params: SessionParams {
            auto_paste: cfg.voice.auto_paste,
            record_audio: cfg.voice.record_audio,
            preprocess: cfg.voice.preprocess.clone(),
            vad_trace: cfg.dev.vad_trace,
            apple_backend_trace: cfg.dev.apple_backend_trace,
            idle_pause,
            open_timeout_ms: runtime.options.open_timeout_ms,
            finalize_timeout_ms: runtime.options.finalize_timeout_ms,
            vad,
            stop_delay_ms: cfg.voice.stop_delay_ms,
            hotwords: profile.asr.hotwords,
            start_app_context,
            profile_name,
            profile_choices: profile_choices(&cfg.profile),
            post_chain,
            post_timeout_ms: cfg.post.timeout_ms,
            start,
            overlay: Some(overlay),
            state: state_store,
        },
        control: SessionControl::new(),
    })
}

fn profile_choices(routes: &crate::config::ProfileRouteCfg) -> Vec<ProfileChoice> {
    let profile_dir = crate::config::profile::default_dir();
    let mut names = vec![routes.default.clone()];
    names.extend(
        routes
            .routes
            .keys()
            .filter(|name| *name != &routes.default)
            .cloned(),
    );
    match std::fs::read_dir(&profile_dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
        Err(error) => {
            tracing::warn!(error = ?error, dir = %profile_dir.display(), "profile choice scan failed");
        }
    }
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            let path = profile_dir.join(format!("{name}.toml"));
            match std::fs::read_to_string(&path)
                .with_context(|| format!("read profile {}", path.display()))
                .and_then(|body| {
                    crate::config::profile::parse(&body)
                        .with_context(|| format!("parse profile {}", path.display()))
                }) {
                Ok(mut profile) => {
                    profile.id = name.clone();
                    ProfileChoice {
                        display_name: profile.display_name(),
                        id: name,
                        asr_instance: profile.asr.instance,
                        chain_summary: profile.post.chain.join(" → "),
                    }
                }
                Err(error) => {
                    tracing::warn!(error = ?error, profile = name, "profile choice load failed");
                    ProfileChoice {
                        id: name.clone(),
                        display_name: name,
                        asr_instance: String::new(),
                        chain_summary: String::new(),
                    }
                }
            }
        })
        .collect()
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

    fn temp_config_home() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("shuohua-daemon-test-{}", ulid::Ulid::generate()));
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
        fake_runtime_with_local_vad(provider, crate::config::asr::LocalVadMode::On)
    }

    fn fake_runtime_with_local_vad(
        provider: Arc<dyn crate::asr::AsrProvider>,
        local_vad: crate::config::asr::LocalVadMode,
    ) -> crate::asr::providers::ProviderRuntime {
        crate::asr::providers::ProviderRuntime {
            provider,
            options: crate::asr::providers::ProviderOptions {
                local_vad,
                open_timeout_ms: 4321,
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
        let config_home = temp_config_home();
        let _env = crate::config::TestConfigHome::set(&config_home);
        let root = config_home.join("shuohua");
        write_minimal_config(
            &root,
            r#"
name = "default"

[asr]
instance = "fake"
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

        let start = prepare(
            &cfg,
            crate::post::AppContext {
                bundle_id: Some("com.example.App".to_string()),
                app_name: Some("Example".to_string()),
            },
            StateStore::new(),
            overlay,
            crate::voice::resume::RecordingStart::Fresh,
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
        assert_eq!(
            start.params.profile_choices,
            [ProfileChoice {
                id: "default".to_string(),
                display_name: "default".to_string(),
                asr_instance: "fake".to_string(),
                chain_summary: String::new(),
            }]
        );
        assert!(start.params.idle_pause);
        assert_eq!(start.params.open_timeout_ms, 4321);
        assert_eq!(start.params.finalize_timeout_ms, 1234);
        assert_eq!(start.params.post_timeout_ms, 30_000);
        assert_eq!(
            start.params.start_app_context.app_name.as_deref(),
            Some("Example")
        );

        let _ = fs::remove_dir_all(config_home);
    }

    #[test]
    fn local_vad_on_overrides_global_vad_backend_off() {
        let config_home = temp_config_home();
        let _env = crate::config::TestConfigHome::set(&config_home);
        let root = config_home.join("shuohua");
        fs::create_dir_all(root.join("profile")).unwrap();
        fs::write(
            root.join("config.toml"),
            r#"
[hotkey]
trigger = "f16"

[voice.vad]
backend = "off"

[profile]
default = "default"
"#,
        )
        .unwrap();
        fs::write(
            root.join("profile/default.toml"),
            r#"
name = "default"

[asr]
instance = "fake"

[post]
chain = []
"#,
        )
        .unwrap();
        let cfg = Arc::new(crate::reload::RuntimeConfig {
            config: crate::config::load_from(&root.join("config.toml")).unwrap(),
            theme: crate::config::theme::EffectiveTheme::default(),
            theme_warning: None,
        });
        let (overlay, _rx) = OverlayHandle::channel();

        let start = prepare(
            &cfg,
            crate::post::AppContext::default(),
            StateStore::new(),
            overlay,
            crate::voice::resume::RecordingStart::Fresh,
            |_name, _overrides| {
                Ok(fake_runtime_with_local_vad(
                    Arc::new(crate::asr::fake::FakeProvider::new()),
                    crate::config::asr::LocalVadMode::On,
                ))
            },
        )
        .unwrap();

        assert!(start.params.idle_pause);
        assert!(matches!(
            start.params.vad.backend,
            crate::config::VoiceVadBackend::Silero
        ));

        let _ = fs::remove_dir_all(config_home);
    }

    #[test]
    fn profile_choices_include_unrouted_profile_files() {
        let config_home = temp_config_home();
        let _env = crate::config::TestConfigHome::set(&config_home);
        let root = config_home.join("shuohua");
        write_minimal_config(
            &root,
            r#"
name = "Default"

[asr]
instance = "fake"

[post]
chain = []
"#,
        );
        fs::write(
            root.join("profile/coding.toml"),
            r#"
name = "Coding"

[asr]
instance = "apple"

[post]
chain = ["zh_filter"]
"#,
        )
        .unwrap();
        let cfg = Arc::new(crate::reload::RuntimeConfig {
            config: crate::config::load_from(&root.join("config.toml")).unwrap(),
            theme: crate::config::theme::EffectiveTheme::default(),
            theme_warning: None,
        });
        let (overlay, _rx) = OverlayHandle::channel();

        let start = prepare(
            &cfg,
            crate::post::AppContext::default(),
            StateStore::new(),
            overlay,
            crate::voice::resume::RecordingStart::Fresh,
            |_name, _overrides| {
                Ok(fake_runtime(
                    Arc::new(crate::asr::fake::FakeProvider::new()),
                ))
            },
        )
        .unwrap();

        assert!(start
            .params
            .profile_choices
            .iter()
            .any(|choice| choice.id == "coding"
                && choice.display_name == "Coding"
                && choice.asr_instance == "apple"
                && choice.chain_summary == "zh_filter"));

        let _ = fs::remove_dir_all(config_home);
    }

    #[test]
    fn prepare_session_start_classifies_provider_build_failure() {
        let config_home = temp_config_home();
        let _env = crate::config::TestConfigHome::set(&config_home);
        let root = config_home.join("shuohua");
        write_minimal_config(
            &root,
            r#"
name = "default"

[asr]
instance = "fake"

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

        let result = prepare(
            &cfg,
            crate::post::AppContext::default(),
            StateStore::new(),
            overlay,
            crate::voice::resume::RecordingStart::Fresh,
            |_name, _overrides| anyhow::bail!("provider unavailable"),
        );
        let Err(error) = result else {
            panic!("provider build failure should reject session start");
        };

        assert_eq!(error, SessionStartError::AsrProvider);

        let _ = fs::remove_dir_all(config_home);
    }
}
