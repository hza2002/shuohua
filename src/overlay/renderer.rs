use anyhow::Result;

use super::OverlayReceiver;

#[cfg(target_os = "macos")]
pub(super) fn run(
    rx: OverlayReceiver,
    cfg: crate::config::theme::EffectiveOverlayCfg,
) -> Result<()> {
    super::macos::run(rx, cfg);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn run(
    _rx: OverlayReceiver,
    _cfg: crate::config::theme::EffectiveOverlayCfg,
) -> Result<()> {
    anyhow::bail!("overlay is not implemented on this platform")
}
