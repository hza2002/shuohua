use shuohua::ipc::protocol::{Command, Event, WireState};

type DaemonClient = shuohua::client_api::DaemonClient;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiShellMetadata {
    app_name: &'static str,
    phase: &'static str,
    daemon_connected: bool,
    placeholder_ready: bool,
}

#[tauri::command]
fn gui_shell_metadata() -> GuiShellMetadata {
    GuiShellMetadata {
        app_name: "Shuohua",
        phase: "Phase 9n",
        daemon_connected: false,
        placeholder_ready: true,
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenRequestPlan {
    history_limit: usize,
    requires_daemon_connection: bool,
    transport_opened: bool,
    commands: Vec<GuiFirstScreenCommandSummary>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenCommandSummary {
    kind: &'static str,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiDaemonStatusSnapshot {
    connected: bool,
    transport_opened: bool,
    snapshot_available: bool,
    state_label: &'static str,
    pid: Option<u32>,
    uptime_ms: Option<u64>,
    recording_id: Option<String>,
    request: GuiDaemonStatusRequestSummary,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiDaemonStatusRequestSummary {
    request_kind: &'static str,
    requires_daemon_connection: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiDaemonStatusRequestError {
    kind: &'static str,
    message: String,
    recoverable: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiHistorySummary {
    connected: bool,
    transport_opened: bool,
    summary_available: bool,
    limit: usize,
    page_record_count: usize,
    matched: Option<u64>,
    page_stats: Option<GuiHistoryAggregateStats>,
    stats_status: Option<&'static str>,
    total: Option<GuiHistoryAggregateStats>,
    current_month: Option<GuiHistoryAggregateStats>,
    today: Option<GuiHistoryAggregateStats>,
    latest_record: Option<GuiHistoryRecordSummary>,
    request: GuiHistorySummaryRequestSummary,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiHistoryRecordSummary {
    id: String,
    status: &'static str,
    duration_ms: u64,
    words: usize,
    text_preview: String,
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiHistoryAggregateStats {
    records: u64,
    words: u64,
    duration_ms: u64,
    asr_duration_ms: u64,
    asr_audio_ms: u64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiHistorySummaryRequestSummary {
    request_kind: &'static str,
    requires_daemon_connection: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiHistorySummaryRequestError {
    kind: &'static str,
    message: String,
    recoverable: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenSummary {
    connected: bool,
    transport_opened: bool,
    summary_available: bool,
    history_limit: usize,
    status: GuiDaemonStatusSnapshot,
    history: GuiHistorySummary,
    timing: GuiFirstScreenSummaryTiming,
    request: GuiFirstScreenSummaryRequestSummary,
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenSummaryTiming {
    connect_duration_ms: u64,
    first_event_ms: u64,
    ready_ms: u64,
    request_duration_ms: u64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenSummaryRequestSummary {
    request_kind: &'static str,
    requires_daemon_connection: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenSummaryRequestError {
    kind: &'static str,
    message: String,
    recoverable: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenRefreshShape {
    explicit_trigger_required: bool,
    default_history_limit: usize,
    requires_daemon_connection: bool,
    transport_opened: bool,
    invoke_target: &'static str,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenRefreshAffordanceShape {
    button_label: &'static str,
    enabled: bool,
    explicit_trigger_required: bool,
    invoke_target: &'static str,
    default_history_limit: usize,
    loading: bool,
    source: &'static str,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenReadinessShape {
    ready: bool,
    inputs: GuiFirstScreenReadinessInputs,
    timing: GuiFirstScreenReadinessTimingShape,
    source: &'static str,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenReadinessInputs {
    daemon_status_received: bool,
    history_page_received: bool,
    history_stats_received: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenReadinessTimingShape {
    connect_duration_ms: Option<u64>,
    first_event_ms: Option<u64>,
    ready_ms: Option<u64>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenOfflineShape {
    connected: bool,
    problem_kind: &'static str,
    recoverable: bool,
    retry_allowed: bool,
    auto_start_allowed: bool,
    service_management_allowed: bool,
    source: &'static str,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenCommandPolicyShape {
    commands: Vec<GuiFirstScreenCommandPolicyEntry>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiFirstScreenCommandPolicyEntry {
    command_name: &'static str,
    auto_invocation_allowed: bool,
    requires_explicit_trigger: bool,
    opens_daemon_transport: bool,
    policy_reason: &'static str,
}

#[tauri::command]
fn gui_first_screen_request_plan(history_limit: Option<usize>) -> GuiFirstScreenRequestPlan {
    let history_limit = history_limit.unwrap_or(20);
    let commands = shuohua::client_api::first_screen_commands(history_limit)
        .into_iter()
        .map(|command| GuiFirstScreenCommandSummary {
            kind: first_screen_command_kind(&command),
        })
        .collect();

    GuiFirstScreenRequestPlan {
        history_limit,
        requires_daemon_connection: true,
        transport_opened: false,
        commands,
    }
}

#[tauri::command]
fn gui_daemon_status_snapshot() -> GuiDaemonStatusSnapshot {
    empty_daemon_status_snapshot()
}

#[tauri::command]
fn gui_first_screen_refresh_shape(history_limit: Option<usize>) -> GuiFirstScreenRefreshShape {
    GuiFirstScreenRefreshShape {
        explicit_trigger_required: true,
        default_history_limit: history_limit.unwrap_or(20),
        requires_daemon_connection: true,
        transport_opened: false,
        invoke_target: "gui_first_screen_summary_request_once",
    }
}

#[tauri::command]
fn gui_first_screen_refresh_affordance_shape(
    history_limit: Option<usize>,
) -> GuiFirstScreenRefreshAffordanceShape {
    GuiFirstScreenRefreshAffordanceShape {
        button_label: "Refresh",
        enabled: true,
        explicit_trigger_required: true,
        invoke_target: "gui_first_screen_summary_request_once",
        default_history_limit: history_limit.unwrap_or(20),
        loading: false,
        source: "placeholder",
    }
}

#[tauri::command]
fn gui_first_screen_readiness_shape() -> GuiFirstScreenReadinessShape {
    GuiFirstScreenReadinessShape {
        ready: false,
        inputs: GuiFirstScreenReadinessInputs {
            daemon_status_received: false,
            history_page_received: false,
            history_stats_received: false,
        },
        timing: GuiFirstScreenReadinessTimingShape {
            connect_duration_ms: None,
            first_event_ms: None,
            ready_ms: None,
        },
        source: "placeholder",
    }
}

#[tauri::command]
fn gui_first_screen_offline_shape() -> GuiFirstScreenOfflineShape {
    GuiFirstScreenOfflineShape {
        connected: false,
        problem_kind: "daemonOffline",
        recoverable: true,
        retry_allowed: true,
        auto_start_allowed: false,
        service_management_allowed: false,
        source: "placeholder",
    }
}

#[tauri::command]
fn gui_first_screen_command_policy_shape() -> GuiFirstScreenCommandPolicyShape {
    GuiFirstScreenCommandPolicyShape {
        commands: vec![
            static_command_policy("gui_shell_metadata"),
            static_command_policy("gui_first_screen_request_plan"),
            static_command_policy("gui_daemon_status_snapshot"),
            static_command_policy("gui_first_screen_refresh_shape"),
            static_command_policy("gui_first_screen_readiness_shape"),
            static_command_policy("gui_first_screen_offline_shape"),
            one_shot_command_policy("gui_daemon_status_request_once"),
            one_shot_command_policy("gui_history_summary_request_once"),
            one_shot_command_policy("gui_first_screen_summary_request_once"),
        ],
    }
}

fn static_command_policy(command_name: &'static str) -> GuiFirstScreenCommandPolicyEntry {
    GuiFirstScreenCommandPolicyEntry {
        command_name,
        auto_invocation_allowed: true,
        requires_explicit_trigger: false,
        opens_daemon_transport: false,
        policy_reason: "staticPreflight",
    }
}

fn one_shot_command_policy(command_name: &'static str) -> GuiFirstScreenCommandPolicyEntry {
    GuiFirstScreenCommandPolicyEntry {
        command_name,
        auto_invocation_allowed: false,
        requires_explicit_trigger: true,
        opens_daemon_transport: true,
        policy_reason: "opensDaemonTransport",
    }
}

#[tauri::command]
async fn gui_daemon_status_request_once(
) -> Result<GuiDaemonStatusSnapshot, GuiDaemonStatusRequestError> {
    let mut client = DaemonClient::connect_default()
        .await
        .map_err(|error| daemon_status_request_error("connectFailed", error))?;

    client
        .send(&Command::DaemonStatus)
        .await
        .map_err(|error| daemon_status_request_error("writeFailed", error))?;

    match client
        .recv_until(|event| {
            if let Some(snapshot) = gui_daemon_status_snapshot_from_event(&event) {
                return Ok(std::ops::ControlFlow::Break(Ok(snapshot)));
            }

            if let Event::Error { kind, msg, .. } = event {
                return Ok(std::ops::ControlFlow::Break(Err(
                    daemon_status_request_message("daemonError", format!("{kind}: {msg}")),
                )));
            }

            Ok(std::ops::ControlFlow::Continue(()))
        })
        .await
        .map_err(|error| daemon_status_request_error("readFailed", error))?
    {
        Some(result) => result,
        None => Err(daemon_status_request_message(
            "daemonClosed",
            "daemon closed IPC before status reply",
        )),
    }
}

#[tauri::command]
async fn gui_history_summary_request_once(
    history_limit: Option<usize>,
) -> Result<GuiHistorySummary, GuiHistorySummaryRequestError> {
    let history_limit = history_limit.unwrap_or(20);
    let mut client = DaemonClient::connect_default()
        .await
        .map_err(|error| history_summary_request_error("connectFailed", error))?;

    client
        .send(&history_summary_page_command(history_limit))
        .await
        .map_err(|error| history_summary_request_error("writeFailed", error))?;
    client
        .send(&Command::GetHistoryStats)
        .await
        .map_err(|error| history_summary_request_error("writeFailed", error))?;

    let mut history_event = None;
    let mut stats_event = None;
    match client
        .recv_until(|event| {
            match event {
                Event::History { .. } => history_event = Some(event),
                Event::HistoryStats { .. } => stats_event = Some(event),
                Event::Error { kind, msg, .. } => {
                    return Ok(std::ops::ControlFlow::Break(Err(
                        history_summary_request_message("daemonError", format!("{kind}: {msg}")),
                    )));
                }
                _ => {}
            }

            if let (Some(history), Some(stats)) = (&history_event, &stats_event) {
                return Ok(std::ops::ControlFlow::Break(Ok(
                    gui_history_summary_from_events(history, stats, history_limit)
                        .expect("history summary events were pre-matched"),
                )));
            }

            Ok(std::ops::ControlFlow::Continue(()))
        })
        .await
        .map_err(|error| history_summary_request_error("readFailed", error))?
    {
        Some(result) => result,
        None => Err(history_summary_request_message(
            "daemonClosed",
            "daemon closed IPC before history summary replies",
        )),
    }
}

#[tauri::command]
async fn gui_first_screen_summary_request_once(
    history_limit: Option<usize>,
) -> Result<GuiFirstScreenSummary, GuiFirstScreenSummaryRequestError> {
    let history_limit = history_limit.unwrap_or(20);
    let request_started = std::time::Instant::now();
    let mut client = DaemonClient::connect_default()
        .await
        .map_err(|error| first_screen_summary_request_error("connectFailed", error))?;
    let connect_duration_ms = elapsed_ms(request_started.elapsed());

    client
        .send(&Command::DaemonStatus)
        .await
        .map_err(|error| first_screen_summary_request_error("writeFailed", error))?;
    client
        .send(&history_summary_page_command(history_limit))
        .await
        .map_err(|error| first_screen_summary_request_error("writeFailed", error))?;
    client
        .send(&Command::GetHistoryStats)
        .await
        .map_err(|error| first_screen_summary_request_error("writeFailed", error))?;

    let mut status_event = None;
    let mut history_event = None;
    let mut stats_event = None;
    let mut first_event_ms = None;
    match client
        .recv_until(|event| {
            if first_event_ms.is_none()
                && matches!(
                    event,
                    Event::DaemonStatus { .. } | Event::History { .. } | Event::HistoryStats { .. }
                )
            {
                first_event_ms = Some(elapsed_ms(request_started.elapsed()));
            }

            match event {
                Event::DaemonStatus { .. } => status_event = Some(event),
                Event::History { .. } => history_event = Some(event),
                Event::HistoryStats { .. } => stats_event = Some(event),
                Event::Error { kind, msg, .. } => {
                    return Ok(std::ops::ControlFlow::Break(Err(
                        first_screen_summary_request_message(
                            "daemonError",
                            format!("{kind}: {msg}"),
                        ),
                    )));
                }
                _ => {}
            }

            if let (Some(status), Some(history), Some(stats)) =
                (&status_event, &history_event, &stats_event)
            {
                let ready_ms = elapsed_ms(request_started.elapsed());
                let timing = GuiFirstScreenSummaryTiming {
                    connect_duration_ms,
                    first_event_ms: first_event_ms.unwrap_or(ready_ms),
                    ready_ms,
                    request_duration_ms: ready_ms,
                };
                return Ok(std::ops::ControlFlow::Break(Ok(
                    gui_first_screen_summary_from_parts(
                        status,
                        history,
                        stats,
                        history_limit,
                        timing,
                    )
                        .expect("first-screen summary events were pre-matched"),
                )));
            }

            Ok(std::ops::ControlFlow::Continue(()))
        })
        .await
        .map_err(|error| first_screen_summary_request_error("readFailed", error))?
    {
        Some(result) => result,
        None => Err(first_screen_summary_request_message(
            "daemonClosed",
            "daemon closed IPC before first-screen summary replies",
        )),
    }
}

fn elapsed_ms(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn empty_daemon_status_snapshot() -> GuiDaemonStatusSnapshot {
    GuiDaemonStatusSnapshot {
        connected: false,
        transport_opened: false,
        snapshot_available: false,
        state_label: "disconnected",
        pid: None,
        uptime_ms: None,
        recording_id: None,
        request: GuiDaemonStatusRequestSummary {
            request_kind: daemon_status_request_kind(&Command::DaemonStatus),
            requires_daemon_connection: true,
        },
    }
}

fn daemon_status_request_error(
    kind: &'static str,
    error: impl std::fmt::Display,
) -> GuiDaemonStatusRequestError {
    daemon_status_request_message(kind, error.to_string())
}

fn daemon_status_request_message(
    kind: &'static str,
    message: impl Into<String>,
) -> GuiDaemonStatusRequestError {
    GuiDaemonStatusRequestError {
        kind,
        message: message.into(),
        recoverable: true,
    }
}

fn history_summary_page_command(limit: usize) -> Command {
    Command::GetHistory {
        limit,
        before: None,
        before_id: None,
        query: None,
    }
}

fn history_summary_request_error(
    kind: &'static str,
    error: impl std::fmt::Display,
) -> GuiHistorySummaryRequestError {
    history_summary_request_message(kind, error.to_string())
}

fn history_summary_request_message(
    kind: &'static str,
    message: impl Into<String>,
) -> GuiHistorySummaryRequestError {
    GuiHistorySummaryRequestError {
        kind,
        message: message.into(),
        recoverable: true,
    }
}

fn first_screen_summary_request_error(
    kind: &'static str,
    error: impl std::fmt::Display,
) -> GuiFirstScreenSummaryRequestError {
    first_screen_summary_request_message(kind, error.to_string())
}

fn first_screen_summary_request_message(
    kind: &'static str,
    message: impl Into<String>,
) -> GuiFirstScreenSummaryRequestError {
    GuiFirstScreenSummaryRequestError {
        kind,
        message: message.into(),
        recoverable: true,
    }
}

#[allow(dead_code)]
fn gui_first_screen_summary_from_events(
    status: &Event,
    history: &Event,
    stats: &Event,
    history_limit: usize,
) -> Option<GuiFirstScreenSummary> {
    gui_first_screen_summary_from_parts(
        status,
        history,
        stats,
        history_limit,
        GuiFirstScreenSummaryTiming {
            connect_duration_ms: 0,
            first_event_ms: 0,
            ready_ms: 0,
            request_duration_ms: 0,
        },
    )
}

fn gui_first_screen_summary_from_parts(
    status: &Event,
    history: &Event,
    stats: &Event,
    history_limit: usize,
    timing: GuiFirstScreenSummaryTiming,
) -> Option<GuiFirstScreenSummary> {
    let status = gui_daemon_status_snapshot_from_event(status)?;
    let history = gui_history_summary_from_events(history, stats, history_limit)?;

    Some(GuiFirstScreenSummary {
        connected: true,
        transport_opened: true,
        summary_available: true,
        history_limit,
        status,
        history,
        timing,
        request: GuiFirstScreenSummaryRequestSummary {
            request_kind: "firstScreenSummary",
            requires_daemon_connection: true,
        },
    })
}

#[allow(dead_code)]
fn gui_history_summary_from_events(
    history: &Event,
    stats: &Event,
    limit: usize,
) -> Option<GuiHistorySummary> {
    let Event::History {
        records,
        matched,
        stats: page_stats,
    } = history
    else {
        return None;
    };
    let Event::HistoryStats { snapshot } = stats else {
        return None;
    };

    Some(GuiHistorySummary {
        connected: true,
        transport_opened: true,
        summary_available: true,
        limit,
        page_record_count: records.len(),
        matched: *matched,
        page_stats: page_stats.map(gui_history_aggregate_stats),
        stats_status: Some(history_stats_status_label(snapshot.status)),
        total: Some(gui_history_aggregate_stats(snapshot.total)),
        current_month: Some(gui_history_aggregate_stats(snapshot.current_month)),
        today: Some(gui_history_aggregate_stats(snapshot.today)),
        latest_record: records.first().map(gui_history_record_summary),
        request: GuiHistorySummaryRequestSummary {
            request_kind: history_summary_request_kind(&history_summary_page_command(limit)),
            requires_daemon_connection: true,
        },
    })
}

fn gui_history_aggregate_stats(stats: shuohua::history::AggregateStats) -> GuiHistoryAggregateStats {
    GuiHistoryAggregateStats {
        records: stats.records,
        words: stats.words,
        duration_ms: stats.duration_ms,
        asr_duration_ms: stats.asr_duration_ms,
        asr_audio_ms: stats.asr_audio_ms,
    }
}

fn gui_history_record_summary(record: &shuohua::history::HistoryRecord) -> GuiHistoryRecordSummary {
    GuiHistoryRecordSummary {
        id: record.id.clone(),
        status: history_status_label(record.status),
        duration_ms: record.duration_ms,
        words: record.text_stats().words,
        text_preview: record.text.chars().take(80).collect(),
    }
}

#[allow(dead_code)]
fn gui_daemon_status_snapshot_from_event(event: &Event) -> Option<GuiDaemonStatusSnapshot> {
    let Event::DaemonStatus {
        pid,
        uptime_ms,
        state,
        recording_id,
    } = event
    else {
        return None;
    };

    Some(GuiDaemonStatusSnapshot {
        connected: true,
        transport_opened: true,
        snapshot_available: true,
        state_label: wire_state_label(*state),
        pid: Some(*pid),
        uptime_ms: Some(*uptime_ms),
        recording_id: recording_id.clone(),
        request: GuiDaemonStatusRequestSummary {
            request_kind: daemon_status_request_kind(&Command::DaemonStatus),
            requires_daemon_connection: true,
        },
    })
}

fn wire_state_label(state: WireState) -> &'static str {
    match state {
        WireState::Idle => "idle",
        WireState::Recording => "recording",
        WireState::Stopping => "stopping",
        WireState::Error => "error",
    }
}

fn history_status_label(status: shuohua::history::HistoryStatus) -> &'static str {
    match status {
        shuohua::history::HistoryStatus::Submitted => "submitted",
        shuohua::history::HistoryStatus::Canceled => "canceled",
        shuohua::history::HistoryStatus::Empty => "empty",
        shuohua::history::HistoryStatus::Error => "error",
        shuohua::history::HistoryStatus::Timeout => "timeout",
    }
}

fn history_stats_status_label(status: shuohua::history::HistoryStatsStatus) -> &'static str {
    match status {
        shuohua::history::HistoryStatsStatus::Ready => "ready",
        shuohua::history::HistoryStatsStatus::Stale => "stale",
        shuohua::history::HistoryStatsStatus::Unavailable => "unavailable",
    }
}

fn first_screen_command_kind(command: &Command) -> &'static str {
    match command {
        Command::Subscribe => "subscribe",
        Command::DaemonStatus => "daemonStatus",
        Command::GetHistory { .. } => "historyPage",
        Command::GetHistoryStats => "historyStats",
        _ => "other",
    }
}

fn daemon_status_request_kind(command: &Command) -> &'static str {
    match command {
        Command::DaemonStatus => "daemonStatus",
        _ => "other",
    }
}

fn history_summary_request_kind(command: &Command) -> &'static str {
    match command {
        Command::GetHistory { .. } => "historySummary",
        _ => "other",
    }
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            gui_shell_metadata,
            gui_first_screen_request_plan,
            gui_daemon_status_snapshot,
            gui_first_screen_refresh_shape,
            gui_first_screen_refresh_affordance_shape,
            gui_first_screen_readiness_shape,
            gui_first_screen_offline_shape,
            gui_first_screen_command_policy_shape,
            gui_daemon_status_request_once,
            gui_history_summary_request_once,
            gui_first_screen_summary_request_once
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Shuohua GUI");
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuohua::history::{
        AggregateStats, HistoryRecord, HistoryStatsSnapshot, HistoryStatsStatus,
    };
    use shuohua::ipc::protocol::{Event, WireState};

    #[test]
    fn daemon_status_event_maps_to_snapshot_shape_without_ipc() {
        let event = Event::DaemonStatus {
            pid: 42,
            uptime_ms: 12_345,
            state: WireState::Recording,
            recording_id: Some("01STATUS".to_string()),
        };

        let snapshot = gui_daemon_status_snapshot_from_event(&event).unwrap();

        assert!(snapshot.connected);
        assert!(snapshot.transport_opened);
        assert!(snapshot.snapshot_available);
        assert_eq!(snapshot.state_label, "recording");
        assert_eq!(snapshot.pid, Some(42));
        assert_eq!(snapshot.uptime_ms, Some(12_345));
        assert_eq!(snapshot.recording_id.as_deref(), Some("01STATUS"));
        assert_eq!(snapshot.request.request_kind, "daemonStatus");
        assert!(snapshot.request.requires_daemon_connection);
        assert!(gui_daemon_status_snapshot_from_event(&Event::HistoryChanged).is_none());
    }

    #[test]
    fn daemon_status_request_error_is_recoverable_shape() {
        let error = daemon_status_request_message("daemonClosed", "closed before reply");

        assert_eq!(error.kind, "daemonClosed");
        assert_eq!(error.message, "closed before reply");
        assert!(error.recoverable);
    }

    #[test]
    fn history_summary_events_map_to_compact_shape_without_ipc() {
        let history = Event::History {
            records: vec![sample_record("01HISTORY", "hello from gui history summary")],
            matched: Some(7),
            stats: Some(AggregateStats {
                records: 2,
                words: 9,
                duration_ms: 1_000,
                asr_duration_ms: 900,
                asr_audio_ms: 800,
            }),
        };
        let stats = Event::HistoryStats {
            snapshot: HistoryStatsSnapshot {
                status: HistoryStatsStatus::Ready,
                total: AggregateStats {
                    records: 10,
                    words: 40,
                    duration_ms: 4_000,
                    asr_duration_ms: 3_000,
                    asr_audio_ms: 2_000,
                },
                current_month: AggregateStats {
                    records: 3,
                    words: 12,
                    duration_ms: 1_200,
                    asr_duration_ms: 1_100,
                    asr_audio_ms: 1_000,
                },
                today: AggregateStats {
                    records: 1,
                    words: 4,
                    duration_ms: 400,
                    asr_duration_ms: 300,
                    asr_audio_ms: 200,
                },
                error: None,
            },
        };

        let summary = gui_history_summary_from_events(&history, &stats, 20).unwrap();

        assert!(summary.connected);
        assert!(summary.transport_opened);
        assert!(summary.summary_available);
        assert_eq!(summary.limit, 20);
        assert_eq!(summary.page_record_count, 1);
        assert_eq!(summary.matched, Some(7));
        assert_eq!(summary.page_stats.unwrap().records, 2);
        assert_eq!(summary.stats_status, Some("ready"));
        assert_eq!(summary.total.unwrap().records, 10);
        assert_eq!(summary.current_month.unwrap().records, 3);
        assert_eq!(summary.today.unwrap().records, 1);
        let latest = summary.latest_record.unwrap();
        assert_eq!(latest.id, "01HISTORY");
        assert_eq!(latest.status, "submitted");
        assert_eq!(latest.words, 5);
        assert_eq!(latest.text_preview, "hello from gui history summary");
        assert_eq!(summary.request.request_kind, "historySummary");
        assert!(summary.request.requires_daemon_connection);
        assert!(gui_history_summary_from_events(&Event::HistoryChanged, &stats, 20).is_none());
        assert!(gui_history_summary_from_events(&history, &Event::HistoryChanged, 20).is_none());
    }

    #[test]
    fn history_summary_request_error_is_recoverable_shape() {
        let error = history_summary_request_message("daemonClosed", "closed before replies");

        assert_eq!(error.kind, "daemonClosed");
        assert_eq!(error.message, "closed before replies");
        assert!(error.recoverable);
    }

    #[test]
    fn first_screen_summary_events_map_to_combined_shape_without_ipc() {
        let status = Event::DaemonStatus {
            pid: 99,
            uptime_ms: 54_321,
            state: WireState::Idle,
            recording_id: None,
        };
        let history = Event::History {
            records: vec![sample_record("01FIRST", "first screen summary")],
            matched: Some(1),
            stats: None,
        };
        let stats = Event::HistoryStats {
            snapshot: HistoryStatsSnapshot {
                status: HistoryStatsStatus::Ready,
                total: AggregateStats {
                    records: 1,
                    words: 3,
                    duration_ms: 333,
                    asr_duration_ms: 222,
                    asr_audio_ms: 111,
                },
                current_month: AggregateStats::default(),
                today: AggregateStats::default(),
                error: None,
            },
        };

        let summary =
            gui_first_screen_summary_from_events(&status, &history, &stats, 20).unwrap();

        assert!(summary.connected);
        assert!(summary.transport_opened);
        assert!(summary.summary_available);
        assert_eq!(summary.history_limit, 20);
        assert_eq!(summary.timing.connect_duration_ms, 0);
        assert_eq!(summary.timing.first_event_ms, 0);
        assert_eq!(summary.timing.ready_ms, 0);
        assert_eq!(summary.timing.request_duration_ms, 0);
        assert_eq!(summary.status.state_label, "idle");
        assert_eq!(summary.status.pid, Some(99));
        assert_eq!(summary.history.page_record_count, 1);
        assert_eq!(
            summary.history.latest_record.as_ref().map(|record| record.id.as_str()),
            Some("01FIRST")
        );
        assert_eq!(summary.request.request_kind, "firstScreenSummary");
        assert!(summary.request.requires_daemon_connection);
        assert!(
            gui_first_screen_summary_from_events(&Event::HistoryChanged, &history, &stats, 20)
                .is_none()
        );
        assert!(gui_first_screen_summary_from_events(
            &status,
            &Event::HistoryChanged,
            &stats,
            20
        )
        .is_none());
        assert!(gui_first_screen_summary_from_events(
            &status,
            &history,
            &Event::HistoryChanged,
            20
        )
        .is_none());
    }

    #[test]
    fn first_screen_summary_request_error_is_recoverable_shape() {
        let error =
            first_screen_summary_request_message("daemonClosed", "closed before summaries");

        assert_eq!(error.kind, "daemonClosed");
        assert_eq!(error.message, "closed before summaries");
        assert!(error.recoverable);
    }

    #[test]
    fn first_screen_refresh_shape_is_static_and_explicit() {
        let shape = gui_first_screen_refresh_shape(Some(12));

        assert!(shape.explicit_trigger_required);
        assert_eq!(shape.default_history_limit, 12);
        assert!(shape.requires_daemon_connection);
        assert!(!shape.transport_opened);
        assert_eq!(
            shape.invoke_target,
            "gui_first_screen_summary_request_once"
        );
    }

    #[test]
    fn first_screen_refresh_affordance_shape_is_static_placeholder() {
        let shape = gui_first_screen_refresh_affordance_shape(Some(12));

        assert_eq!(shape.button_label, "Refresh");
        assert!(shape.enabled);
        assert!(shape.explicit_trigger_required);
        assert_eq!(
            shape.invoke_target,
            "gui_first_screen_summary_request_once"
        );
        assert_eq!(shape.default_history_limit, 12);
        assert!(!shape.loading);
        assert_eq!(shape.source, "placeholder");
    }

    #[test]
    fn first_screen_readiness_shape_is_static_placeholder() {
        let shape = gui_first_screen_readiness_shape();

        assert!(!shape.ready);
        assert!(!shape.inputs.daemon_status_received);
        assert!(!shape.inputs.history_page_received);
        assert!(!shape.inputs.history_stats_received);
        assert_eq!(shape.timing.connect_duration_ms, None);
        assert_eq!(shape.timing.first_event_ms, None);
        assert_eq!(shape.timing.ready_ms, None);
        assert_eq!(shape.source, "placeholder");
    }

    #[test]
    fn first_screen_offline_shape_is_static_placeholder() {
        let shape = gui_first_screen_offline_shape();

        assert!(!shape.connected);
        assert_eq!(shape.problem_kind, "daemonOffline");
        assert!(shape.recoverable);
        assert!(shape.retry_allowed);
        assert!(!shape.auto_start_allowed);
        assert!(!shape.service_management_allowed);
        assert_eq!(shape.source, "placeholder");
    }

    #[test]
    fn first_screen_command_policy_keeps_one_shots_explicit() {
        let shape = gui_first_screen_command_policy_shape();

        let metadata = shape
            .commands
            .iter()
            .find(|entry| entry.command_name == "gui_shell_metadata")
            .unwrap();
        assert!(metadata.auto_invocation_allowed);
        assert!(!metadata.requires_explicit_trigger);
        assert!(!metadata.opens_daemon_transport);
        assert_eq!(metadata.policy_reason, "staticPreflight");

        let summary = shape
            .commands
            .iter()
            .find(|entry| entry.command_name == "gui_first_screen_summary_request_once")
            .unwrap();
        assert!(!summary.auto_invocation_allowed);
        assert!(summary.requires_explicit_trigger);
        assert!(summary.opens_daemon_transport);
        assert_eq!(summary.policy_reason, "opensDaemonTransport");
    }

    fn sample_record(id: &str, text: &str) -> HistoryRecord {
        serde_json::from_value(serde_json::json!({
            "version": 1,
            "id": id,
            "started_at": "2023-11-14T22:13:20Z",
            "ended_at": "2023-11-14T22:13:20Z",
            "duration_ms": 321,
            "status": "submitted",
            "app": "com.example.Editor",
            "text": text,
            "asr": {
                "provider": "test",
                "text": text,
                "duration_ms": 321,
                "audio_ms": 300,
                "sessions": []
            },
            "pipeline": []
        }))
        .unwrap()
    }
}
