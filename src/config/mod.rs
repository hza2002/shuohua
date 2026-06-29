//! 配置加载。schema/profile 路由/theme/热重载见 docs/modules/config.md。

mod main;

pub(crate) use self::main::{
    default_path, load_from, Config, OverlayPosition, ProfileRouteCfg, RecordAudioMode,
    VoicePreprocessBackend, VoicePreprocessCfg, VoiceVadBackend, VoiceVadCfg,
};

pub(crate) mod asr;
pub(crate) mod diagnostics;
pub(crate) mod inventory;
pub(crate) mod paths;
pub(crate) mod post;
pub(crate) mod profile;
pub(crate) mod profile_write;
pub(crate) mod schema;
pub(crate) mod spec;
pub(crate) mod template;
pub(crate) mod theme;
