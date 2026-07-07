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
fn meter_lines_render_braille_rows_plus_legend() {
    crate::i18n::init("en-US");
    let mut page = StatusPage::new();
    page.state = WireState::Recording;
    page.meters = vec![meter(0.8)];
    let theme = TuiTheme::default();

    let lines = meter_lines(&page, &theme, 20, 4);
    assert_eq!(lines.len(), 4, "3 braille rows + 1 legend line");

    let waveform: String = lines[..3]
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect();
    assert!(
        waveform
            .chars()
            .any(|c| ('\u{2800}'..='\u{28FF}').contains(&c)),
        "waveform rows use braille glyphs"
    );

    let legend: String = lines[3]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    assert!(
        legend.contains("peak"),
        "legend shows peak readout: {legend}"
    );
    assert!(legend.contains("speech"), "legend shows VAD key: {legend}");
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
