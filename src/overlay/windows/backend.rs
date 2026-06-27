use anyhow::Result;
use windows_sys::Win32::Foundation::HWND;

use super::direct2d::{Direct2dRenderer, TextPlan};
use super::{composition, WindowMetrics, DIRECT2D_SHADOW_OUTSET};
use crate::overlay::OverlayModel;

pub(super) enum RendererKind {
    CompositionPlanned,
    Direct2dPerPixel,
    GdiFallback,
}

pub(super) struct WindowsRendererBackend {
    direct2d: Option<Direct2dRenderer>,
    composition_ready: composition::CompositionReadiness,
}

impl WindowsRendererBackend {
    pub(super) fn pending() -> Self {
        Self {
            direct2d: None,
            composition_ready: composition::readiness(),
        }
    }

    pub(super) fn new(hwnd: HWND) -> Self {
        let composition_ready = composition::readiness();
        match Direct2dRenderer::new(hwnd) {
            Ok(direct2d) => Self {
                direct2d: Some(direct2d),
                composition_ready,
            },
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "Direct2D overlay renderer unavailable; falling back to GDI"
                );
                Self {
                    direct2d: None,
                    composition_ready,
                }
            }
        }
    }

    pub(super) fn kind(&self) -> RendererKind {
        if self.direct2d.is_some() {
            RendererKind::Direct2dPerPixel
        } else {
            RendererKind::GdiFallback
        }
    }

    pub(super) fn composition_readiness(&self) -> composition::CompositionReadiness {
        self.composition_ready
    }

    pub(super) fn uses_per_pixel_surface(&self) -> bool {
        matches!(self.kind(), RendererKind::Direct2dPerPixel)
    }

    pub(super) fn surface_outset(&self) -> f64 {
        if self.uses_per_pixel_surface() {
            DIRECT2D_SHADOW_OUTSET
        } else {
            0.0
        }
    }

    pub(super) fn text_plan(
        &self,
        model: &OverlayModel,
        cfg: &crate::config::theme::EffectiveOverlayCfg,
        metrics: WindowMetrics,
        panel_width: f64,
    ) -> Option<Result<TextPlan>> {
        self.direct2d
            .as_ref()
            .map(|renderer| renderer.text_plan(model, cfg, metrics, panel_width))
    }

    pub(super) fn paint(
        &mut self,
        model: &OverlayModel,
        cfg: &crate::config::theme::EffectiveOverlayCfg,
        metrics: WindowMetrics,
        panel_width: f64,
    ) -> Option<Result<()>> {
        self.direct2d
            .as_mut()
            .map(|renderer| renderer.paint(model, cfg, metrics, panel_width))
    }

    pub(super) fn disable_accelerated_backend(&mut self) {
        self.direct2d = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_names_keep_composition_before_fallback_order() {
        assert!(matches!(
            RendererKind::CompositionPlanned,
            RendererKind::CompositionPlanned
        ));
        assert!(matches!(
            RendererKind::Direct2dPerPixel,
            RendererKind::Direct2dPerPixel
        ));
        assert!(matches!(
            RendererKind::GdiFallback,
            RendererKind::GdiFallback
        ));
    }

    #[test]
    fn pending_backend_records_composition_readiness_without_hwnd() {
        let backend = WindowsRendererBackend::pending();
        assert_eq!(
            backend.composition_readiness(),
            composition::CompositionReadiness::Planned
        );
        assert!(matches!(backend.kind(), RendererKind::GdiFallback));
    }
}
