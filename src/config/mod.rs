//! 配置加载。schema/profile 路由/theme/热重载见 docs/modules/config.md。

mod main;

pub(crate) use self::main::{
    default_path, load_from, AppIdentity, Config, LinuxProfileMatchers, MacosProfileMatchers,
    OverlayPosition, ProfileRouteCfg, ProfileRouteMatchers, ProfileRoutes, RecordAudioMode,
    VoiceVadBackend, VoiceVadCfg, WindowsProfileMatchers,
};

pub(crate) mod asr;
pub(crate) mod diagnostics;
pub(crate) mod inventory;
pub(crate) mod paths;
pub(crate) mod post;
pub(crate) mod profile;
pub(crate) mod schema;
pub(crate) mod spec;
pub(crate) mod template;
pub(crate) mod theme;
