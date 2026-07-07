//! Overlay 浮层。材质、视图层级不变量与平台边界见 docs/modules/overlay.md。

mod command;
pub mod layout;
mod model;

pub use command::{
    OverlayAction, OverlayActionHandle, OverlayActionReceiver, OverlayCmd, OverlayHandle,
    OverlayReceiver, OverlayState, ProfileChoice, TextKind,
};
pub use model::OverlayModel;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::run;

#[cfg(not(target_os = "macos"))]
pub fn run(
    _rx: OverlayReceiver,
    _actions: OverlayActionHandle,
    _cfg: crate::config::theme::EffectiveOverlayCfg,
    _on_screen: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<()> {
    anyhow::bail!("overlay is not implemented on this platform")
}
