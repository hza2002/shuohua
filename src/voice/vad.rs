#![allow(dead_code)]

//! Small, backend-agnostic VAD state controller.
//!
//! This module intentionally does not know about Silero, Apple, Doubao, or PCM
//! buffering. Backends emit binary speech/silence frames; this controller adds
//! hysteresis so the voice layer can decide when a provider session should
//! resume or pause.

/// Raw frame-level decision emitted by one VAD backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadFrame {
    Speech,
    Silence,
}

/// Stable voice state after hysteresis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadState {
    Silence,
    Speech,
}

/// Edge transitions consumed by the voice session controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadTransition {
    None,
    SpeechStarted,
    SilenceStarted,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VadPolicy {
    pub min_start_voiced_frames: u32,
    pub pause_silence_ms: u32,
    pub frame_ms: u32,
    pub speech_threshold: f32,
    pub silence_threshold: f32,
}

impl Default for VadPolicy {
    fn default() -> Self {
        let speech_threshold = 0.5;
        Self {
            min_start_voiced_frames: 2,
            pause_silence_ms: 1500,
            frame_ms: 32,
            speech_threshold,
            silence_threshold: default_silence_threshold(speech_threshold),
        }
    }
}

impl VadPolicy {
    fn normalized(self) -> Self {
        let speech_threshold = self.speech_threshold.clamp(0.01, 0.99);
        let silence_threshold = self.silence_threshold.clamp(0.0, speech_threshold);
        Self {
            min_start_voiced_frames: self.min_start_voiced_frames.max(1),
            pause_silence_ms: self.pause_silence_ms.max(1),
            frame_ms: self.frame_ms.max(1),
            speech_threshold,
            silence_threshold,
        }
    }
}

pub fn policy_from_config(config: &crate::config::VoiceVadCfg, frame_ms: u32) -> VadPolicy {
    VadPolicy {
        min_start_voiced_frames: config.min_start_voiced_frames,
        pause_silence_ms: config.pause_silence_ms,
        frame_ms,
        speech_threshold: config.threshold,
        silence_threshold: default_silence_threshold(config.threshold),
    }
}

fn default_silence_threshold(speech_threshold: f32) -> f32 {
    (speech_threshold - 0.15).max(speech_threshold * 0.5)
}

#[derive(Debug, Clone)]
pub struct VadController {
    policy: VadPolicy,
    state: VadState,
    voiced_frames: u32,
    silence_ms: u32,
}

impl VadController {
    pub fn new(policy: VadPolicy) -> Self {
        Self {
            policy: policy.normalized(),
            state: VadState::Silence,
            voiced_frames: 0,
            silence_ms: 0,
        }
    }

    pub fn state(&self) -> VadState {
        self.state
    }

    pub fn reset(&mut self) {
        self.state = VadState::Silence;
        self.voiced_frames = 0;
        self.silence_ms = 0;
    }

    pub fn reset_to_speech(&mut self) {
        self.state = VadState::Speech;
        self.voiced_frames = 0;
        self.silence_ms = 0;
    }

    pub fn accept(&mut self, frame: VadFrame) -> VadTransition {
        match (self.state, frame) {
            (VadState::Silence, VadFrame::Speech) => {
                self.voiced_frames = self.voiced_frames.saturating_add(1);
                self.silence_ms = 0;
                if self.voiced_frames >= self.policy.min_start_voiced_frames {
                    self.state = VadState::Speech;
                    self.voiced_frames = 0;
                    VadTransition::SpeechStarted
                } else {
                    VadTransition::None
                }
            }
            (VadState::Silence, VadFrame::Silence) => {
                self.voiced_frames = 0;
                VadTransition::None
            }
            (VadState::Speech, VadFrame::Speech) => {
                self.silence_ms = 0;
                VadTransition::None
            }
            (VadState::Speech, VadFrame::Silence) => {
                self.silence_ms = self.silence_ms.saturating_add(self.policy.frame_ms);
                if self.silence_ms >= self.policy.pause_silence_ms {
                    self.state = VadState::Silence;
                    self.silence_ms = 0;
                    VadTransition::SilenceStarted
                } else {
                    VadTransition::None
                }
            }
        }
    }

    pub fn accept_probability(&mut self, probability: f32) -> VadTransition {
        let frame = match self.state {
            VadState::Silence if probability >= self.policy.speech_threshold => VadFrame::Speech,
            VadState::Speech if probability >= self.policy.silence_threshold => VadFrame::Speech,
            _ => VadFrame::Silence,
        };
        self.accept(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_starts_speech_after_min_voiced_frames() {
        let mut controller = VadController::new(VadPolicy {
            min_start_voiced_frames: 2,
            pause_silence_ms: 1500,
            frame_ms: 32,
            speech_threshold: 0.5,
            silence_threshold: 0.35,
        });

        assert_eq!(controller.accept(VadFrame::Speech), VadTransition::None);
        assert_eq!(
            controller.accept(VadFrame::Speech),
            VadTransition::SpeechStarted
        );
        assert_eq!(controller.state(), VadState::Speech);
    }

    #[test]
    fn controller_pauses_only_after_continuous_silence_window() {
        let mut controller = VadController::new(VadPolicy {
            min_start_voiced_frames: 1,
            pause_silence_ms: 96,
            frame_ms: 32,
            speech_threshold: 0.5,
            silence_threshold: 0.35,
        });

        assert_eq!(
            controller.accept(VadFrame::Speech),
            VadTransition::SpeechStarted
        );
        assert_eq!(controller.accept(VadFrame::Silence), VadTransition::None);
        assert_eq!(controller.accept(VadFrame::Silence), VadTransition::None);
        assert_eq!(
            controller.accept(VadFrame::Silence),
            VadTransition::SilenceStarted
        );
        assert_eq!(controller.state(), VadState::Silence);
    }

    #[test]
    fn speech_resets_pending_silence() {
        let mut controller = VadController::new(VadPolicy {
            min_start_voiced_frames: 1,
            pause_silence_ms: 96,
            frame_ms: 32,
            speech_threshold: 0.5,
            silence_threshold: 0.35,
        });

        assert_eq!(
            controller.accept(VadFrame::Speech),
            VadTransition::SpeechStarted
        );
        assert_eq!(controller.accept(VadFrame::Silence), VadTransition::None);
        assert_eq!(controller.accept(VadFrame::Speech), VadTransition::None);
        assert_eq!(controller.accept(VadFrame::Silence), VadTransition::None);
        assert_eq!(controller.accept(VadFrame::Silence), VadTransition::None);
        assert_eq!(
            controller.accept(VadFrame::Silence),
            VadTransition::SilenceStarted
        );
    }

    #[test]
    fn policy_zero_values_are_normalized() {
        let mut controller = VadController::new(VadPolicy {
            min_start_voiced_frames: 0,
            pause_silence_ms: 0,
            frame_ms: 0,
            speech_threshold: 0.0,
            silence_threshold: 1.0,
        });

        assert_eq!(
            controller.accept(VadFrame::Speech),
            VadTransition::SpeechStarted
        );
        assert_eq!(
            controller.accept(VadFrame::Silence),
            VadTransition::SilenceStarted
        );
    }

    #[test]
    fn reset_returns_to_initial_silence_state() {
        let mut controller = VadController::new(VadPolicy {
            min_start_voiced_frames: 1,
            pause_silence_ms: 96,
            frame_ms: 32,
            speech_threshold: 0.5,
            silence_threshold: 0.35,
        });

        assert_eq!(
            controller.accept(VadFrame::Speech),
            VadTransition::SpeechStarted
        );
        controller.reset();

        assert_eq!(controller.state(), VadState::Silence);
        assert_eq!(controller.accept(VadFrame::Silence), VadTransition::None);
        assert_eq!(
            controller.accept(VadFrame::Speech),
            VadTransition::SpeechStarted
        );
    }

    #[test]
    fn reset_to_speech_starts_fresh_active_window() {
        let mut controller = VadController::new(VadPolicy {
            min_start_voiced_frames: 2,
            pause_silence_ms: 96,
            frame_ms: 32,
            speech_threshold: 0.5,
            silence_threshold: 0.35,
        });

        controller.reset_to_speech();

        assert_eq!(controller.state(), VadState::Speech);
        assert_eq!(controller.accept(VadFrame::Silence), VadTransition::None);
        assert_eq!(controller.accept(VadFrame::Speech), VadTransition::None);
        assert_eq!(controller.accept(VadFrame::Silence), VadTransition::None);
        assert_eq!(controller.accept(VadFrame::Silence), VadTransition::None);
        assert_eq!(
            controller.accept(VadFrame::Silence),
            VadTransition::SilenceStarted
        );
    }

    #[test]
    fn policy_from_config_uses_explicit_config_values() {
        let policy = policy_from_config(
            &crate::config::VoiceVadCfg {
                min_start_voiced_frames: 2,
                pause_silence_ms: 1_500,
                ..crate::config::VoiceVadCfg::default()
            },
            32,
        );

        assert_eq!(policy.min_start_voiced_frames, 2);
        assert_eq!(policy.pause_silence_ms, 1_500);
        assert_eq!(policy.frame_ms, 32);
        assert_eq!(policy.speech_threshold, 0.5);
        assert_eq!(policy.silence_threshold, 0.35);
    }

    #[test]
    fn probability_hysteresis_uses_lower_threshold_after_speech_starts() {
        let mut controller = VadController::new(VadPolicy {
            min_start_voiced_frames: 2,
            pause_silence_ms: 96,
            frame_ms: 32,
            speech_threshold: 0.5,
            silence_threshold: 0.35,
        });

        assert_eq!(controller.accept_probability(0.49), VadTransition::None);
        assert_eq!(controller.accept_probability(0.52), VadTransition::None);
        assert_eq!(
            controller.accept_probability(0.51),
            VadTransition::SpeechStarted
        );
        assert_eq!(controller.accept_probability(0.40), VadTransition::None);
        assert_eq!(controller.accept_probability(0.34), VadTransition::None);
        assert_eq!(controller.accept_probability(0.34), VadTransition::None);
        assert_eq!(
            controller.accept_probability(0.34),
            VadTransition::SilenceStarted
        );
    }
}
