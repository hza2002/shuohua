pub mod model;
pub mod stats;
pub mod store;

pub use model::{
    AsrHistory, AsrSessionHistory, HistoryError, HistoryQuery, HistoryRecord, HistoryStatus,
    PipelineStepHistory, PipelineStepStatus, DEFAULT_HISTORY_PAGE_LIMIT, MAX_HISTORY_PAGE_LIMIT,
};
pub use stats::{
    AggregateStats, AnalyticsPeriod, AnalyticsPoint, AnalyticsQuery, AnalyticsSnapshot,
    HistoryEvent, HistoryService, HistoryStatsSnapshot, HistoryStatsStatus,
};

#[cfg(test)]
pub mod tests {
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
