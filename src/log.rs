//! Daemon logging.
//!
//! CLI commands keep using stdout/stderr for user-facing output. The daemon
//! writes diagnostic logs here, and mirrors the same logs to stderr only when
//! `shuo --daemon` is run from an interactive terminal.

use std::fs::OpenOptions;
use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Context, Result};
use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::prelude::*;

pub struct LogGuard {
    _file_guard: WorkerGuard,
    _stderr_guard: Option<WorkerGuard>,
}

pub fn init_daemon() -> Result<LogGuard> {
    let path = log_file_path(OffsetDateTime::now_utc())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create log dir {}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open log file {}", path.display()))?;
    let (file_writer, file_guard) = tracing_appender::non_blocking(file);

    let timer = timer();
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_timer(timer.clone())
        .with_writer(file_writer)
        .with_filter(daemon_filter());

    let mirror_to_stderr = std::io::stderr().is_terminal();
    let (stderr_layer, stderr_guard) = if mirror_to_stderr {
        let (stderr_writer, guard) = tracing_appender::non_blocking(std::io::stderr());
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(true)
            .with_timer(timer)
            .with_writer(stderr_writer)
            .with_filter(daemon_filter());
        (Some(layer), Some(guard))
    } else {
        (None, None)
    };

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .try_init()
        .context("install tracing subscriber")?;

    Ok(LogGuard {
        _file_guard: file_guard,
        _stderr_guard: stderr_guard,
    })
}

pub fn logs_dir() -> PathBuf {
    crate::state::history::state_dir().join("logs")
}

pub fn log_file_path(now: OffsetDateTime) -> Result<PathBuf> {
    let local = now.to_offset(local_offset());
    let name = format!(
        "shuo-{:04}-{:02}-{:02}.log",
        local.year(),
        u8::from(local.month()),
        local.day()
    );
    Ok(logs_dir().join(name))
}

fn daemon_filter() -> Targets {
    Targets::new()
        .with_target(env!("CARGO_CRATE_NAME"), LevelFilter::DEBUG)
        .with_default(LevelFilter::WARN)
}

fn timer() -> OffsetTime<Vec<time::format_description::FormatItem<'static>>> {
    OffsetTime::new(
        local_offset(),
        format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3][offset_hour sign:mandatory]:[offset_minute]"
        )
        .to_vec(),
    )
}

fn local_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn log_file_path_uses_local_date_prefix() {
        let path = log_file_path(datetime!(2026-06-16 12:34:56 UTC)).unwrap();
        let file_name = path.file_name().unwrap().to_string_lossy();
        assert!(file_name.starts_with("shuo-"));
        assert!(file_name.ends_with(".log"));
    }

    #[test]
    fn logs_dir_lives_under_state_dir() {
        assert!(logs_dir().ends_with("logs"));
    }
}
