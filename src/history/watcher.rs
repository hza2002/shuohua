use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{RecursiveMode, Watcher};

use crate::history::HistoryService;

const DEBOUNCE: Duration = Duration::from_millis(150);

pub struct HistoryWatcher {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for HistoryWatcher {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn start(service: HistoryService) -> Result<HistoryWatcher> {
    let dir = service.history_dir_for_watcher();
    fs::create_dir_all(&dir).with_context(|| format!("create history dir {}", dir.display()))?;

    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (stop_tx, stop_rx) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("history-watcher".to_string())
        .spawn(move || {
            if let Err(error) = run(service, dir, stop_rx, ready_tx) {
                tracing::error!(error = ?error, "history watcher exited");
            }
        })
        .context("spawn history-watcher thread")?;

    match ready_rx
        .recv()
        .context("history watcher startup channel closed")?
    {
        Ok(()) => Ok(HistoryWatcher {
            stop: Some(stop_tx),
            thread: Some(thread),
        }),
        Err(error) => {
            let _ = stop_tx.send(());
            let _ = thread.join();
            anyhow::bail!("{error}");
        }
    }
}

fn run(
    service: HistoryService,
    dir: PathBuf,
    stop_rx: mpsc::Receiver<()>,
    ready_tx: mpsc::SyncSender<std::result::Result<(), String>>,
) -> Result<()> {
    let (event_tx, event_rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = event_tx.send(res);
    })
    .context("create history watcher")?;
    watcher
        .watch(&dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("watch history dir {}", dir.display()))?;
    let _ = ready_tx.send(Ok(()));

    loop {
        match stop_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let Ok(event) = event_rx.try_recv() else {
            continue;
        };
        let mut paths = match event {
            Ok(event) => event.paths,
            Err(error) => {
                tracing::warn!(error = %error, "history watcher notify error");
                service.mark_history_watcher_error();
                continue;
            }
        };

        loop {
            match event_rx.recv_timeout(DEBOUNCE) {
                Ok(Ok(event)) => paths.extend(event.paths),
                Ok(Err(error)) => {
                    tracing::warn!(error = %error, "history watcher notify error");
                    service.mark_history_watcher_error();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }

        paths.sort();
        paths.dedup();
        service.mark_history_paths_changed(&paths);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::{Duration, Instant};

    use time::macros::{datetime, offset};

    use crate::history::{
        store::path_for_month_in_dir, HistoryEvent, HistoryService, HistoryStatsStatus,
    };

    use super::super::stats::tests_support::{record, TestHooks};

    #[test]
    fn watcher_marks_month_dirty_without_scanning() {
        let dir = temp_dir("watcher-dirty-no-scan");
        write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
        let scan_calls = Arc::new(AtomicUsize::new(0));
        let hooks = TestHooks::default().with_before_scan_attempt({
            let scan_calls = Arc::clone(&scan_calls);
            move || {
                scan_calls.fetch_add(1, Ordering::SeqCst);
            }
        });
        let service = HistoryService::with_test_hooks(dir.clone(), offset!(+0), hooks);
        assert_eq!(service.stats().total.records, 1);
        let before_scans = scan_calls.load(Ordering::SeqCst);
        let _watcher = service.watch().unwrap();

        write_line(&dir, record("b", datetime!(2026-06-01 01:00:00 UTC), "two"));
        wait_for_changed(&service);

        assert_eq!(scan_calls.load(Ordering::SeqCst), before_scans);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn watch_creates_history_directory_without_scanning() {
        let dir = temp_dir("watcher-create-dir");
        let scan_calls = Arc::new(AtomicUsize::new(0));
        let list_calls = Arc::new(AtomicUsize::new(0));
        let hooks = TestHooks::default()
            .with_before_scan_attempt({
                let scan_calls = Arc::clone(&scan_calls);
                move || {
                    scan_calls.fetch_add(1, Ordering::SeqCst);
                }
            })
            .with_before_list({
                let list_calls = Arc::clone(&list_calls);
                move || {
                    list_calls.fetch_add(1, Ordering::SeqCst);
                }
            });
        let service = HistoryService::with_test_hooks(dir.clone(), offset!(+0), hooks);

        let _watcher = service.watch().unwrap();

        assert!(dir.is_dir());
        assert_eq!(scan_calls.load(Ordering::SeqCst), 0);
        assert_eq!(list_calls.load(Ordering::SeqCst), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn daemon_append_watcher_echo_is_a_noop() {
        let dir = temp_dir("watcher-append-echo");
        write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());
        assert_eq!(service.stats().total.records, 1);
        let mut rx = service.subscribe();
        let _watcher = service.watch().unwrap();

        service
            .append(record("b", datetime!(2026-06-01 01:00:00 UTC), "two"))
            .unwrap();

        assert_eq!(recv_event(&mut rx), HistoryEvent::Appended);
        assert_no_event(&mut rx);
        assert_eq!(service.stats().status, HistoryStatsStatus::Ready);
        assert_no_event(&mut rx);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn watcher_error_marks_all_dirty_without_scanning() {
        let dir = temp_dir("watcher-error-dirty");
        write_line(&dir, record("a", datetime!(2026-06-01 00:00:00 UTC), "one"));
        let scan_calls = Arc::new(AtomicUsize::new(0));
        let hooks = TestHooks::default().with_before_scan_attempt({
            let scan_calls = Arc::clone(&scan_calls);
            move || {
                scan_calls.fetch_add(1, Ordering::SeqCst);
            }
        });
        let service = HistoryService::with_test_hooks(dir.clone(), offset!(+0), hooks);
        assert_eq!(service.stats().total.records, 1);
        let before_scans = scan_calls.load(Ordering::SeqCst);
        let mut rx = service.subscribe();

        service.mark_history_watcher_error();

        assert_eq!(service.debug_dirty_month_count(), usize::MAX);
        assert_eq!(scan_calls.load(Ordering::SeqCst), before_scans);
        assert_eq!(recv_event(&mut rx), HistoryEvent::Changed);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn watcher_error_in_uninitialized_state_marks_dirty_without_event() {
        let dir = temp_dir("watcher-error-uninit");
        let service =
            HistoryService::with_test_hooks(dir.clone(), offset!(+0), TestHooks::default());
        let mut rx = service.subscribe();

        service.mark_history_watcher_error();

        assert_eq!(service.debug_dirty_month_count(), usize::MAX);
        assert_no_event(&mut rx);
        let _ = fs::remove_dir_all(dir);
    }

    fn wait_for_changed(service: &HistoryService) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if service.debug_dirty_month_count() > 0 {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for dirty month"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
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

    fn temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("shuohua-history-{name}-{}", ulid::Ulid::new()))
    }

    fn write_line(dir: &std::path::Path, record: crate::history::HistoryRecord) {
        let path = path_for_month_in_dir(dir, record.started_at);
        crate::history::store::append_record(&path, &record).unwrap();
    }
}
