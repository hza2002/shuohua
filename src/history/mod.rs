pub mod assets;
pub mod model;
pub mod stats;
pub mod store;
pub mod watcher;

pub use assets::{AudioAssetInfo, AudioAssetState};
pub use model::{
    AsrHistory, AsrSessionHistory, AudioDeleteResult, DeleteResult, HistoryError, HistoryQuery,
    HistoryRecord, HistoryStatus, PipelineStepHistory, PipelineStepStatus,
    DEFAULT_HISTORY_PAGE_LIMIT, MAX_HISTORY_PAGE_LIMIT,
};
pub use stats::{
    AggregateStats, AnalyticsPeriod, AnalyticsPoint, AnalyticsQuery, AnalyticsSnapshot,
    HistoryEvent, HistoryService, HistoryStatsSnapshot, HistoryStatsStatus,
};
pub use watcher::HistoryWatcher;

#[cfg(test)]
pub mod tests {
    mod reconcile {
        use std::fs;
        use std::path::Path;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        use std::time::{Duration, Instant};

        use time::macros::{datetime, offset};

        use crate::history::{
            store::path_for_month_in_dir, AnalyticsPeriod, AnalyticsQuery, HistoryEvent,
            HistoryQuery, HistoryService, HistoryStatsStatus,
        };

        use super::super::stats::tests_support::{record, TestHooks};

        #[test]
        fn first_request_after_external_line_delete_rebuilds_that_month() {
            let dir = temp_dir("reconcile-line-delete");
            let june = dir.join("2026-06.jsonl");
            write_records(
                &june,
                &[
                    record("a", datetime!(2026-06-01 00:00:00 UTC), "one"),
                    record("b", datetime!(2026-06-01 01:00:00 UTC), "two"),
                ],
            );
            let service =
                HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());
            assert_eq!(service.stats().total.records, 2);
            let mut rx = service.subscribe();

            write_records(
                &june,
                &[record("b", datetime!(2026-06-01 01:00:00 UTC), "two")],
            );

            let snapshot = service.stats();

            assert_eq!(snapshot.status, HistoryStatsStatus::Ready);
            assert_eq!(snapshot.total.records, 1);
            assert_eq!(recv_event(&mut rx), HistoryEvent::Changed);
            assert_no_event(&mut rx);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn external_month_delete_removes_its_totals() {
            let dir = temp_dir("reconcile-month-delete");
            write_line(
                &dir,
                record("may", datetime!(2026-05-01 00:00:00 UTC), "may"),
            );
            write_line(
                &dir,
                record("jun", datetime!(2026-06-01 00:00:00 UTC), "jun"),
            );
            let service =
                HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());
            assert_eq!(service.stats().total.records, 2);
            let mut rx = service.subscribe();

            fs::remove_file(dir.join("2026-06.jsonl")).unwrap();

            let year = service
                .analytics(AnalyticsQuery::new(AnalyticsPeriod::Year, "2026"))
                .unwrap();

            assert_eq!(year.status, HistoryStatsStatus::Ready);
            assert_eq!(year.points[4].stats.records, 1);
            assert_eq!(year.points[5].stats.records, 0);
            assert_eq!(recv_event(&mut rx), HistoryEvent::Changed);
            assert_no_event(&mut rx);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn atomic_month_replace_is_detected() {
            let dir = temp_dir("reconcile-atomic-replace");
            let june = dir.join("2026-06.jsonl");
            write_records(
                &june,
                &[record("old", datetime!(2026-06-01 00:00:00 UTC), "old")],
            );
            let service =
                HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());
            assert_eq!(service.stats().total.records, 1);
            let mut rx = service.subscribe();

            let tmp = dir.join("2026-06.tmp");
            write_records(
                &tmp,
                &[record("new", datetime!(2026-06-01 01:00:00 UTC), "new")],
            );
            fs::rename(&tmp, &june).unwrap();

            let records = service.page(HistoryQuery::default()).unwrap();

            assert_ids(&records, &["new"]);
            assert_eq!(recv_event(&mut rx), HistoryEvent::Changed);
            assert_no_event(&mut rx);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn missed_watcher_event_is_found_by_request_fingerprint_check() {
            let dir = temp_dir("reconcile-missed-event");
            write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
            let service =
                HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());
            assert_eq!(service.stats().total.records, 1);
            let mut rx = service.subscribe();

            write_line(&dir, record("b", datetime!(2026-06-01 01:00:00 UTC), "two"));

            let snapshot = service.stats();

            assert_eq!(snapshot.status, HistoryStatsStatus::Ready);
            assert_eq!(snapshot.total.records, 2);
            assert_eq!(recv_event(&mut rx), HistoryEvent::Changed);
            assert_no_event(&mut rx);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn corrupt_edit_keeps_last_valid_stats_and_marks_stale() {
            let dir = temp_dir("reconcile-stale-last-valid");
            let path = dir.join("2026-06.jsonl");
            write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
            let service =
                HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());
            assert_eq!(service.stats().total.records, 1);
            let mut rx = service.subscribe();

            fs::write(&path, "not json\n").unwrap();

            let snapshot = service.stats();

            assert_eq!(snapshot.status, HistoryStatsStatus::Stale);
            assert_eq!(snapshot.total.records, 1);
            assert!(snapshot.error.is_some());
            assert_eq!(recv_event(&mut rx), HistoryEvent::Changed);
            assert_no_event(&mut rx);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn unchanged_failed_fingerprint_is_not_rescanned_or_republished() {
            let dir = temp_dir("reconcile-stale-no-repeat");
            let path = dir.join("2026-06.jsonl");
            write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
            let scan_calls = Arc::new(AtomicUsize::new(0));
            let hooks = TestHooks::default().with_before_scan_attempt({
                let scan_calls = Arc::clone(&scan_calls);
                move || {
                    scan_calls.fetch_add(1, Ordering::SeqCst);
                }
            });
            let service = HistoryService::with_test_hooks(dir.clone(), offset!(+0), hooks);
            assert_eq!(service.stats().status, HistoryStatsStatus::Ready);
            let mut rx = service.subscribe();

            fs::write(&path, "not json\n").unwrap();

            assert_eq!(service.stats().status, HistoryStatsStatus::Stale);
            assert_eq!(recv_event(&mut rx), HistoryEvent::Changed);
            assert_eq!(service.stats().status, HistoryStatsStatus::Stale);

            assert_eq!(scan_calls.load(Ordering::SeqCst), 3);
            assert_no_event(&mut rx);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn changed_failed_fingerprint_retries() {
            let dir = temp_dir("reconcile-stale-retry");
            let path = dir.join("2026-06.jsonl");
            write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
            let scan_calls = Arc::new(AtomicUsize::new(0));
            let hooks = TestHooks::default().with_before_scan_attempt({
                let scan_calls = Arc::clone(&scan_calls);
                move || {
                    scan_calls.fetch_add(1, Ordering::SeqCst);
                }
            });
            let service = HistoryService::with_test_hooks(dir.clone(), offset!(+0), hooks);
            assert_eq!(service.stats().status, HistoryStatsStatus::Ready);
            let mut rx = service.subscribe();
            fs::write(&path, "not json\n").unwrap();
            assert_eq!(service.stats().status, HistoryStatsStatus::Stale);
            assert_eq!(recv_event(&mut rx), HistoryEvent::Changed);

            write_records(
                &path,
                &[
                    record("a", datetime!(2026-06-01 00:00:00 UTC), "one"),
                    record("b", datetime!(2026-06-01 01:00:00 UTC), "two"),
                ],
            );

            let snapshot = service.stats();

            assert_eq!(snapshot.status, HistoryStatsStatus::Ready);
            assert_eq!(snapshot.total.records, 2);
            assert_eq!(recv_event(&mut rx), HistoryEvent::Changed);
            assert_eq!(scan_calls.load(Ordering::SeqCst), 4);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn history_events_are_published_after_unlock() {
            let dir = temp_dir("reconcile-event-after-unlock");
            write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
            let service =
                HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());
            assert_eq!(service.stats().total.records, 1);
            let mut rx = service.subscribe();

            write_line(&dir, record("b", datetime!(2026-06-01 01:00:00 UTC), "two"));
            let snapshot = service.stats();

            assert_eq!(snapshot.total.records, 2);
            assert_eq!(recv_event(&mut rx), HistoryEvent::Changed);
            assert_eq!(service.stats().status, HistoryStatsStatus::Ready);
            let _ = fs::remove_dir_all(dir);
        }

        fn temp_dir(name: &str) -> std::path::PathBuf {
            std::env::temp_dir().join(format!("shuohua-history-{name}-{}", ulid::Ulid::new()))
        }

        fn write_line(dir: &std::path::Path, record: crate::history::HistoryRecord) {
            let path = path_for_month_in_dir(dir, record.started_at);
            crate::history::store::append_record(&path, &record).unwrap();
        }

        fn write_records(path: &Path, records: &[crate::history::HistoryRecord]) {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            let mut body = Vec::new();
            for record in records {
                serde_json::to_writer(&mut body, record).unwrap();
                body.push(b'\n');
            }
            fs::write(path, body).unwrap();
        }

        fn recv_event(rx: &mut tokio::sync::broadcast::Receiver<HistoryEvent>) -> HistoryEvent {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match rx.try_recv() {
                    Ok(event) => return event,
                    Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                        assert!(
                            Instant::now() < deadline,
                            "timed out waiting for history event"
                        );
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("history event receive failed: {error}"),
                }
            }
        }

        fn assert_no_event(rx: &mut tokio::sync::broadcast::Receiver<HistoryEvent>) {
            std::thread::sleep(Duration::from_millis(250));
            assert_eq!(
                rx.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            );
        }

        fn assert_ids(records: &[crate::history::HistoryRecord], expected: &[&str]) {
            let ids: Vec<_> = records.iter().map(|record| record.id.as_str()).collect();
            assert_eq!(ids, expected);
        }
    }

    mod lazy {
        use std::fs;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        use time::macros::{datetime, offset};

        use crate::history::{
            store::path_for_month_in_dir, AnalyticsPeriod, AnalyticsQuery, HistoryEvent,
            HistoryService, HistoryStatsStatus,
        };

        use super::super::stats::tests_support::{record, TestHooks};

        #[test]
        fn new_service_does_not_read_history_files() {
            let dir = temp_dir("lazy-new");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("2026-06.jsonl"), "").unwrap();
            let list_calls = Arc::new(AtomicUsize::new(0));
            let hooks = TestHooks::default().with_before_list({
                let list_calls = Arc::clone(&list_calls);
                move || {
                    list_calls.fetch_add(1, Ordering::SeqCst);
                }
            });

            let _service = HistoryService::with_test_hooks(dir.clone(), offset!(+8), hooks);

            assert_eq!(list_calls.load(Ordering::SeqCst), 0);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn first_stats_request_builds_the_index_once() {
            let dir = temp_dir("lazy-first-stats");
            write_line(
                &dir,
                record("a", datetime!(2026-06-01 00:00:00 UTC), "hello"),
            );
            let scan_calls = Arc::new(AtomicUsize::new(0));
            let hooks = TestHooks::default().with_before_scan_attempt({
                let scan_calls = Arc::clone(&scan_calls);
                move || {
                    scan_calls.fetch_add(1, Ordering::SeqCst);
                }
            });
            let service = HistoryService::with_test_hooks(dir.clone(), offset!(+8), hooks);

            let first = service.stats();
            let second = service.stats();

            assert_eq!(first.status, HistoryStatsStatus::Ready);
            assert_eq!(second.status, HistoryStatsStatus::Ready);
            assert_eq!(scan_calls.load(Ordering::SeqCst), 1);
            assert_eq!(first.total.records, 1);
            assert_eq!(second.total.records, 1);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn local_offset_change_rebuilds_the_index() {
            let dir = temp_dir("lazy-offset");
            write_line(
                &dir,
                record("a", datetime!(2026-06-30 23:30:00 UTC), "hello"),
            );
            let service =
                HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());

            let utc = service
                .analytics(AnalyticsQuery::new(AnalyticsPeriod::Day, "2026-06-30"))
                .unwrap();
            service.set_test_local_offset(offset!(+8));
            let local = service
                .analytics(AnalyticsQuery::new(AnalyticsPeriod::Day, "2026-07-01"))
                .unwrap();

            assert_eq!(
                utc.points
                    .iter()
                    .map(|point| point.stats.records)
                    .sum::<u64>(),
                1
            );
            assert_eq!(
                local
                    .points
                    .iter()
                    .map(|point| point.stats.records)
                    .sum::<u64>(),
                1
            );
            let old_day_after_offset_change = service
                .analytics(AnalyticsQuery::new(AnalyticsPeriod::Day, "2026-06-30"))
                .unwrap();
            assert_eq!(
                old_day_after_offset_change
                    .points
                    .iter()
                    .map(|point| point.stats.records)
                    .sum::<u64>(),
                0
            );
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn append_reconciles_a_missed_external_edit_before_updating_totals() {
            let dir = temp_dir("lazy-append-reconcile");
            let service =
                HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());
            write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
            assert_eq!(service.stats().total.records, 1);
            let mut rx = service.subscribe();

            write_line(&dir, record("b", datetime!(2026-06-01 01:00:00 UTC), "two"));
            service
                .append(record("c", datetime!(2026-06-01 02:00:00 UTC), "three"))
                .unwrap();

            let snapshot = service.stats();
            assert_eq!(snapshot.status, HistoryStatsStatus::Ready);
            assert_eq!(snapshot.total.records, 3);
            assert_eq!(rx.try_recv().unwrap(), HistoryEvent::Changed);
            assert_eq!(rx.try_recv().unwrap(), HistoryEvent::Appended);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn append_in_uninitialized_state_writes_without_scanning() {
            let dir = temp_dir("lazy-append-uninitialized");
            let scan_calls = Arc::new(AtomicUsize::new(0));
            let hooks = TestHooks::default().with_before_scan_attempt({
                let scan_calls = Arc::clone(&scan_calls);
                move || {
                    scan_calls.fetch_add(1, Ordering::SeqCst);
                }
            });
            let service = HistoryService::with_test_hooks(dir.clone(), offset!(+0), hooks);

            service
                .append(record("a", datetime!(2026-06-01 00:00:00 UTC), "one"))
                .unwrap();

            assert_eq!(scan_calls.load(Ordering::SeqCst), 0);
            let path = path_for_month_in_dir(&dir, datetime!(2026-06-01 00:00:00 UTC));
            assert_eq!(fs::read_to_string(path).unwrap().lines().count(), 1);
            let _ = fs::remove_dir_all(dir);
        }

        fn temp_dir(name: &str) -> std::path::PathBuf {
            std::env::temp_dir().join(format!("shuohua-history-{name}-{}", ulid::Ulid::new()))
        }

        fn write_line(dir: &std::path::Path, record: crate::history::HistoryRecord) {
            let path = path_for_month_in_dir(dir, record.started_at);
            crate::history::store::append_record(&path, &record).unwrap();
        }
    }
}
