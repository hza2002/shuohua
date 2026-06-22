//! Overlay 浮层。材质、视图层级不变量与平台边界见 docs/modules/overlay.md。

mod command;
pub mod layout;
mod model;
mod renderer;

pub use command::{OverlayCmd, OverlayHandle, OverlayReceiver, OverlayState, TextKind};
pub use model::OverlayModel;

#[cfg(target_os = "macos")]
mod macos;

pub fn run(
    rx: OverlayReceiver,
    cfg: crate::config::theme::EffectiveOverlayCfg,
) -> anyhow::Result<()> {
    renderer::run(rx, cfg)
}
