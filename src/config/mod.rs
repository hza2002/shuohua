//! 配置加载。schema/profile 路由/theme/热重载见 docs/modules/config.md。

mod main;

pub(crate) use self::main::{
    default_path, load_from, Config, OverlayPosition, ProfileRouteCfg, RecordAudioMode,
    VoicePreprocessBackend, VoicePreprocessCfg, VoiceVadBackend, VoiceVadCfg,
};

pub use self::main::DEFAULT_OVERLAY_WIDTH_PX;

#[cfg(test)]
/// Serializes process-wide config-home overrides across all test modules.
pub(crate) struct TestConfigHome {
    _lock: std::sync::MutexGuard<'static, ()>,
    old: Option<std::ffi::OsString>,
}

#[cfg(test)]
impl TestConfigHome {
    pub(crate) fn set(path: &std::path::Path) -> Self {
        use std::sync::{Mutex, OnceLock};

        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let lock = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let old = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", path);
        Self { _lock: lock, old }
    }
}

#[cfg(test)]
impl Drop for TestConfigHome {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}

pub(crate) mod asr;
pub(crate) mod diagnostics;
pub(crate) mod field_view;
pub(crate) mod field_write;
pub(crate) mod inventory;
pub(crate) mod paths;
pub(crate) mod post;
pub(crate) mod profile;
pub(crate) mod profile_compose_write;
pub(crate) mod profile_write;
pub(crate) mod schema;
pub(crate) mod spec;
pub(crate) mod template;
pub(crate) mod theme;
