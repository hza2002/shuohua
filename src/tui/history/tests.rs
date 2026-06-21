use super::*;
use crate::history::{
    AsrHistory, AsrSessionHistory, HistoryStatus, PipelineStepHistory, PipelineStepStatus,
};
use crate::tui::history::render::*;

fn sample_record(id: &str, day: u8) -> HistoryRecord {
    let started_at = time::Date::from_calendar_date(2026, time::Month::June, day)
        .unwrap()
        .with_hms(12, 0, 0)
        .unwrap()
        .assume_utc();
    HistoryRecord {
        version: 1,
        id: id.to_string(),
        started_at,
        ended_at: started_at + time::Duration::seconds(3),
        duration_ms: 3000,
        status: HistoryStatus::Submitted,
        app: Some("com.example.App".to_string()),
        text: format!("text {id}"),
        text_stats: crate::text_stats::compute(&format!("text {id}")),
        asr: AsrHistory {
            provider: "apple".to_string(),
            text: format!("asr {id}"),
            duration_ms: 3000,
            audio_ms: 3000,
            sessions: vec![AsrSessionHistory {
                text: format!("asr {id}"),
                started_at,
                ended_at: started_at + time::Duration::seconds(3),
                audio_ms: 3000,
            }],
        },
        pipeline: vec![PipelineStepHistory {
            name: "filler".to_string(),
            status: PipelineStepStatus::Ok,
            duration_ms: 1.0,
            text: Some(format!("text {id}")),
            error: None,
        }],
        error: None,
    }
}

#[test]
fn local_time_format_omits_fraction_and_offset() {
    let value = time::macros::datetime!(2026-06-17 12:34:56.789 UTC);
    let text = format_local_time(value);

    assert!(!text.contains('.'));
    assert!(!text.ends_with('Z'));
    assert_eq!(text.len(), "2026-06-17 12:34:56".len());
}

#[test]
fn short_app_label_uses_bundle_tail() {
    assert_eq!(short_app_label(Some("com.mitchellh.ghostty")), "ghostty");
    assert_eq!(short_app_label(None), "-");
}

#[test]
fn truncate_display_marks_long_values() {
    assert_eq!(truncate_display("Ghostty", 9), "Ghostty");
    assert_eq!(truncate_display("Ghostty", 10), "Ghostty");
    assert_eq!(truncate_display("VeryLongApp", 9), "VeryLong…");
}

#[test]
fn visible_range_keeps_selected_near_middle() {
    assert_eq!(visible_range_for_selection(0, 100, 9), 0..9);
    assert_eq!(visible_range_for_selection(4, 100, 9), 0..9);
    assert_eq!(visible_range_for_selection(20, 100, 9), 16..25);
    assert_eq!(visible_range_for_selection(98, 100, 9), 91..100);
}

#[test]
fn page_requests_older_history_from_oldest_loaded_record() {
    let mut page = HistoryPage::new();
    page.records = vec![
        sample_record("01HXYZABCDEF0123456789AAA1", 3),
        sample_record("01HXYZABCDEF0123456789AAA0", 2),
    ];

    let outcome = page.load_more_outcome();

    assert_eq!(
        outcome.command,
        Some(crate::ipc::protocol::Command::GetHistory {
            limit: HISTORY_PAGE_SIZE,
            before: Some("2026-06-02T12:00:00Z".to_string()),
            query: None,
        })
    );
}

#[test]
fn appending_history_deduplicates_existing_records() {
    let mut page = HistoryPage::new();
    let record = sample_record("01HXYZABCDEF0123456789AAA1", 3);
    page.apply_event(
        &Event::History {
            records: vec![record.clone()],
        },
        true,
    );

    page.apply_event(
        &Event::HistoryAppended {
            record: Box::new(record),
        },
        true,
    );

    assert_eq!(page.records.len(), 1);
}

#[test]
fn appending_history_keeps_existing_selection_on_same_record() {
    let mut page = HistoryPage::new();
    page.records = vec![
        sample_record("01HXYZABCDEF0123456789AAA2", 4),
        sample_record("01HXYZABCDEF0123456789AAA1", 3),
    ];
    page.selected = 1;

    page.apply_event(
        &Event::HistoryAppended {
            record: Box::new(sample_record("01HXYZABCDEF0123456789AAA3", 5)),
        },
        true,
    );

    assert_eq!(page.records[page.selected].id, "01HXYZABCDEF0123456789AAA1");
}

#[test]
fn appending_history_preserves_filtered_selection_when_new_record_does_not_match() {
    let mut page = HistoryPage::new();
    page.records = vec![
        sample_record("01HXYZABCDEF0123456789AAA2", 4),
        sample_record("01HXYZABCDEF0123456789AAA1", 3),
    ];
    page.search = "AAA1".to_string();
    page.selected = 0;

    page.apply_event(
        &Event::HistoryAppended {
            record: Box::new(sample_record("01HXYZABCDEF0123456789AAA3", 5)),
        },
        true,
    );

    assert_eq!(
        page.selected_record().unwrap().id,
        "01HXYZABCDEF0123456789AAA1"
    );
}

#[test]
fn initial_history_preserves_server_order() {
    let mut page = HistoryPage::new();
    page.apply_event(
        &Event::History {
            records: vec![
                sample_record("01HXYZABCDEF0123456789AAA2", 4),
                sample_record("01HXYZABCDEF0123456789AAA1", 3),
                sample_record("01HXYZABCDEF0123456789AAA0", 2),
            ],
        },
        true,
    );

    assert_eq!(page.records[0].id, "01HXYZABCDEF0123456789AAA2");
    assert_eq!(page.records[2].id, "01HXYZABCDEF0123456789AAA0");
}

#[test]
fn load_more_from_empty_history_marks_request_in_flight() {
    let mut page = HistoryPage::new();

    let outcome = page.load_more_outcome();

    assert!(page.loading_more);
    assert!(matches!(
        outcome.command,
        Some(crate::ipc::protocol::Command::GetHistory { before: None, .. })
    ));
}

#[test]
fn loading_more_appends_and_deduplicates_older_records() {
    let mut page = HistoryPage::new();
    let newest = sample_record("01HXYZABCDEF0123456789AAA2", 4);
    let duplicate = sample_record("01HXYZABCDEF0123456789AAA1", 3);
    page.records = vec![newest, duplicate.clone()];
    page.loading_more = true;

    page.apply_event(
        &Event::History {
            records: vec![duplicate, sample_record("01HXYZABCDEF0123456789AAA0", 2)],
        },
        true,
    );

    assert_eq!(page.records.len(), 3);
    assert_eq!(page.records[2].id, "01HXYZABCDEF0123456789AAA0");
}
