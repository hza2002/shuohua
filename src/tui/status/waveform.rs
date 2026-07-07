//! Braille high-resolution audio envelope.
//!
//! Each terminal cell is a 2×4 braille dot grid, so we pack two windows per
//! column (newest on the right) and mirror the RMS level around the center for
//! an oscilloscope-like "音波". Height is the loudness-correlated RMS on a fixed
//! dBFS scale; cells are colored by VAD (speech vs. non-speech). We only ever
//! have per-window `peak`/`rms`/`vad` (50ms), not raw PCM, so this is a level
//! envelope, not a true waveform.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::state::AudioMeter;

/// Braille dot bit for [sub-column 0..2][row 0..4]. Layout per Unicode braille:
/// rows 0..2 are dots 1/2/3 (left) and 4/5/6 (right); row 3 is dots 7/8.
const DOT_BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

pub struct WaveGrid {
    pub w: usize,
    pub h: usize,
    cells: Vec<u8>,
    speech: Vec<bool>,
}

impl WaveGrid {
    fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            cells: vec![0; w * h],
            speech: vec![false; w * h],
        }
    }

    fn set(&mut self, sub_x: usize, dot_y: usize) {
        let cx = sub_x / 2;
        let cy = dot_y / 4;
        if cx >= self.w || cy >= self.h {
            return;
        }
        self.cells[cy * self.w + cx] |= DOT_BITS[sub_x % 2][dot_y % 4];
    }

    fn mark_speech(&mut self, sub_x: usize) {
        let cx = sub_x / 2;
        if cx >= self.w {
            return;
        }
        for cy in 0..self.h {
            self.speech[cy * self.w + cx] = true;
        }
    }

    #[cfg(test)]
    fn dot_lit(&self, sub_x: usize, dot_y: usize) -> bool {
        let cx = sub_x / 2;
        let cy = dot_y / 4;
        self.cells[cy * self.w + cx] & DOT_BITS[sub_x % 2][dot_y % 4] != 0
    }

    #[cfg(test)]
    fn cell_is_speech(&self, cx: usize, cy: usize) -> bool {
        self.speech[cy * self.w + cx]
    }

    /// Render the grid to colored lines, top row first. Speech cells use
    /// `speech`, the rest `silent`.
    pub fn to_lines(&self, speech: Color, silent: Color) -> Vec<Line<'static>> {
        (0..self.h)
            .map(|cy| {
                let spans = (0..self.w)
                    .map(|cx| {
                        let bits = self.cells[cy * self.w + cx];
                        let ch = char::from_u32(0x2800 + bits as u32).unwrap_or(' ');
                        let color = if self.speech[cy * self.w + cx] {
                            speech
                        } else {
                            silent
                        };
                        Span::styled(ch.to_string(), Style::default().fg(color))
                    })
                    .collect::<Vec<_>>();
                Line::from(spans)
            })
            .collect()
    }
}

/// Bottom of the meter scale in dBFS. This is a *fixed*, calibrated scale — not
/// an auto-range and not a fitted magic number: the top is 0 dBFS (digital
/// full scale, amplitude 1.0) and the floor is the conventional level-meter
/// bottom (DAWs/OBS mark −60..0 dBFS). Because the scale is absolute, silence
/// reads as silence instead of being amplified.
const FLOOR_DB: f32 = -60.0;

/// Height (0..1) for a linear peak on the fixed dBFS scale.
fn dbfs_level(peak: f32) -> f32 {
    let peak = peak.clamp(0.0, 1.0);
    if peak <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * peak.log10();
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

/// Voice-activity classification for a window: `Some(true/false)` from the VAD,
/// or `None` when no VAD info is available.
fn frame_speech(m: &AudioMeter) -> Option<bool> {
    m.vad_speech.or_else(|| m.vad_probability.map(|p| p >= 0.5))
}

/// Build a mirrored level-envelope braille grid from the meter history. The most
/// recent meters occupy the rightmost sub-columns. There is no always-on
/// baseline: a frame with no level draws nothing, so real silence looks calm
/// instead of leaving a constant band across the meter.
///
/// Height uses **RMS** (the loudness-correlated measure), not peak — peak is a
/// spiky, transient-driven clip indicator that would keep ambient noise lit as a
/// jittery band. Height and color are independent: **height** is the fixed dBFS
/// RMS level, and **color** marks whether the VAD classified that frame as
/// speech, so a loud non-speech sound is tall but muted-colored.
pub fn build_wave_grid(meters: &[AudioMeter], w: usize, h: usize) -> WaveGrid {
    let w = w.max(1);
    let h = h.max(1);
    let mut grid = WaveGrid::new(w, h);
    let sub_cols = w * 2;
    let half = (2 * h) as f32; // dots from center to top/bottom edge
    let center_up = 2 * h - 1; // dot just above center
    let center_dn = 2 * h; // dot just below center
    let n = meters.len();
    for sub_x in 0..sub_cols {
        // Right-align: rightmost sub-column is the newest meter.
        let from_right = sub_cols - 1 - sub_x;
        let Some(m) = (from_right < n).then(|| &meters[n - 1 - from_right]) else {
            continue;
        };
        // Rounding to whole dots acts as a soft gate: very quiet ambient rounds
        // to zero (calm silence), while audible signal shows proportionally.
        let lit = (dbfs_level(m.rms) * half).round() as usize;
        for k in 0..lit {
            if center_up >= k {
                grid.set(sub_x, center_up - k);
            }
            grid.set(sub_x, center_dn + k);
        }
        if frame_speech(m) == Some(true) {
            grid.mark_speech(sub_x);
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meter(peak: f32, speech: bool) -> AudioMeter {
        AudioMeter {
            rms: peak,
            peak,
            clipped: false,
            vad_probability: None,
            vad_speech: Some(speech),
        }
    }

    fn meter_no_vad(peak: f32) -> AudioMeter {
        AudioMeter {
            rms: peak,
            peak,
            clipped: false,
            vad_probability: None,
            vad_speech: None,
        }
    }

    #[test]
    fn height_tracks_rms_not_peak() {
        // A brief transient: high peak, low sustained RMS. The meter should read
        // low, because loudness (RMS) drives height — clicks don't inflate it.
        let spike = AudioMeter {
            rms: 0.02,
            peak: 0.9,
            clipped: false,
            vad_probability: None,
            vad_speech: Some(true),
        };
        let grid = build_wave_grid(&[spike], 1, 1);
        assert!(
            !grid.dot_lit(1, 0),
            "a transient peak with low RMS stays low"
        );
    }

    #[test]
    fn dbfs_level_is_a_fixed_absolute_scale() {
        assert_eq!(dbfs_level(0.0), 0.0, "silence -> baseline");
        assert!((dbfs_level(1.0) - 1.0).abs() < 1e-6, "0 dBFS -> full");
        // -60 dBFS floor: 0.001 (=-60 dBFS) sits at the very bottom.
        assert!(dbfs_level(0.001) < 0.02);
        // Monotonic, and quiet speech is clearly visible (not linear-tiny):
        assert!(dbfs_level(0.1) > dbfs_level(0.03));
        assert!(
            dbfs_level(0.05) > 0.4,
            "quiet speech is visible: {}",
            dbfs_level(0.05)
        );
    }

    #[test]
    fn height_reflects_level_independent_of_speech() {
        // Ambient noise (non-speech) still shows its real level as height, so the
        // meter proves the mic is receiving audio — the VAD only colors it.
        let ambient = build_wave_grid(&[meter(0.5, false)], 1, 1);
        assert!(
            ambient.dot_lit(1, 0),
            "loud non-speech still draws an envelope"
        );
        assert!(
            !ambient.cell_is_speech(0, 0),
            "but it is not colored as speech"
        );

        // Quiet ambient sits low; speech sits high — height, not VAD, sets size.
        let quiet = build_wave_grid(&[meter(0.01, false)], 1, 1);
        assert!(
            !quiet.dot_lit(1, 0),
            "quiet ambient stays near the baseline"
        );
    }

    #[test]
    fn height_is_shown_with_or_without_vad() {
        // No VAD info -> height still shown (works if the user disables VAD).
        let grid = build_wave_grid(&[meter_no_vad(1.0)], 1, 1);
        assert!(grid.dot_lit(1, 0), "full peak fills even without VAD");
        assert!(!grid.cell_is_speech(0, 0), "no VAD -> not speech-colored");
    }

    #[test]
    fn silence_draws_nothing() {
        // No always-on baseline: empty history and a digitally-silent frame both
        // leave the meter blank, so real silence looks calm.
        let empty = build_wave_grid(&[], 1, 1);
        for dot_y in 0..4 {
            assert!(!empty.dot_lit(0, dot_y) && !empty.dot_lit(1, dot_y));
        }
        let silent = build_wave_grid(&[meter(0.0, false)], 1, 1);
        for dot_y in 0..4 {
            assert!(!silent.dot_lit(1, dot_y), "zero level draws no dots");
        }
    }

    #[test]
    fn full_peak_fills_the_column_top_to_bottom() {
        // Speech at 0 dBFS (peak=1) fills all 4 dots of the rightmost sub-column.
        let grid = build_wave_grid(&[meter(1.0, true)], 1, 1);
        for dot_y in 0..4 {
            assert!(
                grid.dot_lit(1, dot_y),
                "dot {dot_y} should be lit at full peak"
            );
        }
        // The unfilled left sub-column keeps only the baseline.
        assert!(!grid.dot_lit(0, 0));
    }

    #[test]
    fn speech_meter_marks_its_cell() {
        let silent = build_wave_grid(&[meter(0.5, false)], 1, 1);
        assert!(!silent.cell_is_speech(0, 0));
        let speaking = build_wave_grid(&[meter(0.5, true)], 1, 1);
        assert!(speaking.cell_is_speech(0, 0));
    }

    #[test]
    fn newest_meter_lands_on_the_rightmost_column() {
        // Two cells (4 sub-columns), single loud+speech meter -> only rightmost.
        let grid = build_wave_grid(&[meter(1.0, true)], 2, 1);
        assert!(grid.cell_is_speech(1, 0), "newest maps to the right cell");
        assert!(!grid.cell_is_speech(0, 0), "left cell stays silent padding");
    }
}
