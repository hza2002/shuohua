use super::*;
use crate::tui::status::render::*;

fn meter(peak: f32) -> AudioMeter {
    AudioMeter {
        rms: peak,
        peak,
        clipped: false,
        vad_probability: None,
        vad_speech: None,
    }
}

#[test]
fn trim_meters_to_capacity_keeps_large_tail() {
    let mut page = StatusPage::new();
    page.meters = (0..1100).map(|idx| meter(idx as f32)).collect::<Vec<_>>();

    page.trim_meters_to_capacity();

    assert_eq!(page.meters.len(), MAX_METER_HISTORY);
    assert_eq!(page.meters.first().unwrap().peak, 76.0);
    assert_eq!(page.meters.last().unwrap().peak, 1099.0);
}

#[test]
fn meter_capacity_tracks_terminal_width_with_minimum_and_4k_cap() {
    assert_eq!(StatusPage::meter_capacity_for_terminal_width(200), 189);
    assert_eq!(StatusPage::meter_capacity_for_terminal_width(20), 16);
    assert_eq!(
        StatusPage::meter_capacity_for_terminal_width(3840),
        MAX_METER_HISTORY
    );
}

#[test]
fn waveform_levels_use_low_and_high_blocks() {
    assert_eq!(upper_level(0.0), '▁');
    assert_eq!(upper_level(1.0), '█');
    assert_eq!(lower_level(0.0), '▔');
    assert_eq!(lower_level(1.0), '▁');
}

#[test]
fn audio_lines_render_one_char_per_meter() {
    let meters = vec![
        AudioMeter {
            rms: 0.0,
            peak: 0.0,
            clipped: false,
            vad_probability: Some(0.0),
            vad_speech: Some(false),
        },
        AudioMeter {
            rms: 1.0,
            peak: 1.0,
            clipped: true,
            vad_probability: Some(1.0),
            vad_speech: Some(true),
        },
    ];

    assert_eq!(audio_upper(&meters).chars().count(), 2);
    assert_eq!(audio_lower(&meters).chars().count(), 2);
}

#[test]
fn status_header_includes_colored_asr_metadata() {
    crate::i18n::init("en-US");
    let page = StatusPage::new();
    let theme = TuiTheme::default();

    let lines = status_header_lines(
        &page,
        &theme,
        "Ghostty",
        "com.mitchellh.ghostty",
        "apple",
        "-",
        0,
    );
    let text = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.contains("Ghostty"));
    assert!(text.contains("asr apple"));
    assert!(text.contains("chain -"));
    assert_eq!(lines.len(), 3);
}

#[test]
fn ignores_events_for_stale_recordings() {
    let mut page = StatusPage::new();
    page.recording_id = Some("current".to_string());

    page.apply_event(
        &Event::Partial {
            recording_id: "old".to_string(),
            text: "stale".to_string(),
        },
        true,
    );
    page.apply_event(
        &Event::Segment {
            recording_id: "old".to_string(),
            text: "stale segment".to_string(),
        },
        true,
    );
    page.apply_event(
        &Event::AudioMeter {
            recording_id: "old".to_string(),
            meter: meter(0.5),
        },
        true,
    );

    assert!(page.partial.is_empty());
    assert!(page.segments.is_empty());
    assert!(page.meters.is_empty());
}

#[test]
fn live_speech_keeps_tail_when_space_is_limited() {
    let mut page = StatusPage::new();
    page.segments = vec!["abcdefghijklmnopqrstuvwxyz".to_string()];
    page.partial = "0123456789".to_string();
    let theme = TuiTheme::default();

    let line = live_speech_lines(&page, &theme, 10, 1);
    let text = line[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.starts_with("... "));
    assert!(text.ends_with("456789"));
    assert!(display_width(&text) <= 16);
}

#[test]
fn live_speech_keeps_tail_for_wide_cjk_text() {
    let mut page = StatusPage::new();
    page.segments = vec!["这是很长很长的一段已经定型的语音识别文本".to_string()];
    page.partial = "最新的部分".to_string();
    let theme = TuiTheme::default();

    let line = live_speech_lines(&page, &theme, 16, 1);
    let text = line[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.starts_with("... "));
    assert!(text.ends_with("最新的部分"));
    assert!(display_width(&text) <= 16);
}
