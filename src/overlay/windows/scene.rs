use super::icons::{state_icon_plan, StateIconPlan};
use crate::overlay::layout::{self as L, OverlayFrames};
use crate::overlay::OverlayModel;

#[derive(Debug, Clone)]
pub(super) struct TextPlan {
    pub(super) text: String,
    pub(super) lines: usize,
}

#[derive(Debug, Clone)]
pub(super) struct WindowsOverlayScene {
    pub(super) text: TextPlan,
    pub(super) frames: OverlayFrames,
    pub(super) state_icon: StateIconPlan,
    pub(super) state_label: String,
    pub(super) state_color: u32,
    pub(super) stats: String,
    pub(super) meta: String,
    pub(super) meta_color: u32,
    pub(super) body_color: u32,
}

impl WindowsOverlayScene {
    pub(super) fn from_model(
        model: &OverlayModel,
        cfg: &crate::config::theme::EffectiveOverlayCfg,
        panel_width: f64,
        text: TextPlan,
    ) -> Self {
        let frames = L::overlay_frames(panel_width, cfg.core.text_scale, text.lines);
        let app = model.app_name.as_deref().unwrap_or_default();
        let stats = L::stats_text(
            &L::format_duration(model.dur_ms),
            &crate::t!("overlay.word_count", n = model.words),
            app,
        );
        let (meta, meta_color) = if let Some(notice) = &model.notice {
            (notice.text.clone(), cfg.core.text.notice)
        } else {
            (model.chain_summary.clone(), cfg.core.text.tertiary)
        };
        let body_color = if model.error_text.is_empty() {
            cfg.core.text.primary
        } else {
            cfg.core.text.error
        };

        Self {
            text,
            frames,
            state_icon: state_icon_plan(model.state),
            state_label: model.state_label.clone(),
            state_color: model.state_color,
            stats,
            meta,
            meta_color,
            body_color,
        }
    }
}
