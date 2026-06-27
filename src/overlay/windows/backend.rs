use anyhow::Result;
use windows_sys::Win32::Foundation::HWND;

use super::direct2d::Direct2dRenderer;
use super::scene::{TextPlan, WindowsOverlayScene};
use super::{composition, WindowMetrics, DIRECT2D_SHADOW_OUTSET};
use crate::overlay::OverlayModel;

const COMPOSITION_PROBE_ENV: &str = "SHUOHUA_WINDOWS_OVERLAY_COMPOSITION_PROBE";
const COMPOSITION_VISIBLE_ENV: &str = "SHUOHUA_WINDOWS_OVERLAY_COMPOSITION_VISIBLE";

pub(super) enum RendererKind {
    CompositionPlanned,
    CompositionVisible,
    Direct2dPerPixel,
    GdiFallback,
}

pub(super) struct WindowsRendererBackend {
    composition: Option<composition::CompositionRenderer>,
    direct2d: Option<Direct2dRenderer>,
    composition_ready: composition::CompositionReadiness,
    composition_visible: bool,
}

impl WindowsRendererBackend {
    pub(super) fn pending() -> Self {
        Self {
            composition: None,
            direct2d: None,
            composition_ready: composition::readiness(),
            composition_visible: false,
        }
    }

    pub(super) fn new(hwnd: HWND) -> Self {
        let (composition, composition_ready) = probe_composition(hwnd);
        let composition_visible =
            composition.is_some() && std::env::var_os(COMPOSITION_VISIBLE_ENV).is_some();
        match Direct2dRenderer::new(hwnd) {
            Ok(direct2d) => Self {
                composition,
                direct2d: Some(direct2d),
                composition_ready,
                composition_visible,
            },
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "Direct2D overlay renderer unavailable; falling back to GDI"
                );
                Self {
                    composition,
                    direct2d: None,
                    composition_ready,
                    composition_visible,
                }
            }
        }
    }

    pub(super) fn kind(&self) -> RendererKind {
        if self.composition_visible {
            RendererKind::CompositionVisible
        } else if self.direct2d.is_some() {
            RendererKind::Direct2dPerPixel
        } else {
            RendererKind::GdiFallback
        }
    }

    pub(super) fn composition_readiness(&self) -> composition::CompositionReadiness {
        self.composition_ready
    }

    pub(super) fn uses_per_pixel_surface(&self) -> bool {
        matches!(
            self.kind(),
            RendererKind::CompositionVisible | RendererKind::Direct2dPerPixel
        )
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
        let composition_scene = if self.composition.is_some() {
            self.direct2d
                .as_ref()
                .and_then(|renderer| renderer.text_plan(model, cfg, metrics, panel_width).ok())
                .map(|text| WindowsOverlayScene::from_model(model, cfg, panel_width, text))
        } else {
            None
        };
        let mut composition_painted = false;
        if let (Some(composition), Some(scene)) =
            (self.composition.as_mut(), composition_scene.as_ref())
        {
            if let Err(error) = composition.update_reserved_scene(scene, metrics) {
                tracing::warn!(
                    ?error,
                    "DirectComposition reserved scene update failed; keeping fallback renderer"
                );
            } else {
                composition_painted = true;
            }
        }
        if self.composition_visible && composition_painted {
            return Some(Ok(()));
        }
        self.direct2d
            .as_mut()
            .map(|renderer| renderer.paint(model, cfg, metrics, panel_width))
    }

    pub(super) fn disable_accelerated_backend(&mut self) {
        self.direct2d = None;
    }
}

fn probe_composition(
    hwnd: HWND,
) -> (
    Option<composition::CompositionRenderer>,
    composition::CompositionReadiness,
) {
    if std::env::var_os(COMPOSITION_PROBE_ENV).is_none() {
        return (None, composition::CompositionReadiness::Planned);
    }
    match composition::CompositionRenderer::new(hwnd) {
        Ok(renderer) => {
            tracing::info!(
                "DirectComposition overlay probe initialized; renderer remains fallback"
            );
            (
                Some(renderer),
                composition::CompositionReadiness::ProbeReady,
            )
        }
        Err(error) => {
            tracing::warn!(
                ?error,
                "DirectComposition overlay probe unavailable; using fallback renderer"
            );
            (None, composition::CompositionReadiness::Disabled)
        }
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
            RendererKind::CompositionVisible,
            RendererKind::CompositionVisible
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
        assert!(!backend.composition_visible);
    }
}
