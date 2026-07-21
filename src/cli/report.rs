use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use anyhow::{Context, Result};
use clap::Args;
use flate2::write::GzEncoder;
use flate2::Compression;
use time::{Date, Duration, OffsetDateTime, UtcOffset};

const LAUNCHD_LOG_TAIL_BYTES: u64 = 64 * 1024;

#[derive(Debug, Args)]
pub struct ReportArgs {
    /// Align the log window to this recording id without collecting history.
    #[arg(long)]
    pub recording: Option<String>,

    /// Output directory or final .tar.gz path. Defaults to the current directory.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

pub fn run(args: ReportArgs) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let dates = window_dates(now, args.recording.as_deref())?;
    let target = output_path(args.out.as_deref(), now)?;
    if target.exists() {
        anyhow::bail!("report output already exists: {}", target.display());
    }
    let staging =
        std::env::temp_dir().join(format!("shuo-report-staging-{}", ulid::Ulid::generate()));
    let bundle_name = target
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("shuo-report")
        .trim_end_matches(".tar")
        .to_string();
    let root = staging.join(&bundle_name);
    fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;

    let result = build_report_tree(&root, now, &dates)
        .and_then(|summary| {
            fs::write(root.join("summary.txt"), summary)
                .with_context(|| format!("write {}", root.join("summary.txt").display()))?;
            write_archive(&root, &target)
        })
        .map(|()| {
            println!(
                "{}",
                crate::i18n::tr(
                    "cli.report.written",
                    &[("path", absolute_path(&target).display().to_string())]
                )
            );
        });

    let _ = fs::remove_dir_all(&staging);
    result
}

fn build_report_tree(root: &Path, now: OffsetDateTime, dates: &[Date]) -> Result<String> {
    let mut collected = Vec::new();
    let mut missing = Vec::new();

    let doctor = capture_doctor();
    fs::write(root.join("doctor.txt"), &doctor.text)
        .with_context(|| format!("write {}", root.join("doctor.txt").display()))?;
    collected.push("doctor.txt".to_string());

    collect_logs(root, dates, &mut collected, &mut missing)?;
    collect_launchd(root, &mut collected, &mut missing)?;

    Ok(build_summary(&SummaryInput {
        version: env!("CARGO_PKG_VERSION"),
        arch: std::env::consts::ARCH,
        generated_at: &now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| now.to_string()),
        window_dates: dates,
        collected: &collected,
        missing: &missing,
        doctor_status: &doctor.status,
    }))
}

fn collect_logs(
    root: &Path,
    dates: &[Date],
    collected: &mut Vec<String>,
    missing: &mut Vec<String>,
) -> Result<()> {
    let logs_dir = crate::paths::StateDirs::discover().logs();
    let entries = read_dir_paths(&logs_dir).unwrap_or_default();
    let selected = select_log_files(&entries, dates);
    let out_dir = root.join("logs");
    if !selected.files.is_empty() {
        fs::create_dir_all(&out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    }
    for path in selected.files {
        let Some(name) = path.file_name() else {
            continue;
        };
        let rel = PathBuf::from("logs").join(name);
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        fs::write(root.join(&rel), redact_report_log(&text))
            .with_context(|| format!("write {}", rel.display()))?;
        collected.push(rel.display().to_string());
    }
    for date in selected.missing_dates {
        missing.push(format!("logs/{}", log_file_name(date)));
    }
    Ok(())
}

fn collect_launchd(
    root: &Path,
    collected: &mut Vec<String>,
    missing: &mut Vec<String>,
) -> Result<()> {
    let out_dir = root.join("launchd");
    let mut wrote_dir = false;
    match crate::cli::service::launchd_status() {
        crate::cli::service::LaunchdStatus::Installed(path) => {
            fs::create_dir_all(&out_dir)
                .with_context(|| format!("create {}", out_dir.display()))?;
            wrote_dir = true;
            let rel = PathBuf::from("launchd").join("service.summary.txt");
            fs::write(
                root.join(&rel),
                launchd_summary(crate::cli::service::plist_program().as_deref(), &path),
            )
            .with_context(|| format!("write {}", rel.display()))?;
            collected.push(rel.display().to_string());
        }
        crate::cli::service::LaunchdStatus::NotInstalled(path) => {
            missing.push(format!("launchd/service.plist ({})", path.display()));
        }
        #[cfg(not(target_os = "macos"))]
        crate::cli::service::LaunchdStatus::Unsupported => {
            missing.push("launchd/service.plist (unsupported platform)".to_string());
        }
    }

    let state_root = crate::paths::StateDirs::discover().root().to_path_buf();
    for name in ["launchd.stdout.log", "launchd.stderr.log"] {
        let path = state_root.join(name);
        if path.exists() {
            if !wrote_dir {
                fs::create_dir_all(&out_dir)
                    .with_context(|| format!("create {}", out_dir.display()))?;
                wrote_dir = true;
            }
            let rel = PathBuf::from("launchd").join(name);
            fs::write(
                root.join(&rel),
                redact_report_log(&read_tail(&path, LAUNCHD_LOG_TAIL_BYTES)?),
            )
            .with_context(|| format!("write {}", rel.display()))?;
            collected.push(rel.display().to_string());
        } else {
            missing.push(format!("launchd/{name}"));
        }
    }
    Ok(())
}

fn launchd_summary(program: Option<&Path>, plist: &Path) -> String {
    let mut out = String::new();
    out.push_str("launchd service summary\n");
    out.push_str(&format!("plist: {}\n", plist.display()));
    match program {
        Some(program) => out.push_str(&format!("program: {}\n", program.display())),
        None => out.push_str("program: unavailable\n"),
    }
    out.push_str("note: raw plist is not included\n");
    out
}

fn redact_report_log(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        if line_mentions_retained_audio(line) {
            out.push_str("[redacted retained-audio log line]\n");
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn line_mentions_retained_audio(line: &str) -> bool {
    line.contains("retained audio") || line.contains("/audio/")
}

fn capture_doctor() -> DoctorCapture {
    let output = ProcessCommand::new(std::env::current_exe().unwrap_or_else(|_| "shuo".into()))
        .arg("doctor")
        .output();
    match output {
        Ok(output) => {
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            if !output.stderr.is_empty() {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            DoctorCapture {
                text,
                status: output.status.to_string(),
            }
        }
        Err(error) => DoctorCapture {
            text: format!("failed to run shuo doctor: {error:#}\n"),
            status: "spawn failed".to_string(),
        },
    }
}

struct DoctorCapture {
    text: String,
    status: String,
}

fn write_archive(root: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .with_context(|| format!("create {}", target.display()))?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("shuo-report");
    builder
        .append_dir_all(name, root)
        .with_context(|| format!("append {}", root.display()))?;
    let encoder = builder.into_inner().context("finish report tar")?;
    encoder.finish().context("finish report gzip")?;
    Ok(())
}

fn output_path(out: Option<&Path>, now: OffsetDateTime) -> Result<PathBuf> {
    let name = report_file_name(now);
    let path = match out {
        Some(path) if path.exists() && path.is_dir() => path.join(name),
        Some(path) if !path.exists() && !is_tar_gz_path(path) => path.join(name),
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir()
            .context("resolve current directory")?
            .join(name),
    };
    Ok(path)
}

fn is_tar_gz_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".tar.gz"))
}

fn report_file_name(now: OffsetDateTime) -> String {
    let stamp = now
        .format(&time::macros::format_description!(
            "[year][month][day]-[hour][minute][second]Z"
        ))
        .unwrap_or_else(|_| "unknown-time".to_string());
    format!("shuo-report-{stamp}.tar.gz")
}

fn read_dir_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        paths.push(
            entry
                .with_context(|| format!("read entry under {}", dir.display()))?
                .path(),
        );
    }
    Ok(paths)
}

fn read_tail(path: &Path, max_bytes: u64) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let len = file
        .metadata()
        .with_context(|| format!("stat {}", path.display()))?
        .len();
    file.seek(SeekFrom::Start(len.saturating_sub(max_bytes)))
        .with_context(|| format!("seek {}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("read {}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn window_dates(now: OffsetDateTime, recording: Option<&str>) -> Result<Vec<Date>> {
    let local_offset = local_offset();
    let anchor = match recording {
        Some(id) => {
            let ulid = ulid::Ulid::from_string(id)
                .map_err(|_| anyhow::anyhow!("invalid recording id: {id}"))?;
            system_time_to_offset_datetime(ulid.datetime())?.to_offset(local_offset)
        }
        None => now.to_offset(local_offset),
    };
    let date = anchor.date();
    let dates = if recording.is_some() {
        vec![date - Duration::days(1), date, date + Duration::days(1)]
    } else {
        vec![date - Duration::days(2), date - Duration::days(1), date]
    };
    Ok(dates)
}

fn local_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

fn system_time_to_offset_datetime(value: std::time::SystemTime) -> Result<OffsetDateTime> {
    let duration = value
        .duration_since(std::time::UNIX_EPOCH)
        .context("ulid timestamp before unix epoch")?;
    let seconds = i64::try_from(duration.as_secs()).context("ulid timestamp too large")?;
    OffsetDateTime::from_unix_timestamp(seconds)
        .context("convert ulid timestamp")?
        .replace_nanosecond(duration.subsec_nanos())
        .context("convert ulid timestamp nanos")
}

struct SelectedLogs {
    files: Vec<PathBuf>,
    missing_dates: Vec<Date>,
}

fn select_log_files(entries: &[PathBuf], window: &[Date]) -> SelectedLogs {
    let mut files = Vec::new();
    let mut missing_dates = Vec::new();
    for date in window {
        let name = log_file_name(*date);
        match entries
            .iter()
            .find(|path| path.file_name().and_then(|n| n.to_str()) == Some(name.as_str()))
        {
            Some(path) => files.push(path.clone()),
            None => missing_dates.push(*date),
        }
    }
    SelectedLogs {
        files,
        missing_dates,
    }
}

fn log_file_name(date: Date) -> String {
    format!(
        "shuo-{:04}-{:02}-{:02}.log",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

struct SummaryInput<'a> {
    version: &'a str,
    arch: &'a str,
    generated_at: &'a str,
    window_dates: &'a [Date],
    collected: &'a [String],
    missing: &'a [String],
    doctor_status: &'a str,
}

fn build_summary(input: &SummaryInput<'_>) -> String {
    let mut out = String::new();
    out.push_str("shuo report\n");
    out.push_str(&format!("version: {}\n", input.version));
    out.push_str(&format!("arch: {}\n", input.arch));
    out.push_str(&format!("generated_at: {}\n", input.generated_at));
    out.push_str(&format!(
        "window_dates: {}\n",
        input
            .window_dates
            .iter()
            .map(|date| date.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out.push_str(&format!("doctor status: {}\n", input.doctor_status));
    out.push_str("\nprivacy:\n");
    out.push_str("- config file contents are not included\n");
    out.push_str("- history is not included\n");
    out.push_str("- retained audio is not included\n");
    out.push_str("- launchd files may include local filesystem paths\n");
    out.push_str("\ncollected:\n");
    for item in input.collected {
        out.push_str(&format!("- {item}\n"));
    }
    out.push_str("\nmissing:\n");
    if input.missing.is_empty() {
        out.push_str("- none\n");
    } else {
        for item in dedupe(input.missing) {
            out.push_str(&format!("- {item}\n"));
        }
    }
    out
}

fn dedupe(items: &[String]) -> Vec<&str> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for item in items {
        if seen.insert(item.as_str()) {
            out.push(item.as_str());
        }
    }
    out
}

#[cfg(test)]
fn date(year: i32, month: u8, day: u8) -> Date {
    Date::from_calendar_date(year, time::Month::try_from(month).unwrap(), day).unwrap()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use time::macros::datetime;

    #[test]
    fn default_window_uses_today_and_previous_two_local_dates() {
        let dates = super::window_dates(datetime!(2026-06-30 12:00:00 UTC), None).unwrap();

        assert_eq!(
            dates,
            [
                super::date(2026, 6, 28),
                super::date(2026, 6, 29),
                super::date(2026, 6, 30)
            ]
        );
    }

    #[test]
    fn recording_window_uses_previous_current_and_next_local_dates() {
        let ulid = ulid::Ulid::from_datetime(std::time::UNIX_EPOCH).to_string();
        let dates = super::window_dates(datetime!(2026-06-30 12:00:00 UTC), Some(&ulid)).unwrap();

        assert_eq!(
            dates,
            [
                super::date(1969, 12, 31),
                super::date(1970, 1, 1),
                super::date(1970, 1, 2)
            ]
        );
    }

    #[test]
    fn select_log_files_reports_missing_dates() {
        let entries = vec![
            PathBuf::from("/state/logs/shuo-2026-06-28.log"),
            PathBuf::from("/state/logs/shuo-2026-06-30.log"),
            PathBuf::from("/state/logs/other.log"),
        ];

        let selected = super::select_log_files(
            &entries,
            &[
                super::date(2026, 6, 28),
                super::date(2026, 6, 29),
                super::date(2026, 6, 30),
            ],
        );

        assert_eq!(selected.files.len(), 2);
        assert_eq!(selected.missing_dates, [super::date(2026, 6, 29)]);
    }

    #[test]
    fn summary_states_that_config_and_history_are_not_included() {
        let summary = super::build_summary(&super::SummaryInput {
            version: "0.3.0",
            arch: "aarch64",
            generated_at: "2026-06-30T12:00:00Z",
            window_dates: &[super::date(2026, 6, 30)],
            collected: &["doctor.txt".to_string()],
            missing: &["logs/shuo-2026-06-30.log".to_string()],
            doctor_status: "exit status: 1",
        });

        assert!(summary.contains("config file contents are not included"));
        assert!(summary.contains("history is not included"));
        assert!(summary.contains("doctor status: exit status: 1"));
    }

    #[test]
    fn read_tail_tolerates_utf8_boundary_split() {
        let dir = std::env::temp_dir().join(format!("shuohua-report-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("launchd.stderr.log");
        std::fs::write(&path, "prefix 中").unwrap();

        let tail = super::read_tail(&path, 2).unwrap();

        assert!(!tail.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn report_log_text_redacts_retained_audio_lines() {
        let input = "\
2026-06-30 INFO daemon ready
2026-06-30 INFO recording_id=01KABC path=/Users/u/.local/state/shuohua/audio/01KABC.flac retained audio saved
2026-06-30 ERROR something else
";

        let redacted = super::redact_report_log(input);

        assert!(redacted.contains("daemon ready"));
        assert!(redacted.contains("[redacted retained-audio log line]"));
        assert!(redacted.contains("something else"));
        assert!(!redacted.contains(".flac"), "{redacted}");
        assert!(
            !redacted.contains("/Users/u/.local/state/shuohua/audio"),
            "{redacted}"
        );
    }

    #[test]
    fn launchd_summary_does_not_include_raw_plist_environment() {
        let summary = super::launchd_summary(
            Some(std::path::Path::new("/Users/u/.local/bin/shuo")),
            std::path::Path::new("/Users/u/Library/LaunchAgents/com.hza2002.shuohua.plist"),
        );

        assert!(summary.contains("program: /Users/u/.local/bin/shuo"));
        assert!(!summary.contains("EnvironmentVariables"));
    }

    #[test]
    fn output_path_treats_missing_extensionless_path_as_directory() {
        let path = super::output_path(
            Some(std::path::Path::new("/tmp/shuo-reports")),
            datetime!(2026-06-30 12:00:00 UTC),
        )
        .unwrap();

        assert_eq!(
            path,
            std::path::PathBuf::from("/tmp/shuo-reports/shuo-report-20260630-120000Z.tar.gz")
        );
    }

    #[test]
    fn output_path_accepts_missing_tar_gz_path_as_file() {
        let path = super::output_path(
            Some(std::path::Path::new("/tmp/custom.tar.gz")),
            datetime!(2026-06-30 12:00:00 UTC),
        )
        .unwrap();

        assert_eq!(path, std::path::PathBuf::from("/tmp/custom.tar.gz"));
    }
}
