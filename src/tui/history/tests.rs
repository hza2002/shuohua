use super::*;
use crate::history::{
    AggregateStats, AnalyticsPeriod, AnalyticsPoint, AnalyticsSnapshot, AsrHistory,
    AsrSessionHistory, HistoryStatsSnapshot, HistoryStatsStatus, HistoryStatus,
    PipelineStepHistory, PipelineStepStatus,
};
use crate::tui::history::render::*;
use crate::tui::ui;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

fn stats(total_records: u64, total_words: u64, total_duration_ms: u64) -> HistoryStatsSnapshot {
    HistoryStatsSnapshot {
        status: HistoryStatsStatus::Ready,
        total: AggregateStats {
            records: total_records,
            words: total_words,
            duration_ms: total_duration_ms,
            asr_duration_ms: total_duration_ms.saturating_sub(10_000),
            asr_audio_ms: total_duration_ms / 2,
        },
        current_month: AggregateStats::default(),
        today: AggregateStats::default(),
        error: None,
    }
}

fn analytics(period: AnalyticsPeriod, status: HistoryStatsStatus) -> AnalyticsSnapshot {
    AnalyticsSnapshot {
        status,
        period,
        anchor: match period {
            AnalyticsPeriod::Year => "2026",
            AnalyticsPeriod::Month => "2026-06",
            AnalyticsPeriod::Day => "2026-06-17",
        }
        .to_string(),
        points: vec![
            AnalyticsPoint {
                key: "a".to_string(),
                stats: AggregateStats {
                    records: 1,
                    words: 2,
                    duration_ms: 3_000,
                    asr_duration_ms: 3_500,
                    asr_audio_ms: 4_000,
                },
            },
            AnalyticsPoint {
                key: "b".to_string(),
                stats: AggregateStats {
                    records: 3,
                    words: 4,
                    duration_ms: 5_000,
                    asr_duration_ms: 5_500,
                    asr_audio_ms: 6_000,
                },
            },
        ],
        error: (status == HistoryStatsStatus::Stale).then(|| "index stale".to_string()),
    }
}

fn analytics_with_anchor(
    period: AnalyticsPeriod,
    anchor: &str,
    status: HistoryStatsStatus,
) -> AnalyticsSnapshot {
    AnalyticsSnapshot {
        anchor: anchor.to_string(),
        ..analytics(period, status)
    }
}

fn press(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
}

fn error(kind: &str) -> Event {
    Event::Error {
        recording_id: None,
        kind: kind.to_string(),
        msg: "boom".to_string(),
    }
}

fn history_event(records: Vec<HistoryRecord>) -> Event {
    Event::History {
        records,
        matched: None,
        stats: None,
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
    assert_eq!(ui::visible_range_for_selection(0, 100, 9), 0..9);
    assert_eq!(ui::visible_range_for_selection(4, 100, 9), 0..9);
    assert_eq!(ui::visible_range_for_selection(20, 100, 9), 16..25);
    assert_eq!(ui::visible_range_for_selection(98, 100, 9), 91..100);
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
            before_id: Some("01HXYZABCDEF0123456789AAA0".to_string()),
            query: None,
        })
    );
}

#[test]
fn first_history_entry_requests_page_stats_and_visible_analytics() {
    let mut page = HistoryPage::new();

    let commands = page.enter_commands();

    assert_eq!(
        commands,
        vec![
            crate::ipc::protocol::Command::GetHistory {
                limit: HISTORY_PAGE_SIZE,
                before: None,
                before_id: None,
                query: None,
            },
            crate::ipc::protocol::Command::GetHistoryStats,
            crate::ipc::protocol::Command::GetHistoryAnalytics {
                period: AnalyticsPeriod::Month,
                anchor: page.analytics.selection.anchor.clone(),
            },
        ]
    );
}

#[test]
fn history_append_before_initial_load_only_marks_refresh_needed() {
    let mut page = HistoryPage::new();

    page.apply_event(&Event::HistoryChanged, true);

    assert!(page.refresh_needed);
    assert!(page.records.is_empty());
}

#[test]
fn records_summary_shows_compact_totals_without_loaded_count() {
    let mut page = HistoryPage::new();
    page.records = vec![
        sample_record("01HXYZABCDEF0123456789AAA1", 3),
        sample_record("01HXYZABCDEF0123456789AAA2", 4),
    ];
    page.apply_event(
        &Event::HistoryStats {
            snapshot: stats(10, 500, 3_723_000),
        },
        true,
    );

    let summary = history_summary_text(&page);

    assert!(summary.contains("10 records"));
    assert!(summary.contains("500 words"));
    assert!(summary.contains("Total 1:02:03"));
    assert!(summary.contains("Speech 1:01:53"));
    assert!(summary.contains("Effective 31:01"));
    assert!(!summary.contains("loaded"));
    assert!(!summary.contains("matched"));
}

#[test]
fn search_summary_uses_hit_ratio_and_total_stats() {
    let mut page = HistoryPage::new();
    page.records = vec![
        sample_record("01HXYZABCDEF0123456789AAA1", 3),
        sample_record("01HXYZABCDEF0123456789AAA2", 4),
    ];
    page.search = "AAA1".to_string();
    page.search_stats = Some(SearchStats {
        query: "AAA1".to_string(),
        matched: 23,
        stats: AggregateStats {
            records: 23,
            words: 900,
            duration_ms: 456_000,
            asr_duration_ms: 400_000,
            asr_audio_ms: 300_000,
        },
    });
    page.apply_event(
        &Event::HistoryStats {
            snapshot: stats(555, 500, 123_000),
        },
        true,
    );

    let summary = history_summary_text(&page);

    assert!(summary.contains("23/555 records"));
    assert!(summary.contains("900 words"));
    assert!(summary.contains("Total 07:36"));
    assert!(!summary.contains("loaded"));
    assert!(!summary.contains("matched"));
}

#[test]
fn search_summary_waits_for_daemon_match_count() {
    let mut page = HistoryPage::new();
    page.records = vec![sample_record("01HXYZABCDEF0123456789AAA1", 3)];
    page.search = "AAA1".to_string();
    page.apply_event(
        &Event::HistoryStats {
            snapshot: stats(10, 500, 123_000),
        },
        true,
    );

    let summary = history_summary_text(&page);

    assert!(summary.contains("?/10 records"));
    assert!(summary.contains("- words"));
}

#[test]
fn near_tail_navigation_auto_loads_more_history() {
    let mut page = HistoryPage::new();
    page.records = (0..HISTORY_PAGE_SIZE)
        .map(|idx| {
            sample_record(
                &format!("01HXYZABCDEF0123456789A{idx:03}"),
                (idx % 28 + 1) as u8,
            )
        })
        .collect();
    page.selected = HISTORY_PAGE_SIZE - 21;

    let outcome = page.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

    assert!(page.loading_more);
    assert!(outcome.status.is_none());
    assert!(matches!(
        outcome.command,
        Some(crate::ipc::protocol::Command::GetHistory {
            before: Some(_),
            ..
        })
    ));
}

#[test]
fn details_show_total_speech_and_effective_audio_durations() {
    let page = HistoryPage::new();
    let mut record = sample_record("01HXYZABCDEF0123456789AAA1", 3);
    record.duration_ms = 390_000;
    record.asr.duration_ms = 320_000;
    record.asr.audio_ms = 250_000;

    let text = history_detail_text(
        &page,
        &crate::config::theme::TuiTheme::default(),
        &record,
        HistoryDetail::Details,
    )
    .into_iter()
    .map(|line| line.to_string())
    .collect::<Vec<_>>()
    .join("\n");

    assert!(text.contains("total: 06:30"));
    assert!(text.contains("speech: 05:20"));
    assert!(text.contains("effective: 04:10"));
}

#[test]
fn asr_detail_shows_speech_and_effective_audio_durations() {
    let page = HistoryPage::new();
    let mut record = sample_record("01HXYZABCDEF0123456789AAA1", 3);
    record.asr.duration_ms = 320_000;
    record.asr.audio_ms = 250_000;

    let text = history_detail_text(
        &page,
        &crate::config::theme::TuiTheme::default(),
        &record,
        HistoryDetail::Asr,
    )
    .into_iter()
    .map(|line| line.to_string())
    .collect::<Vec<_>>()
    .join("\n");

    assert!(text.contains("speech: 05:20"));
    assert!(text.contains("effective: 04:10"));
}

#[test]
fn search_sends_query_to_daemon() {
    let mut page = HistoryPage::new();
    page.start_search();

    page.on_key(press('a'));
    let outcome = page.on_key(press('b'));

    assert_eq!(
        outcome.command,
        Some(crate::ipc::protocol::Command::GetHistory {
            limit: HISTORY_PAGE_SIZE,
            before: None,
            before_id: None,
            query: Some("ab".to_string()),
        })
    );
}

#[test]
fn load_more_while_searching_sends_query_to_daemon() {
    let mut page = HistoryPage::new();
    page.records = vec![sample_record("01HXYZABCDEF0123456789AAA1", 3)];
    page.search = "needle".to_string();

    let outcome = page.load_more_outcome();

    assert_eq!(
        outcome.command,
        Some(crate::ipc::protocol::Command::GetHistory {
            limit: HISTORY_PAGE_SIZE,
            before: Some("2026-06-03T12:00:00Z".to_string()),
            before_id: Some("01HXYZABCDEF0123456789AAA1".to_string()),
            query: Some("needle".to_string()),
        })
    );
}

#[test]
fn esc_clears_search_by_requesting_unfiltered_page() {
    let mut page = HistoryPage::new();
    page.search = "needle".to_string();

    let outcome = page.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(
        outcome.command,
        Some(crate::ipc::protocol::Command::GetHistory {
            limit: HISTORY_PAGE_SIZE,
            before: None,
            before_id: None,
            query: None,
        })
    );
}

#[test]
fn search_mode_esc_clears_search_by_requesting_unfiltered_page() {
    let mut page = HistoryPage::new();
    page.start_search();
    page.on_key(press('a'));

    let outcome = page.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(
        outcome.command,
        Some(crate::ipc::protocol::Command::GetHistory {
            limit: HISTORY_PAGE_SIZE,
            before: None,
            before_id: None,
            query: None,
        })
    );
    assert!(page.search.is_empty());
    assert!(!page.searching);
}

#[test]
fn analytics_switches_year_month_day_periods() {
    let mut page = HistoryPage::new();

    page.on_key(press('s'));
    let month = page.on_key(press('p')).command;
    let day = page.on_key(press('p')).command;
    let year = page.on_key(press('p')).command;

    assert!(matches!(
        month,
        Some(crate::ipc::protocol::Command::GetHistoryAnalytics {
            period: AnalyticsPeriod::Day,
            ..
        })
    ));
    assert!(matches!(
        day,
        Some(crate::ipc::protocol::Command::GetHistoryAnalytics {
            period: AnalyticsPeriod::Year,
            ..
        })
    ));
    assert!(matches!(
        year,
        Some(crate::ipc::protocol::Command::GetHistoryAnalytics {
            period: AnalyticsPeriod::Month,
            ..
        })
    ));
}

#[test]
fn analytics_switches_metric_and_chart_kind() {
    let mut page = HistoryPage::new();
    page.on_key(press('s'));

    page.on_key(press('v'));
    assert_eq!(page.analytics.selection.metric, AnalyticsMetric::Words);
    page.on_key(press('v'));
    assert_eq!(page.analytics.selection.metric, AnalyticsMetric::Duration);
    page.on_key(press('c'));
    assert_eq!(page.analytics.selection.chart, AnalyticsChart::Line);
}

#[test]
fn analytics_switches_previous_and_next_anchor() {
    let mut page = HistoryPage::new();
    page.on_key(press('s'));
    page.analytics.selection.period = AnalyticsPeriod::Month;
    page.analytics.selection.anchor = "2026-06".to_string();

    let previous = page.on_key(press('[')).command;
    let next = page.on_key(press(']')).command;

    assert!(matches!(
        previous,
        Some(crate::ipc::protocol::Command::GetHistoryAnalytics {
            period: AnalyticsPeriod::Month,
            ref anchor,
        }) if anchor == "2026-05"
    ));
    assert!(matches!(
        next,
        Some(crate::ipc::protocol::Command::GetHistoryAnalytics {
            period: AnalyticsPeriod::Month,
            ref anchor,
        }) if anchor == "2026-06"
    ));
}

#[test]
fn history_changed_coalesces_one_refresh_batch() {
    let mut page = HistoryPage::new();
    page.analytics.selection.period = AnalyticsPeriod::Month;
    page.analytics.selection.anchor = "2026-06".to_string();
    page.enter_commands();

    page.apply_event(&Event::HistoryChanged, true);
    page.apply_event(&Event::HistoryChanged, true);
    page.apply_event(&history_event(vec![]), true);
    page.apply_event(
        &Event::HistoryStats {
            snapshot: stats(0, 0, 0),
        },
        true,
    );
    page.apply_event(
        &Event::HistoryAnalytics {
            snapshot: analytics(AnalyticsPeriod::Month, HistoryStatsStatus::Ready),
        },
        true,
    );
    let first = page.refresh_commands();
    let second = page.refresh_commands();

    assert_eq!(first.len(), 3);
    assert!(second.is_empty());
}

#[test]
fn standalone_analytics_response_does_not_complete_refresh_batch() {
    let mut page = HistoryPage::new();
    page.on_key(press('s'));
    page.analytics.selection.period = AnalyticsPeriod::Month;
    page.analytics.selection.anchor = "2026-06".to_string();
    page.on_key(press(']'));
    page.apply_event(&Event::HistoryChanged, true);

    assert_eq!(page.refresh_commands().len(), 3);
    page.apply_event(
        &Event::HistoryAnalytics {
            snapshot: analytics_with_anchor(
                AnalyticsPeriod::Month,
                "2026-07",
                HistoryStatsStatus::Ready,
            ),
        },
        true,
    );

    assert!(page.refresh_in_flight);
    assert!(page.refresh_commands().is_empty());
}

#[test]
fn delete_confirmation_returns_an_ipc_command() {
    let mut page = HistoryPage::new();
    page.records = vec![sample_record("01HXYZABCDEF0123456789AAA1", 3)];

    page.on_key(press('d'));
    let delete_audio = page.feed_confirm_key(press('y')).unwrap();
    page.on_key(press('x'));
    let delete_history = page.feed_confirm_key(press('y')).unwrap();

    assert!(matches!(
        delete_audio.command,
        Some(crate::ipc::protocol::Command::DeleteAudio { .. })
    ));
    assert!(matches!(
        delete_history.command,
        Some(crate::ipc::protocol::Command::DeleteHistory { .. })
    ));
}

#[test]
fn delete_response_and_history_changed_are_order_independent() {
    let mut page = HistoryPage::new();
    page.records = vec![sample_record("01HXYZABCDEF0123456789AAA1", 3)];

    page.apply_event(&Event::HistoryChanged, true);
    page.apply_event(
        &Event::AudioDeleted {
            id: "01HXYZABCDEF0123456789AAA1".to_string(),
            deleted: true,
        },
        true,
    );
    let first = page.refresh_commands();
    page.apply_event(
        &Event::HistoryDeleted {
            id: "01HXYZABCDEF0123456789AAA1".to_string(),
            record_deleted: true,
            audio_deleted: true,
            audio_error: None,
        },
        true,
    );

    assert_eq!(first.len(), 3);
    assert!(page.refresh_commands().is_empty());
}

#[test]
fn stale_snapshot_keeps_last_valid_chart_and_shows_warning() {
    let mut page = HistoryPage::new();
    page.analytics.selection.period = AnalyticsPeriod::Month;
    page.analytics.selection.anchor = "2026-06".to_string();
    page.apply_event(
        &Event::HistoryAnalytics {
            snapshot: analytics(AnalyticsPeriod::Month, HistoryStatsStatus::Ready),
        },
        true,
    );
    let ready_points = page.analytics.snapshot.as_ref().unwrap().points.clone();

    page.apply_event(
        &Event::HistoryAnalytics {
            snapshot: analytics(AnalyticsPeriod::Month, HistoryStatsStatus::Stale),
        },
        true,
    );

    assert_eq!(
        page.analytics.snapshot.as_ref().unwrap().points,
        ready_points
    );
    assert!(page
        .analytics
        .warning
        .as_deref()
        .unwrap_or_default()
        .contains("stale"));
}

#[test]
fn stale_snapshot_without_error_uses_fallback_warning() {
    let mut page = HistoryPage::new();
    page.analytics.selection.period = AnalyticsPeriod::Month;
    page.analytics.selection.anchor = "2026-06".to_string();
    let mut snapshot = analytics(AnalyticsPeriod::Month, HistoryStatsStatus::Stale);
    snapshot.error = None;

    page.apply_event(&Event::HistoryAnalytics { snapshot }, true);

    assert!(!page
        .analytics
        .warning
        .as_deref()
        .unwrap_or_default()
        .is_empty());
}

#[test]
fn stale_analytics_response_for_old_selection_is_ignored() {
    let mut page = HistoryPage::new();
    page.analytics.selection.period = AnalyticsPeriod::Month;
    page.analytics.selection.anchor = "2026-07".to_string();

    page.apply_event(
        &Event::HistoryAnalytics {
            snapshot: analytics(AnalyticsPeriod::Month, HistoryStatsStatus::Ready),
        },
        true,
    );

    assert!(page.analytics.snapshot.is_none());
}

#[test]
fn history_changed_keeps_existing_records() {
    let mut page = HistoryPage::new();
    let record = sample_record("01HXYZABCDEF0123456789AAA1", 3);
    page.enter_commands();
    page.apply_event(&history_event(vec![record.clone()]), true);

    page.apply_event(&Event::HistoryChanged, true);

    assert_eq!(page.records.len(), 1);
}

#[test]
fn legacy_history_appended_deduplicates_existing_records() {
    let mut page = HistoryPage::new();
    let record = sample_record("01HXYZABCDEF0123456789AAA1", 3);
    page.enter_commands();
    page.apply_event(&history_event(vec![record.clone()]), true);

    page.apply_event(
        &Event::HistoryAppended {
            record: Box::new(record),
        },
        true,
    );

    assert_eq!(page.records.len(), 1);
}

#[test]
fn history_changed_keeps_existing_selection_on_same_record() {
    let mut page = HistoryPage::new();
    page.records = vec![
        sample_record("01HXYZABCDEF0123456789AAA2", 4),
        sample_record("01HXYZABCDEF0123456789AAA1", 3),
    ];
    page.selected = 1;

    page.apply_event(&Event::HistoryChanged, true);

    assert_eq!(page.records[page.selected].id, "01HXYZABCDEF0123456789AAA1");
}

#[test]
fn history_changed_preserves_selection() {
    let mut page = HistoryPage::new();
    page.records = vec![
        sample_record("01HXYZABCDEF0123456789AAA2", 4),
        sample_record("01HXYZABCDEF0123456789AAA1", 3),
    ];
    page.selected = 1;

    page.apply_event(&Event::HistoryChanged, true);

    // HistoryChanged only marks a refresh; the loaded page (the server's match
    // set) and the current selection index are left untouched.
    assert_eq!(
        page.selected_record().unwrap().id,
        "01HXYZABCDEF0123456789AAA1"
    );
}

#[test]
fn initial_history_preserves_server_order() {
    let mut page = HistoryPage::new();
    page.enter_commands();
    page.apply_event(
        &history_event(vec![
            sample_record("01HXYZABCDEF0123456789AAA2", 4),
            sample_record("01HXYZABCDEF0123456789AAA1", 3),
            sample_record("01HXYZABCDEF0123456789AAA0", 2),
        ]),
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
    page.load_more_outcome();

    page.apply_event(
        &history_event(vec![
            duplicate,
            sample_record("01HXYZABCDEF0123456789AAA0", 2),
        ]),
        true,
    );

    assert_eq!(page.records.len(), 3);
    assert_eq!(page.records[2].id, "01HXYZABCDEF0123456789AAA0");
}

#[test]
fn search_response_uses_search_request_even_if_loading_more_is_true() {
    let mut page = HistoryPage::new();
    page.records = vec![
        sample_record("01HXYZABCDEF0123456789AAA2", 4),
        sample_record("01HXYZABCDEF0123456789AAA1", 3),
    ];
    page.loading_more = true;
    page.start_search();
    page.on_key(press('a'));

    page.apply_event(
        &history_event(vec![sample_record("01HXYZABCDEF0123456789AAA0", 2)]),
        true,
    );

    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].id, "01HXYZABCDEF0123456789AAA0");
}

#[test]
fn history_responses_follow_pending_request_order() {
    let mut page = HistoryPage::new();
    page.records = vec![sample_record("01HXYZABCDEF0123456789AAA2", 4)];

    page.load_more_outcome();
    page.start_search();
    page.on_key(press('a'));
    page.apply_event(
        &history_event(vec![sample_record("01HXYZABCDEF0123456789AAA1", 3)]),
        true,
    );
    page.apply_event(
        &history_event(vec![sample_record("01HXYZABCDEF0123456789AAA0", 2)]),
        true,
    );

    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].id, "01HXYZABCDEF0123456789AAA0");
}

#[test]
fn search_change_discards_stale_load_more_response() {
    let mut page = HistoryPage::new();
    page.records = vec![sample_record("01HXYZABCDEF0123456789AAA2", 4)];

    page.load_more_outcome();
    page.start_search();
    page.on_key(press('a'));
    page.apply_event(
        &history_event(vec![sample_record("01HXYZABCDEF0123456789AAA1", 3)]),
        true,
    );

    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].id, "01HXYZABCDEF0123456789AAA2");
}

#[test]
fn load_more_waits_while_refresh_is_in_flight() {
    let mut page = HistoryPage::new();
    page.records = vec![sample_record("01HXYZABCDEF0123456789AAA2", 4)];
    page.apply_event(&Event::HistoryChanged, true);
    assert_eq!(page.refresh_commands().len(), 3);

    let outcome = page.load_more_outcome();

    assert!(outcome.command.is_none());
}

#[test]
fn search_change_discards_stale_refresh_page() {
    let mut page = HistoryPage::new();
    page.records = vec![sample_record("01HXYZABCDEF0123456789AAA2", 4)];
    page.apply_event(&Event::HistoryChanged, true);
    assert_eq!(page.refresh_commands().len(), 3);

    page.start_search();
    page.on_key(press('a'));
    page.apply_event(
        &history_event(vec![sample_record("01HXYZABCDEF0123456789AAA1", 3)]),
        true,
    );

    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].id, "01HXYZABCDEF0123456789AAA2");
    assert!(page.refresh_in_flight);
}

#[test]
fn rapid_search_discards_stale_search_response() {
    let mut page = HistoryPage::new();
    page.start_search();

    page.on_key(press('a'));
    page.on_key(press('b'));
    page.apply_event(
        &history_event(vec![sample_record("01HXYZABCDEF0123456789AAA1", 3)]),
        true,
    );
    page.apply_event(
        &history_event(vec![sample_record("01HXYZABCDEF0123456789AAA2", 4)]),
        true,
    );

    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].id, "01HXYZABCDEF0123456789AAA2");
}

#[test]
fn refresh_errors_unblock_history_changed_refresh() {
    let mut page = HistoryPage::new();
    page.enter_commands();
    page.apply_event(&Event::HistoryChanged, true);

    page.apply_event(&error("history_read"), true);
    page.apply_event(&error("history_stats"), true);
    page.apply_event(&error("history_analytics"), true);

    assert!(!page.refresh_in_flight);
    assert_eq!(page.refresh_commands().len(), 3);
}

#[test]
fn filtered_trusts_server_results_without_local_refilter() {
    // The daemon already filters by `query`; the loaded page IS the match set.
    // filtered() must not re-filter locally, or an in-flight query would hide
    // server-returned rows that don't substring-match the raw search buffer.
    let mut page = HistoryPage::new();
    page.records = vec![sample_record("aaa", 1), sample_record("bbb", 2)];
    page.search = "no-such-substring".to_string();

    assert_eq!(page.filtered().len(), 2);
}

#[test]
fn m_key_requests_load_more() {
    let mut page = HistoryPage::new();

    let outcome = page.on_key(press('m'));

    assert!(matches!(
        outcome.command,
        Some(crate::ipc::protocol::Command::GetHistory { .. })
    ));
    assert!(page.loading_more);
}

#[test]
fn list_click_selects_row_under_cursor() {
    let mut page = HistoryPage::new();
    page.records = (0..5)
        .map(|i| sample_record(&format!("id{i}"), (i + 1) as u8))
        .collect();
    *page.list_hit.borrow_mut() = Some(super::ListHit {
        x: 1,
        y: 3,
        width: 40,
        height: 5,
        first: 0,
    });

    page.on_mouse(10, 5, crate::tui::page::MouseKind::Down);

    // row 5 - list top 3 = visible offset 2, first index 0 -> record 2.
    assert_eq!(page.selected, 2);
}

#[test]
fn list_click_outside_rows_keeps_selection() {
    let mut page = HistoryPage::new();
    page.records = (0..3)
        .map(|i| sample_record(&format!("id{i}"), (i + 1) as u8))
        .collect();
    page.selected = 1;
    *page.list_hit.borrow_mut() = Some(super::ListHit {
        x: 1,
        y: 3,
        width: 40,
        height: 3,
        first: 0,
    });

    // Click below the last rendered row must not change selection.
    page.on_mouse(10, 20, crate::tui::page::MouseKind::Down);

    assert_eq!(page.selected, 1);
}

#[test]
fn detail_scroll_clamps_and_resets_on_selection_change() {
    let mut page = HistoryPage::new();
    page.records = (0..3)
        .map(|i| sample_record(&format!("id{i}"), (i + 1) as u8))
        .collect();
    page.detail_max_scroll.set(10);

    page.scroll_detail(4);
    assert_eq!(page.detail_scroll, 4);
    page.scroll_detail(100);
    assert_eq!(page.detail_scroll, 10, "clamped to content height");
    page.scroll_detail(-100);
    assert_eq!(page.detail_scroll, 0);

    // Scrolled into a long detail, then moving selection resets the offset.
    page.detail_scroll = 6;
    page.move_down();
    assert_eq!(page.detail_scroll, 0, "new record resets detail scroll");

    // Switching the detail view also resets it.
    page.detail_scroll = 5;
    page.next_detail();
    assert_eq!(page.detail_scroll, 0, "new detail view resets scroll");
}

#[test]
fn scroll_hint_shown_only_when_detail_overflows() {
    use crate::tui::page::Page;
    let mut page = HistoryPage::new();
    page.records = vec![sample_record("id0", 1)];

    page.detail_max_scroll.set(0);
    assert!(
        !page.key_hints().iter().any(|h| h.keys == "PgUp/PgDn"),
        "no scroll hint when the detail fits"
    );

    page.detail_max_scroll.set(5);
    assert!(
        page.key_hints().iter().any(|h| h.keys == "PgUp/PgDn"),
        "scroll hint appears once the detail overflows"
    );
}

#[test]
fn wheel_over_detail_pane_scrolls_detail_not_list() {
    let mut page = HistoryPage::new();
    page.records = (0..5)
        .map(|i| sample_record(&format!("id{i}"), (i + 1) as u8))
        .collect();
    page.selected = 2;
    page.detail_max_scroll.set(10);
    *page.detail_hit.borrow_mut() = Some(ratatui::layout::Rect::new(40, 3, 40, 10));

    // Wheel inside the detail rect scrolls the detail, selection unchanged.
    page.on_mouse(50, 5, crate::tui::page::MouseKind::ScrollDown);
    assert_eq!(page.selected, 2, "selection must not move");
    assert_eq!(page.detail_scroll, 1, "detail scrolled instead");

    // Wheel outside the detail rect moves the selection as before.
    page.on_mouse(5, 5, crate::tui::page::MouseKind::ScrollDown);
    assert_eq!(page.selected, 3);
}

#[test]
fn detail_tab_click_switches_detail_view() {
    let mut page = HistoryPage::new();
    page.records = vec![sample_record("id0", 1)];
    assert_eq!(page.detail, super::HistoryDetail::Details);

    // The renderer registers one hit rect per detail sub-tab.
    *page.detail_tabs.borrow_mut() = vec![
        (
            ratatui::layout::Rect::new(0, 0, 8, 1),
            super::HistoryDetail::Details,
        ),
        (
            ratatui::layout::Rect::new(8, 0, 6, 1),
            super::HistoryDetail::Asr,
        ),
    ];

    // Clicking the second tab switches the detail view (not the record list).
    page.on_mouse(9, 0, crate::tui::page::MouseKind::Down);
    assert_eq!(page.detail, super::HistoryDetail::Asr);
}

#[test]
fn scroll_moves_selection() {
    let mut page = HistoryPage::new();
    page.records = (0..5)
        .map(|i| sample_record(&format!("id{i}"), (i + 1) as u8))
        .collect();
    page.selected = 0;

    page.on_mouse(0, 0, crate::tui::page::MouseKind::ScrollDown);
    assert_eq!(page.selected, 1);
    page.on_mouse(0, 0, crate::tui::page::MouseKind::ScrollUp);
    assert_eq!(page.selected, 0);
}
