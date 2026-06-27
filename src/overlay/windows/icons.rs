use crate::overlay::OverlayState;

pub(super) const FLUENT_ICON_FONT: &str = "Segoe Fluent Icons";
pub(super) const MDL2_ICON_FONT: &str = "Segoe MDL2 Assets";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IconAnimation {
    Breathe,
    Pulse,
    Rotate,
    Dots,
    Shake,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StateIconPlan {
    pub(super) state: OverlayState,
    pub(super) fluent_glyph: char,
    pub(super) mdl2_glyph: char,
    pub(super) animation: IconAnimation,
}

pub(super) fn state_icon_plan(state: OverlayState) -> StateIconPlan {
    match state {
        OverlayState::Idle => StateIconPlan {
            state,
            fluent_glyph: '\u{E720}',
            mdl2_glyph: '\u{E720}',
            animation: IconAnimation::Breathe,
        },
        OverlayState::Connecting => StateIconPlan {
            state,
            fluent_glyph: '\u{E895}',
            mdl2_glyph: '\u{E895}',
            animation: IconAnimation::Rotate,
        },
        OverlayState::Recording => StateIconPlan {
            state,
            fluent_glyph: '\u{E1D6}',
            mdl2_glyph: '\u{E1D6}',
            animation: IconAnimation::Pulse,
        },
        OverlayState::Thinking => StateIconPlan {
            state,
            fluent_glyph: '\u{E712}',
            mdl2_glyph: '\u{E712}',
            animation: IconAnimation::Dots,
        },
        OverlayState::Stopping => StateIconPlan {
            state,
            fluent_glyph: '\u{E15B}',
            mdl2_glyph: '\u{E15B}',
            animation: IconAnimation::Pulse,
        },
        OverlayState::Error => StateIconPlan {
            state,
            fluent_glyph: '\u{E7BA}',
            mdl2_glyph: '\u{E7BA}',
            animation: IconAnimation::Shake,
        },
    }
}

pub(super) fn icon_font_fallback_order() -> [&'static str; 2] {
    [FLUENT_ICON_FONT, MDL2_ICON_FONT]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_icon_plan_uses_official_system_icon_fonts() {
        assert_eq!(
            icon_font_fallback_order(),
            ["Segoe Fluent Icons", "Segoe MDL2 Assets"]
        );
    }

    #[test]
    fn every_overlay_state_has_an_icon_and_animation_plan() {
        for state in [
            OverlayState::Idle,
            OverlayState::Connecting,
            OverlayState::Recording,
            OverlayState::Thinking,
            OverlayState::Stopping,
            OverlayState::Error,
        ] {
            let plan = state_icon_plan(state);
            assert_eq!(plan.state, state);
            assert_ne!(plan.fluent_glyph, '\0');
            assert_ne!(plan.mdl2_glyph, '\0');
        }
    }

    #[test]
    fn recording_uses_system_glyph_not_hand_drawn_bars_in_the_plan() {
        let plan = state_icon_plan(OverlayState::Recording);
        assert_eq!(plan.animation, IconAnimation::Pulse);
    }
}
