# Tracing Logging And History Partition Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace daemon stdout/stderr logging with a production-friendly `tracing` logging system, keep terminal mirroring for foreground daemon debugging, and partition history into local-month JSONL files.

**Architecture:** CLI commands keep using stdout/stderr for user-facing interaction. Daemon code emits structured `tracing` events only; the logger always writes daily local-date log files and mirrors the same events to stderr only when run from an interactive terminal. History records keep the existing JSON schema but move from one `history.jsonl` file to `history/YYYY-MM.jsonl`.

**Tech Stack:** Rust, `tracing`, `tracing-subscriber`, `tracing-appender`, `time`, existing JSONL history store.

---

## Logging Policy

Use this as the migration checklist for every existing or new log event.

- No environment variables for log control.
- No `[log]` config section in this phase.
- No `shuo logs` command.
- Daemon business code must not call `println!`, `eprintln!`, or `debug_println!`.
- CLI commands may continue to use `println!` for user-facing results.
- Launchd stdout/stderr remain only for panic, very early failure, or logger initialization failure.
- File logs are diagnostic logs, not low-value info logs. They should be sparse but useful.
- Use `INFO` only for lifecycle anchors: daemon ready, recording started, recording ended, dev trace enabled.
- Use `DEBUG` for non-sensitive diagnostic details useful in user bug reports.
- Use `WARN` for recoverable failures or degraded behavior.
- Use `ERROR` for failed critical operations.
- Do not emit `TRACE` in this phase; VAD trace sidecar remains separate.
- Official daemon logs must never record ASR text, clipboard text, prompts, hotword details, post input/output text, or provider raw payloads that may contain text.
- `voice/observer` remains a dev-only sidecar and may contain ASR text when `--features dev` plus `voice.vad_trace = true` are enabled.

## File Map

- Modify `Cargo.toml`
  Add `tracing`, `tracing-subscriber`, and `tracing-appender`.

- Replace `src/log.rs`
  Own daemon logger initialization, daily local-date file naming, terminal mirroring, and shared format.

- Modify `src/main.rs`
  Initialize logger at daemon startup, keep very-early failures on stderr, replace daemon `eprintln!` calls, and hide `--daemon` from clap help if acceptable during implementation.

- Modify daemon modules:
  `src/reload.rs`, `src/ipc/server.rs`, `src/post/mod.rs`, `src/voice/finish.rs`, `src/voice/dispatch.rs`, `src/voice/recorder.rs`, `src/voice/observer.rs`, `src/asr/providers/apple.rs`, `src/asr/providers/doubao.rs`, `src/overlay/debug.rs`.
  Convert existing prints to `tracing` events or delete them.

- Modify `src/state/history.rs`
  Move append path to `history/YYYY-MM.jsonl` using local month for filenames while keeping UTC timestamps inside records.

- Modify `src/ipc/server.rs`
  Read history from monthly files, newest files first, no compatibility with old top-level `history.jsonl`.

- Modify `src/cli/service.rs`
  Remove `RUST_LOG` from launchd plist and keep `StandardOutPath` / `StandardErrorPath` only as fallback.

- Modify docs:
  `docs/DESIGN.md`, `docs/CLI.md`, `docs/SCHEMA.md`, `docs/MODULES.md`, `CHANGELOG.md`.

---

### Task 1: Document The Final Logging And History Design

**Files:**
- Modify: `docs/DESIGN.md`
- Modify: `docs/CLI.md`
- Modify: `docs/SCHEMA.md`
- Modify: `docs/MODULES.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Update logging design in `docs/DESIGN.md`**

Replace the old §2.13 stderr/debug_println design with:

```markdown
### 2.13 正式日志系统

shuohua daemon 不把 stdout/stderr 当业务日志通道。daemon 业务代码统一使用
`tracing`，正式日志写入：

`${XDG_STATE_HOME:-~/.local/state}/shuohua/logs/shuo-YYYY-MM-DD.log`

文件名使用本地日期；日志行时间使用本地时间并带 UTC offset。history schema
里的时间戳仍为 UTC RFC3339。

当 `shuo --daemon` 从交互式 terminal 直接运行时，同一份日志同时 mirror 到
stderr，方便开发和 release 包排查。launchd 启动时不 mirror；plist 的
`StandardOutPath` / `StandardErrorPath` 只作为 panic、极早期失败、logger 初始化
失败的兜底。

不提供环境变量或配置项控制日志等级。本阶段不做 `shuo logs` 命令，也不自动清理
日志文件。

日志是诊断日志，不是 session 事实源。单次录音的详细事实仍以 history JSONL 为准。
正式日志只记录少量锚点和异常：

- daemon ready
- recording started: recording_id、provider、app、multi_session
- recording ended: recording_id、status、audio_ms、session_count、pipeline step status
- config reload success/failure
- ASR / recorder / dispatch / history / IPC / hotkey 异常
- dev trace enabled: recording_id、trace path

正式日志不得记录识别正文、clipboard 内容、prompt、hotwords 明细、post 输入输出正文、
可能含正文的 provider 原始响应。`voice/observer` trace sidecar 是 dev-only 诊断文件，
不属于正式 daemon log。
```

- [ ] **Step 2: Update launchd docs in `docs/CLI.md`**

Remove the plist `EnvironmentVariables` block that sets `RUST_LOG`. Update text under `StandardOutPath` / `StandardErrorPath`:

```markdown
**StandardOutPath / StandardErrorPath**：保留为兜底。正式 daemon 日志写入
`~/.local/state/shuohua/logs/shuo-YYYY-MM-DD.log`；launchd stdout/stderr 只用于
panic、极早期失败、logger 初始化失败等正式 logger 尚未接管的情况。
```

- [ ] **Step 3: Update history docs in `docs/SCHEMA.md`**

Change the history location section from one top-level file to monthly files:

```markdown
`${XDG_STATE_HOME:-~/.local/state}/shuohua/history/YYYY-MM.jsonl`

文件名使用本地月份；每条 record 内部 `started_at` / `ended_at` 仍使用 UTC RFC3339。
一次 recording = 一条 JSON 行。record JSON schema 不因分片变化升 version。
```

Remove statements that call `history.jsonl` the only file.

- [ ] **Step 4: Update module map in `docs/MODULES.md`**

Change the `src/log.rs` description to:

```markdown
src/log.rs # tracing 初始化：daily file appender、本地时间格式、TTY mirror
```

Change `state/history.rs` description to mention monthly JSONL files.

- [ ] **Step 5: Add changelog entry**

Add a top entry:

```markdown
## 2026-06-16 - Logging design transition

- Decided to replace daemon stdout/stderr logging with `tracing` file logs.
- Kept terminal mirroring for foreground `shuo --daemon` development runs.
- Defined official logs as sparse diagnostic logs with strict privacy boundaries.
- Planned monthly history JSONL partitioning without legacy migration.
```

- [ ] **Step 6: Commit docs**

Run:

```bash
git status --short --branch -uall
git add docs/DESIGN.md docs/CLI.md docs/SCHEMA.md docs/MODULES.md CHANGELOG.md
git commit -m "Document daemon logging design"
```

Expected: one docs-only commit.

---

### Task 2: Add Tracing Logger Infrastructure

**Files:**
- Modify: `Cargo.toml`
- Replace: `src/log.rs`
- Modify: `src/main.rs`
- Test: add unit tests in `src/log.rs`

- [ ] **Step 1: Add dependencies**

Add to `[dependencies]`:

```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "time", "registry"] }
tracing-appender = "0.2"
```

Do not add `env-filter`; log filtering is not controlled by env or config in this phase.

- [ ] **Step 2: Replace `src/log.rs` with logger API**

Implement these public APIs:

```rust
pub struct LogGuard {
    _file_guard: tracing_appender::non_blocking::WorkerGuard,
    _stderr_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

pub fn init_daemon() -> anyhow::Result<LogGuard>;
pub fn logs_dir() -> std::path::PathBuf;
pub fn log_file_path(now: time::OffsetDateTime) -> anyhow::Result<std::path::PathBuf>;
```

Implementation requirements:

- `logs_dir()` returns `state::history::state_dir().join("logs")`.
- `log_file_path(now)` converts `now` to local offset with `now.to_offset(local_offset())`, then returns `logs/shuo-YYYY-MM-DD.log`.
- If local offset cannot be determined, fall back to UTC but still format with offset.
- Create parent directory before opening the file.
- Use `tracing_appender::non_blocking` for the file writer.
- Mirror to stderr only when `std::io::stderr().is_terminal()` is true.
- Use `tracing_subscriber::registry()` with fmt layers.
- File layer records `DEBUG` and above for `shuohua` targets and `WARN` and above for dependency targets.
- Stderr mirror uses the same policy.
- Include timestamp, level, target, fields, and message.
- Keep formatting compact and human-readable; do not use JSON logs.

- [ ] **Step 3: Handle terminal detection**

Use stable std:

```rust
use std::io::IsTerminal;

let mirror_to_stderr = std::io::stderr().is_terminal();
```

- [ ] **Step 4: Initialize logger in daemon path**

In `run_daemon_process()`, after `DaemonLock::acquire()` and before config loading:

```rust
let _log_guard = log::init_daemon().context("initialize daemon logger")?;
```

Keep `_log_guard` alive until `run_daemon_process()` returns.

- [ ] **Step 5: Preserve early stderr fallback**

Do not try to log before `init_daemon()` succeeds. If `DaemonLock::acquire()` or `init_daemon()` fails, returning `Err` is fine; cargo/launchd stderr receives that early failure.

- [ ] **Step 6: Add logger tests**

Add tests for:

```rust
#[test]
fn log_file_path_uses_local_date_prefix()
```

The assertion should check the filename shape, not the machine timezone:

```rust
let path = log_file_path(time::macros::datetime!(2026-06-16 12:34:56 UTC)).unwrap();
let file_name = path.file_name().unwrap().to_string_lossy();
assert!(file_name.starts_with("shuo-"));
assert!(file_name.ends_with(".log"));
```

Add:

```rust
#[test]
fn logs_dir_lives_under_state_dir()
```

Assert `logs_dir().ends_with("logs")`.

- [ ] **Step 7: Verify infrastructure**

Run:

```bash
cargo fmt
cargo check
cargo test log
```

Expected: all pass.

- [ ] **Step 8: Commit logger infrastructure**

Run:

```bash
git status --short --branch -uall
git add Cargo.toml Cargo.lock src/log.rs src/main.rs
git commit -m "Add tracing daemon logger"
```

---

### Task 3: Migrate Daemon Entry, Reload, IPC, And Launchd Logging

**Files:**
- Modify: `src/main.rs`
- Modify: `src/reload.rs`
- Modify: `src/ipc/server.rs`
- Modify: `src/cli/service.rs`

- [ ] **Step 1: Remove launchd `RUST_LOG`**

In `src/cli/service.rs`, remove this plist block:

```xml
  <key>EnvironmentVariables</key>
  <dict>
    <key>RUST_LOG</key>
    <string>info</string>
  </dict>
```

Keep `StandardOutPath` and `StandardErrorPath`.

- [ ] **Step 2: Hide `--daemon` from normal help**

Change the clap arg in `src/cli/mod.rs`:

```rust
#[arg(long, hide = true)]
pub daemon: bool,
```

This keeps release usability while keeping the normal command surface clean.

- [ ] **Step 3: Replace daemon entry `eprintln!` calls**

In `src/main.rs`, convert:

```rust
eprintln!("[shuo] config ...");
```

to:

```rust
tracing::info!(
    config_path = %cfg_path.display(),
    trigger = %cfg.hotkey.trigger,
    parsed_trigger = %trigger,
    post_timeout_ms = cfg.post.timeout_ms,
    auto_paste = cfg.voice.auto_paste,
    record_audio = cfg.voice.record_audio,
    stop_delay_ms = cfg.voice.stop_delay_ms,
    vad_trace = cfg.voice.vad_trace,
    language = %cfg.ui.language,
    "daemon config loaded"
);
```

Convert daemon ready:

```rust
tracing::info!(
    uds = %socket_path.display(),
    trigger = %cfg_rx.borrow().hotkey.trigger,
    "daemon ready"
);
```

Convert fatal daemon thread / hotkey thread exits to `tracing::error!(error = ?e, "...")` immediately before `std::process::exit(2)`.

- [ ] **Step 4: Replace startup failure logs in main loop**

For app profile, post chain, and ASR provider init failures:

```rust
tracing::warn!(error = ?e, "app profile load failed");
tracing::warn!(error = ?e, "post chain load failed");
tracing::error!(error = ?e, "asr provider init failed");
```

Do not include hotwords or provider private config values.

- [ ] **Step 5: Replace hotkey pipe logs**

Use:

```rust
tracing::error!(error = %e, "hotkey pipe read failed");
tracing::warn!(frame = ?buf, "dropped unknown hotkey frame");
```

Raw frame bytes are not sensitive.

- [ ] **Step 6: Replace reload logs**

In `src/reload.rs`:

```rust
tracing::error!(error = ?e, "config watcher exited");
tracing::warn!(error = %e, "notify error");
tracing::warn!(error = ?e, "config reload failed; keeping previous config");
tracing::info!(path = %path.display(), "config reloaded");
tracing::debug!(language = %next, "language changed");
tracing::debug!(trigger = %next, parsed = %printed, "hotkey trigger changed");
tracing::warn!(trigger = ?next, error = ?e, "invalid hotkey; keeping previous trigger");
```

- [ ] **Step 7: Replace IPC logs**

In `src/ipc/server.rs`:

```rust
tracing::debug!(error = ?e, "ipc client ended");
tracing::error!(error = %e, "serialize IPC event failed");
tracing::warn!(error = ?e, "history read failed");
tracing::warn!(lagged = n, "IPC client lagged");
tracing::warn!("IPC client queue full");
```

Do not log UDS event payloads because they can contain text.

- [ ] **Step 8: Verify**

Run:

```bash
cargo fmt
cargo check
cargo test
rg -n "eprintln!|debug_println!" src/main.rs src/reload.rs src/ipc/server.rs src/cli/service.rs src/cli/mod.rs
```

Expected: no daemon-path `eprintln!` or `debug_println!` remains in those daemon modules; CLI `println!` remains.

- [ ] **Step 9: Commit**

Run:

```bash
git status --short --branch -uall
git add src/main.rs src/reload.rs src/ipc/server.rs src/cli/service.rs src/cli/mod.rs docs/CLI.md
git commit -m "Migrate daemon control logs to tracing"
```

---

### Task 4: Migrate Voice, Recorder, Dispatch, Post, ASR, And Observer Logs

**Files:**
- Modify: `src/voice/finish.rs`
- Modify: `src/voice/dispatch.rs`
- Modify: `src/voice/recorder.rs`
- Modify: `src/voice/observer.rs`
- Modify: `src/post/mod.rs`
- Modify: `src/asr/providers/apple.rs`
- Modify: `src/asr/providers/doubao.rs`
- Modify: `src/overlay/debug.rs`

- [ ] **Step 1: Add recording start anchors**

In both single-session and multi-session recording starts, replace `debug_println!` with:

```rust
tracing::info!(
    recording_id = %recording_id,
    provider = %provider.name(),
    app = ?params.start_app_context.bundle_id,
    multi_session = false,
    "recording started"
);
```

Use `multi_session = true` in the multi-session path.

- [ ] **Step 2: Log dev trace enabled once**

After `RecordingObserver::start(...)`, if `params.vad_trace` is true:

```rust
tracing::info!(recording_id = %recording_id, "dev voice trace enabled");
```

If `RecordingObserver` exposes the trace path during implementation, include `path = %path.display()`. Do not log trace contents.

- [ ] **Step 3: Convert voice errors and warnings**

Use these mappings:

```rust
tracing::warn!(recording_id = %recording_id, error = ?e, "record_audio enabled but audio path preparation failed");
tracing::error!(recording_id = %recording_id, error = ?e, "recorder start failed");
tracing::error!(recording_id = %recording_id, error = %err, "ASR open failed");
tracing::error!(recording_id = %recording_id, error = %e, "ASR send_pcm failed");
tracing::warn!(recording_id = %recording_id, "recorder ended unexpectedly");
tracing::warn!(recording_id = %recording_id, finalize_timeout_ms, "ASR final timed out");
tracing::error!(recording_id = %recording_id, error = %err, "ASR event error");
tracing::error!(recording_id = %recording_id, error = ?e, "clipboard write failed");
tracing::error!(recording_id = %recording_id, error = ?e, "history append failed");
```

- [ ] **Step 4: Delete sensitive debug narration**

Remove, do not convert:

```rust
crate::debug_println!("[shuo]   partial#{seq}: {text}");
crate::debug_println!("[shuo]   segment: {text}");
crate::debug_println!("[shuo]   segment (drain): {text}");
crate::debug_println!("[shuo]   partial#{seq} (drain): {text}");
crate::debug_println!("[shuo]   segment (final): {text}");
crate::debug_println!("[shuo]   partial#{seq} (final): {text}");
crate::debug_println!("[shuo] ✓ 最终: {}", out.text);
```

If a diagnostic is still needed, replace with length/count only:

```rust
tracing::debug!(recording_id = %recording_id, seq, chars = text.chars().count(), "ASR partial received");
```

Use this sparingly; avoid high-frequency logs unless it helps diagnose a known failure mode.

- [ ] **Step 5: Add recording end anchors**

At every terminal recording path, ensure one sparse summary exists:

```rust
tracing::info!(
    recording_id = %recording_id,
    status = "submitted",
    audio_ms,
    session_count,
    pipeline_steps = steps.len(),
    "recording ended"
);
```

For canceled/error/timeout paths, use the same message with `status = "canceled" | "error" | "timeout"` and include `error_kind` when available. Do not include text.

- [ ] **Step 6: Migrate dispatch logs**

In `src/voice/dispatch.rs`:

```rust
tracing::debug!("clipboard write succeeded");
tracing::debug!("auto paste succeeded");
tracing::warn!(error = ?e, "auto paste failed; text remains on clipboard");
```

If `recording_id` is not available in this layer, do not widen APIs just for it unless the call site already has the value and the change stays small.

- [ ] **Step 7: Migrate recorder logs**

In `src/voice/recorder.rs`:

```rust
tracing::warn!(error = ?e, "wav writer failed");
tracing::debug!(src_rate, channels, "recorder format selected");
tracing::warn!(error = %err, "recorder stream error");
```

- [ ] **Step 8: Migrate post logs**

In `src/post/mod.rs`:

```rust
tracing::debug!(step = %p.name(), duration_ms = step.duration_ms, "post step succeeded");
tracing::warn!(step = %p.name(), error = %e, "post step failed; skipped");
tracing::warn!(step = %p.name(), timeout_ms = timeout.as_millis(), "post step timed out; skipped");
```

Do not log `PipelineText`.

- [ ] **Step 9: Migrate observer failure log**

In `src/voice/observer.rs`, replace:

```rust
eprintln!("[trace] disabled: {e:#}");
```

with:

```rust
tracing::warn!(error = ?e, "dev voice trace disabled");
```

- [ ] **Step 10: Migrate Apple helper stderr**

In `src/asr/providers/apple.rs`, replace helper stderr `debug_println!` with:

```rust
tracing::debug!(line = %line, "apple helper stderr");
```

Before accepting this, inspect helper stderr content. If it may contain recognized text, replace with:

```rust
tracing::debug!("apple helper emitted stderr line");
```

- [ ] **Step 11: Migrate Doubao logs**

Convert connection log:

```rust
tracing::debug!(logid = %logid, "doubao connected");
```

For `DriftProbe`, do not log text. Replace drift/mismatch logs with:

```rust
tracing::warn!(
    utterance_index = i,
    previous_chars = prev.chars().count(),
    current_chars = current.chars().count(),
    "doubao utterance drift detected"
);
```

and:

```rust
tracing::warn!(
    ours_chars = ours.chars().count(),
    doubao_chars = doubao_text.chars().count(),
    "doubao final text mismatch"
);
```

- [ ] **Step 12: Migrate overlay debug logs**

`src/overlay/debug.rs` is debug-only SPI probing. Convert `eprintln!` to `tracing::debug!`, unless the line is obsolete. Keep selector names and type encodings; they are not user content.

- [ ] **Step 13: Verify privacy boundary**

Run:

```bash
rg -n "partial|segment|最终|clipboard|prompt|hotwords|text =" src/voice src/asr src/post src/log.rs
rg -n "eprintln!|debug_println!" src/voice src/asr src/post src/overlay src/log.rs
```

Manually inspect every hit. Expected:

- No `debug_println!` remains.
- No daemon-path `eprintln!` remains except genuinely early fallback if any.
- No `tracing::*` event logs recognized text, clipboard contents, prompts, or hotword values.

- [ ] **Step 14: Run checks**

Run:

```bash
cargo fmt
cargo check
cargo test
```

Expected: all pass.

- [ ] **Step 15: Commit**

Run:

```bash
git status --short --branch -uall
git add src/voice src/post src/asr src/overlay src/log.rs
git commit -m "Migrate voice diagnostics to tracing"
```

---

### Task 5: Partition History Into Monthly JSONL Files

**Files:**
- Modify: `src/state/history.rs`
- Modify: `src/ipc/server.rs`
- Test: `src/state/history.rs`, `src/ipc/server.rs`
- Modify docs if Task 1 did not already cover final text.

- [ ] **Step 1: Replace default history path API**

In `src/state/history.rs`, replace `default_path()` with:

```rust
pub fn history_dir() -> PathBuf {
    state_dir().join("history")
}

pub fn path_for_month(now: time::OffsetDateTime) -> Result<PathBuf> {
    let local = to_local_offset(now);
    let name = format!("{:04}-{:02}.jsonl", local.year(), u8::from(local.month()));
    Ok(history_dir().join(name))
}

pub fn append_default(record: &HistoryRecord) -> Result<()> {
    append_record(&path_for_month(record.started_at)?, record)
}
```

Keep `append_record(path, record)` for tests.

- [ ] **Step 2: Add local offset helper**

Use the same local time helper style as `src/log.rs`. If duplication becomes ugly, move the helper to one small shared internal function without adding a new abstraction layer.

- [ ] **Step 3: Add history listing API**

Add:

```rust
pub fn monthly_history_files() -> Result<Vec<PathBuf>> {
    let dir = history_dir();
    let mut files = Vec::new();
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "jsonl") {
                    files.push(path);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("read history dir {}", dir.display())),
    }
    files.sort();
    files.reverse();
    Ok(files)
}
```

- [ ] **Step 4: Update IPC history read**

Change `handle_client` to call:

```rust
let records = read_history(limit, before.as_deref(), query.as_deref())
```

Change `read_history` signature to:

```rust
fn read_history(limit: usize, before: Option<&str>, query: Option<&str>) -> Result<Vec<HistoryRecord>>
```

Inside it:

- Iterate `history::monthly_history_files()?`.
- Read newest files first.
- Parse each non-empty line.
- Apply `before` and `query`.
- Collect records.
- Sort by `started_at` descending.
- Truncate to `limit`.

No legacy top-level `history.jsonl` compatibility.

- [ ] **Step 5: Keep query behavior unchanged**

`history_matches()` may continue searching recognized text because this is a TUI/history query path, not official daemon logging. Do not log the query or matched text.

- [ ] **Step 6: Add tests**

In `src/state/history.rs`, add:

```rust
#[test]
fn path_for_month_uses_year_month_jsonl_name()
```

Assert filename matches `YYYY-MM.jsonl` shape.

Add:

```rust
#[test]
fn append_default_writes_under_history_dir()
```

If using real default state dir is awkward, keep this as an `append_record()` path test and add a pure `path_for_month()` test.

In `src/ipc/server.rs`, update or add a test that creates two monthly files and verifies `read_history()` returns newest records first and respects `limit`.

- [ ] **Step 7: Update docs if needed**

Ensure `docs/SCHEMA.md` says:

- monthly files use local month names
- record timestamps remain UTC
- no automatic migration from old top-level `history.jsonl`

- [ ] **Step 8: Verify**

Run:

```bash
cargo fmt
cargo check
cargo test history
cargo test ipc
cargo test
```

Expected: all pass.

- [ ] **Step 9: Commit**

Run:

```bash
git status --short --branch -uall
git add src/state/history.rs src/ipc/server.rs docs/SCHEMA.md
git commit -m "Partition history by month"
```

---

### Task 6: Final Audit And Cleanup

**Files:**
- Whole repo audit.

- [ ] **Step 1: Audit print macros**

Run:

```bash
rg -n "println!|eprintln!|debug_println!|dbg!" src build.rs
```

Expected allowed hits:

- `println!` in CLI commands and `build.rs` cargo directives.
- TUI stdout setup.
- No daemon business `eprintln!`.
- No `debug_println!`.

- [ ] **Step 2: Audit sensitive logging**

Run:

```bash
rg -n "tracing::(debug|info|warn|error)!|debug!|info!|warn!|error!" src
```

Manually inspect all events for sensitive fields. Confirm no official log event records:

- recognized text
- clipboard text
- prompt body
- hotwords contents
- post input/output
- raw provider payload that may contain text

- [ ] **Step 3: Audit missing useful diagnostics**

Inspect these flows and add sparse logs only if absent:

- daemon startup: config loaded, UDS ready
- hotkey tap failure and pipe failure
- app profile load failure
- post chain load failure
- ASR provider init/open/send/finalize/resume failure
- recorder start/stream failure
- first-audio timeout
- post step failed/timeout skipped
- dispatch clipboard/autopaste failure
- history append failure
- IPC serialization/history read failure
- config reload parse failure

Each added log must use structured fields and must pass the privacy audit.

- [ ] **Step 4: Run full verification**

Run:

```bash
cargo fmt
cargo check
cargo test
git status --short --branch -uall
```

Expected:

- `cargo fmt` changes are included.
- `cargo check` passes.
- `cargo test` passes.
- Only intended files are modified.

- [ ] **Step 5: Optional foreground daemon smoke test**

Do not start long-lived GUI/daemon for the user. If the implementer is allowed to run a short foreground check, run:

```bash
cargo run -- --daemon
```

Then stop with Ctrl+C after confirming:

- terminal shows mirrored tracing logs
- `~/.local/state/shuohua/logs/shuo-YYYY-MM-DD.log` exists
- launchd stdout/stderr are not involved

If not run, state clearly that real daemon behavior needs manual verification.

- [ ] **Step 6: Commit final cleanup if needed**

Only if Task 6 made code/doc changes:

```bash
git status --short --branch -uall
git add <changed files>
git commit -m "Audit daemon logging coverage"
```

---

## Review Checklist

- [ ] No env var or config controls log level.
- [ ] No `shuo logs` command was added.
- [ ] `--daemon` remains available in release.
- [ ] Foreground `shuo --daemon` mirrors the same logs to terminal.
- [ ] launchd stdout/stderr remain fallback only.
- [ ] Official logs use local date filenames and local timestamps with offset.
- [ ] History files use local month filenames.
- [ ] History record timestamps remain UTC RFC3339.
- [ ] Official logs do not contain sensitive text.
- [ ] `voice/observer` remains separate dev trace sidecar.
- [ ] `cargo fmt`, `cargo check`, and `cargo test` pass after implementation.
