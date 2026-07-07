use anyhow::Result;
use clap::Args;
use std::ops::ControlFlow;
use std::time::Duration;

use crate::asr::types::{LanguageMode, SessionCtx};
use crate::asr::AsrProvider;
use crate::config::diagnostics::{AsrRuntimeTarget, LlmRuntimeTarget};
use crate::ipc::protocol::{Command, Event};
use crate::platform::permissions::{
    accessibility_trusted, microphone_authorization, MicrophoneAuthorization,
};
use crate::post::llm::LlmCleanup;

const DAEMON_STATUS_TIMEOUT: Duration = Duration::from_secs(1);
const RUNTIME_CHECK_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Include explicit runtime checks for configured ASR and LLM components.
    #[arg(long)]
    pub runtime: bool,

    /// Run Apple voice-processing capture smoke test. This may prompt for microphone permission.
    #[arg(long)]
    pub apple_capture_smoke: bool,
}

pub async fn run(args: DoctorArgs) -> Result<()> {
    let mut report = DoctorReport::default();
    println!("shuo doctor");
    println!(
        "{}",
        tr(
            "cli.doctor.version",
            &[("version", env!("CARGO_PKG_VERSION").to_string())]
        )
    );
    report.record_with_step(check_config(), "cli.doctor.next_step_config");
    report.record_with_step(check_i18n(), "cli.doctor.next_step_report_issue");
    report.record_with_step(check_hotkey(), "cli.doctor.next_step_hotkey");
    report.record_with_step(
        check_microphone_input(),
        "cli.doctor.next_step_microphone_input",
    );
    report.record_with_step(check_uds().await, "cli.doctor.next_step_daemon");
    report.record_with_step(check_launchd(), "cli.doctor.next_step_launchd");
    report.record_with_step(check_install(), "cli.doctor.next_step_install");
    report.record_with_step(check_permissions(), "cli.doctor.next_step_permissions");
    if args.apple_capture_smoke {
        report.record_with_step(
            check_apple_capture_smoke().await,
            "cli.doctor.next_step_microphone_input",
        );
    }
    if args.runtime {
        report.record_with_step(check_runtime().await, "cli.doctor.next_step_runtime_config");
    } else {
        println!("runtime: {}", tr("cli.doctor.runtime_skipped", &[]));
        report.add_next_step("cli.doctor.next_step_runtime");
    }
    report.into_result()
}

#[cfg(target_os = "macos")]
async fn check_apple_capture_smoke() -> CheckStatus {
    const SMOKE_MS: u64 = 800;
    println!("voice.apple_capture_smoke: probing {SMOKE_MS}ms capture helper");
    match crate::voice::apple_source::capture_smoke(SMOKE_MS).await {
        Ok(result) => {
            println!(
                "voice.apple_capture_smoke: OK first_frame_samples={}",
                result.samples_in_first_frame
            );
            CheckStatus::Ok
        }
        Err(error) => {
            println!("voice.apple_capture_smoke: ERROR {error:#}");
            CheckStatus::Error
        }
    }
}

#[cfg(not(target_os = "macos"))]
async fn check_apple_capture_smoke() -> CheckStatus {
    println!("voice.apple_capture_smoke: ERROR Apple capture smoke is only implemented on macOS");
    CheckStatus::Error
}

fn tr(key: &str, vars: &[(&str, String)]) -> String {
    crate::i18n::tr(key, vars)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Default)]
struct DoctorReport {
    errors: usize,
    warnings: usize,
    next_steps: Vec<&'static str>,
}

impl DoctorReport {
    fn record(&mut self, status: CheckStatus) {
        match status {
            CheckStatus::Ok => {}
            CheckStatus::Warning => self.warnings += 1,
            CheckStatus::Error => self.errors += 1,
        }
    }

    fn record_with_step(&mut self, status: CheckStatus, key: &'static str) {
        self.record(status);
        if status != CheckStatus::Ok {
            self.add_next_step(key);
        }
    }

    fn add_next_step(&mut self, key: &'static str) {
        if !self.next_steps.contains(&key) {
            self.next_steps.push(key);
        }
    }

    fn into_result(self) -> Result<()> {
        self.print_summary();
        if self.errors > 0 {
            anyhow::bail!(
                "{}",
                tr(
                    "cli.doctor.blocking_issues",
                    &[("count", self.errors.to_string())]
                )
            );
        }
        Ok(())
    }

    fn print_summary(&self) {
        println!("{}", tr("cli.doctor.summary_title", &[]));
        println!(
            "{}",
            tr(
                "cli.doctor.summary_counts",
                &[
                    ("errors", self.errors.to_string()),
                    ("warnings", self.warnings.to_string())
                ]
            )
        );
        if self.next_steps.is_empty() {
            println!("{}", tr("cli.doctor.next_steps_none", &[]));
            return;
        }
        println!("{}", tr("cli.doctor.next_steps_title", &[]));
        for key in &self.next_steps {
            println!("  - {}", tr(key, &[]));
        }
    }
}

fn check_i18n() -> CheckStatus {
    let diagnostics = crate::i18n::diagnostics::diagnose_embedded();
    if diagnostics.is_empty() {
        println!("i18n.embedded: OK");
        return CheckStatus::Ok;
    }
    println!(
        "i18n.embedded: ERROR {}",
        tr(
            "cli.doctor.i18n_diagnostics",
            &[("count", diagnostics.len().to_string())]
        )
    );
    for diagnostic in diagnostics {
        println!("i18n.embedded: {diagnostic:?}");
    }
    CheckStatus::Error
}

fn check_config() -> CheckStatus {
    let report = crate::config::diagnostics::run_local();
    println!(
        "config.local: {}",
        tr(
            "cli.doctor.config_checked",
            &[
                ("count", report.files_checked.to_string()),
                ("path", report.root.display().to_string())
            ]
        )
    );
    for diagnostic in &report.diagnostics {
        println!(
            "config.{:?}: {:?} {} {}: {}",
            diagnostic.scope,
            diagnostic.severity,
            diagnostic.source.display(),
            diagnostic.path,
            diagnostic.message
        );
    }
    let mut status = if report.has_errors() {
        println!("config.local: ERROR");
        CheckStatus::Error
    } else {
        println!("config.local: OK");
        CheckStatus::Ok
    };

    let path = crate::config::default_path();
    match crate::config::load_from(&path) {
        Ok(cfg) => {
            println!("{}", tr("cli.doctor.effective_config", &[]));
            println!("  hotkey.trigger = {:?}", cfg.hotkey.trigger);
            println!("  hotkey.cancel = {:?}", cfg.hotkey.cancel);
            println!("  hotkey.resume = {:?}", cfg.hotkey.resume);
            println!("  post.timeout_ms = {}", cfg.post.timeout_ms);
            println!("  voice.stop_delay_ms = {}", cfg.voice.stop_delay_ms);
            println!("  voice.record_audio = {}", cfg.voice.record_audio);
            println!("  voice.auto_paste = {}", cfg.voice.auto_paste);
            println!(
                "  voice.preprocess.backend = {:?}",
                cfg.voice.preprocess.backend
            );
            println!("  dev.vad_trace = {}", cfg.dev.vad_trace);
            println!(
                "  dev.apple_backend_trace = {}",
                cfg.dev.apple_backend_trace
            );
            println!("  ui.language = {:?}", cfg.ui.language);
            println!("  ui.theme = {:?}", cfg.ui.theme);
            println!("  ui.theme_tui = {:?}", cfg.ui.theme_tui);
            println!("  ui.theme_overlay = {:?}", cfg.ui.theme_overlay);
            println!("  overlay.position = {:?}", cfg.overlay.position);
        }
        Err(e) => {
            println!(
                "effective config: {}",
                tr(
                    "cli.doctor.effective_config_unavailable",
                    &[("error", format!("{e:#}"))]
                )
            );
            println!(
                "hint: {}",
                tr(
                    "cli.doctor.hint_edit_path",
                    &[("path", path.display().to_string())]
                )
            );
            status = CheckStatus::Error;
        }
    }
    status
}

fn check_microphone_input() -> CheckStatus {
    match crate::voice::recorder::probe_default_input() {
        Ok(info) => {
            let name = info.name.unwrap_or_else(|| "<unknown>".to_string());
            println!(
                "microphone.input: OK {name} ({}Hz, {}ch, {:?})",
                info.sample_rate, info.channels, info.sample_format
            );
            CheckStatus::Ok
        }
        Err(e) => {
            println!("microphone.input: ERROR {e:#}");
            println!("hint: {}", tr("cli.doctor.hint_microphone_input", &[]));
            CheckStatus::Error
        }
    }
}

fn check_hotkey() -> CheckStatus {
    match crate::config::load_from(&crate::config::default_path()).and_then(|cfg| {
        crate::hotkey::Bindings::parse(&cfg.hotkey.trigger, &cfg.hotkey.cancel, &cfg.hotkey.resume)
            .map(|bindings| (cfg, bindings))
    }) {
        Ok((cfg, bindings)) => {
            let trigger = bindings
                .combo_for(crate::hotkey::HotkeyAction::Toggle)
                .map(ToString::to_string)
                .unwrap_or_else(|| "<missing>".to_string());
            let cancel = bindings
                .combo_for(crate::hotkey::HotkeyAction::Cancel)
                .map(ToString::to_string)
                .unwrap_or_else(|| "<missing>".to_string());
            let resume = bindings
                .combo_for(crate::hotkey::HotkeyAction::Resume)
                .map(ToString::to_string)
                .unwrap_or_else(|| "<missing>".to_string());
            println!(
                "hotkey: {}",
                tr(
                    "cli.doctor.hotkey_ok",
                    &[
                        ("trigger_raw", format!("{:?}", cfg.hotkey.trigger)),
                        ("trigger", trigger),
                        ("cancel_raw", format!("{:?}", cfg.hotkey.cancel)),
                        ("cancel", cancel),
                        ("resume_raw", format!("{:?}", cfg.hotkey.resume)),
                        ("resume", resume)
                    ]
                )
            );
            CheckStatus::Ok
        }
        Err(e) => {
            println!("hotkey: ERROR {e:#}");
            println!("hint: {}", tr("cli.doctor.hint_hotkey_grammar", &[]));
            CheckStatus::Error
        }
    }
}

async fn check_runtime() -> CheckStatus {
    let plan = match crate::config::diagnostics::runtime_check_plan() {
        Ok(plan) => plan,
        Err(report) => {
            println!(
                "runtime: {}",
                tr("cli.doctor.runtime_skipped_config_errors", &[])
            );
            for diagnostic in &report.diagnostics {
                println!(
                    "runtime.blocker.{:?}: {:?} {} {}: {}",
                    diagnostic.scope,
                    diagnostic.severity,
                    diagnostic.source.display(),
                    diagnostic.path,
                    diagnostic.message
                );
            }
            return CheckStatus::Error;
        }
    };
    if plan.is_empty() {
        println!(
            "runtime: {}",
            tr(
                "cli.doctor.runtime_no_profiles",
                &[("path", plan.root.display().to_string())]
            )
        );
        return CheckStatus::Warning;
    }
    let mut status = CheckStatus::Ok;
    for target in plan.asr_targets() {
        if check_asr_runtime(target).await == CheckStatus::Error {
            status = CheckStatus::Error;
        }
    }
    let llm_targets = plan.llm_targets();
    if llm_targets.is_empty() {
        println!(
            "llm.runtime: {}",
            tr("cli.doctor.llm_runtime_no_components", &[])
        );
    } else {
        for target in llm_targets {
            if check_llm_runtime(target).await == CheckStatus::Error {
                status = CheckStatus::Error;
            }
        }
    }
    status
}

async fn check_asr_runtime(target: AsrRuntimeTarget) -> CheckStatus {
    let ctx = SessionCtx {
        language: LanguageMode::Multilingual {
            hint: vec!["zh-CN".to_string(), "en-US".to_string()],
        },
        hotwords: target.hotwords.clone(),
    };
    match target.instance.kind {
        crate::config::asr::instance::AsrKind::Apple => check_apple_runtime(target, ctx).await,
        crate::config::asr::instance::AsrKind::Doubao => {
            match crate::asr::providers::doubao::DoubaoProvider::new_from_path_with_overrides(
                &target.instance.path,
                Some(&target.overrides),
            ) {
                Ok(provider) => {
                    let caps = provider.caps();
                    println!(
                        "{}",
                        asr_runtime_probe_line(target.instance.kind.as_str(), &target.profiles)
                    );
                    match provider.check_runtime(ctx).await {
                        Ok(()) => println!(
                            "asr.doubao.runtime: OK profiles=[{}] hotwords={} multilingual={}",
                            target.profiles.join(", "),
                            caps.hotwords,
                            caps.multilingual
                        ),
                        Err(e) => {
                            println!(
                                "asr.doubao.runtime: ERROR profiles=[{}] {e}",
                                target.profiles.join(", ")
                            );
                            println!(
                                "hint: {}",
                                tr(
                                    "cli.doctor.hint_doubao_runtime",
                                    &[("path", target.instance.path.display().to_string())]
                                )
                            );
                            return CheckStatus::Error;
                        }
                    }
                    CheckStatus::Ok
                }
                Err(e) => {
                    println!(
                        "asr.doubao.runtime: ERROR profiles=[{}] {e:#}",
                        target.profiles.join(", ")
                    );
                    println!(
                        "hint: {}",
                        tr(
                            "cli.doctor.hint_edit_path",
                            &[("path", target.instance.path.display().to_string())]
                        )
                    );
                    CheckStatus::Error
                }
            }
        }
        crate::config::asr::instance::AsrKind::Tencent => {
            match crate::asr::providers::tencent::TencentProvider::new_from_path_with_overrides(
                &target.instance.path,
                Some(&target.overrides),
            ) {
                Ok(provider) => {
                    let caps = provider.caps();
                    println!(
                        "{}",
                        asr_runtime_probe_line(target.instance.kind.as_str(), &target.profiles)
                    );
                    match provider.check_runtime(ctx).await {
                        Ok(()) => println!(
                            "asr.tencent.runtime: OK profiles=[{}] hotwords={} multilingual={}",
                            target.profiles.join(", "),
                            caps.hotwords,
                            caps.multilingual
                        ),
                        Err(e) => {
                            println!(
                                "asr.tencent.runtime: ERROR profiles=[{}] {e}",
                                target.profiles.join(", ")
                            );
                            println!(
                                "hint: {}",
                                tr(
                                    "cli.doctor.hint_edit_path",
                                    &[("path", target.instance.path.display().to_string())]
                                )
                            );
                            return CheckStatus::Error;
                        }
                    }
                    CheckStatus::Ok
                }
                Err(e) => {
                    println!(
                        "asr.tencent.runtime: ERROR profiles=[{}] {e:#}",
                        target.profiles.join(", ")
                    );
                    println!(
                        "hint: {}",
                        tr(
                            "cli.doctor.hint_edit_path",
                            &[("path", target.instance.path.display().to_string())]
                        )
                    );
                    CheckStatus::Error
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
async fn check_apple_runtime(target: AsrRuntimeTarget, ctx: SessionCtx) -> CheckStatus {
    match crate::asr::providers::apple::AppleProvider::new_from_path_with_overrides(
        &target.instance.path,
        Some(&target.overrides),
    ) {
        Ok(provider) => {
            let caps = provider.caps();
            println!(
                "{}",
                asr_runtime_probe_line(target.instance.kind.as_str(), &target.profiles)
            );
            if let Some(notice) = provider.runtime_check_notice() {
                println!("{}", notice.line);
            }
            match provider.check_runtime(ctx).await {
                Ok(()) => println!(
                    "asr.apple.runtime: OK profiles=[{}] hotwords={} multilingual={}",
                    target.profiles.join(", "),
                    caps.hotwords,
                    caps.multilingual
                ),
                Err(e) => {
                    println!(
                        "asr.apple.runtime: ERROR profiles=[{}] {e}",
                        target.profiles.join(", ")
                    );
                    println!(
                        "hint: {}",
                        tr(
                            "cli.doctor.hint_apple_runtime",
                            &[("path", target.instance.path.display().to_string())]
                        )
                    );
                    return CheckStatus::Error;
                }
            }
            CheckStatus::Ok
        }
        Err(e) => {
            println!(
                "asr.apple.runtime: ERROR profiles=[{}] {e:#}",
                target.profiles.join(", ")
            );
            println!(
                "hint: {}",
                tr(
                    "cli.doctor.hint_edit_path",
                    &[("path", target.instance.path.display().to_string())]
                )
            );
            CheckStatus::Error
        }
    }
}

#[cfg(not(target_os = "macos"))]
async fn check_apple_runtime(target: AsrRuntimeTarget, _ctx: SessionCtx) -> CheckStatus {
    println!(
        "asr.{}.runtime: ERROR profiles=[{}] Apple ASR provider is only implemented on macOS",
        target.instance.kind.as_str(),
        target.profiles.join(", ")
    );
    CheckStatus::Error
}

fn asr_runtime_probe_line(provider: &str, profiles: &[String]) -> String {
    format!(
        "asr.{provider}.runtime: {}",
        tr(
            "cli.doctor.asr_runtime_probing",
            &[("profiles", profiles.join(", "))]
        )
    )
}

async fn check_llm_runtime(target: LlmRuntimeTarget) -> CheckStatus {
    let dir = crate::config::post::PostDir {
        dir: crate::config::post::default_dir(),
    };
    let processor_cfg =
        match crate::config::post::load_llm_config(&target.id, &dir, &target.overrides) {
            Ok(cfg) => cfg,
            Err(e) => {
                println!(
                    "llm.runtime: ERROR profiles=[{}] component={} {e:#}",
                    target.profiles.join(", "),
                    target.id
                );
                return CheckStatus::Error;
            }
        };
    let cfg = match crate::post::build_llm_cleanup_config(processor_cfg) {
        Ok(cfg) => cfg,
        Err(e) => {
            println!(
                "llm.runtime: ERROR profiles=[{}] component={} {e:#}",
                target.profiles.join(", "),
                target.id
            );
            return CheckStatus::Error;
        }
    };
    let provider = cfg.provider_name.clone();
    let checker = LlmCleanup::new(cfg);
    match tokio::time::timeout(RUNTIME_CHECK_TIMEOUT, checker.check_runtime()).await {
        Err(_) => {
            println!(
                "llm.runtime: ERROR {}",
                tr(
                    "cli.doctor.llm_runtime_timeout",
                    &[
                        ("profiles", target.profiles.join(", ")),
                        ("component", target.id.clone()),
                        ("provider", provider.clone()),
                        ("seconds", RUNTIME_CHECK_TIMEOUT.as_secs().to_string())
                    ]
                )
            );
            println!("hint: {}", tr("cli.doctor.hint_llm_runtime", &[]));
            CheckStatus::Error
        }
        Ok(Ok(())) => {
            println!(
                "llm.runtime: OK profiles=[{}] component={} provider={}",
                target.profiles.join(", "),
                target.id,
                provider
            );
            CheckStatus::Ok
        }
        Ok(Err(e)) => {
            println!(
                "llm.runtime: ERROR profiles=[{}] component={} provider={} {e}",
                target.profiles.join(", "),
                target.id,
                provider
            );
            println!("hint: {}", tr("cli.doctor.hint_llm_runtime", &[]));
            CheckStatus::Error
        }
    }
}

async fn check_uds() -> CheckStatus {
    match tokio::time::timeout(DAEMON_STATUS_TIMEOUT, query_daemon_status()).await {
        Ok(Ok(Some(status))) => {
            println!("{status}");
            CheckStatus::Ok
        }
        Ok(Ok(None)) => {
            println!(
                "daemon: {}",
                tr(
                    "cli.doctor.daemon_not_running",
                    &[(
                        "socket",
                        crate::ipc::server::default_socket_path()
                            .display()
                            .to_string()
                    )]
                )
            );
            CheckStatus::Warning
        }
        Ok(Err(e)) => {
            println!("daemon: ERROR {e:#}");
            CheckStatus::Error
        }
        Err(_) => {
            println!(
                "daemon: ERROR {}",
                tr(
                    "cli.doctor.daemon_status_timeout",
                    &[("seconds", DAEMON_STATUS_TIMEOUT.as_secs().to_string())]
                )
            );
            CheckStatus::Error
        }
    }
}

async fn query_daemon_status() -> Result<Option<String>> {
    let socket = crate::ipc::server::default_socket_path();
    let mut client = match crate::ipc::client::IpcClient::connect(&socket).await {
        Ok(client) => client,
        Err(error) if crate::ipc::client::connect_error_is_absent(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    client.send(&Command::DaemonStatus).await?;
    client
        .recv_until(|event| match event {
            Event::DaemonStatus {
                pid,
                uptime_ms,
                state,
                recording_id,
            } => Ok(ControlFlow::Break(format!(
                "daemon: OK {}",
                tr(
                    "cli.doctor.daemon_ok",
                    &[
                        ("pid", pid.to_string()),
                        ("uptime", format_duration(uptime_ms)),
                        ("state", format!("{state:?}")),
                        (
                            "recording",
                            recording_id.as_deref().unwrap_or("-").to_string()
                        )
                    ]
                )
            ))),
            Event::Error { kind, msg, .. } => anyhow::bail!("{kind}: {msg}"),
            _ => Ok(ControlFlow::Continue(())),
        })
        .await
}

fn check_launchd() -> CheckStatus {
    match crate::cli::service::launchd_status() {
        crate::cli::service::LaunchdStatus::Installed(path) => {
            println!(
                "launchd.plist: {}",
                tr(
                    "cli.service.plist_installed",
                    &[("path", path.display().to_string())]
                )
            );
            CheckStatus::Ok
        }
        crate::cli::service::LaunchdStatus::NotInstalled(path) => {
            println!(
                "launchd.plist: {}",
                tr(
                    "cli.service.plist_not_installed",
                    &[("path", path.display().to_string())]
                )
            );
            CheckStatus::Warning
        }
        #[cfg(not(target_os = "macos"))]
        crate::cli::service::LaunchdStatus::Unsupported => {
            println!(
                "launchd.plist: {}",
                tr("cli.service.management_unsupported", &[])
            );
            CheckStatus::Warning
        }
    }
}

fn check_install() -> CheckStatus {
    let preferred = match crate::install::InstallLayout::preferred_bin() {
        Ok(path) => path,
        Err(e) => {
            println!("install.paths: ERROR {e:#}");
            return CheckStatus::Error;
        }
    };
    let current = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            println!("install.paths: ERROR {e:#}");
            return CheckStatus::Error;
        }
    };
    println!(
        "install.paths: current={} preferred={}",
        current.display(),
        preferred.display()
    );
    let plist = crate::cli::service::plist_program();
    let path_first = crate::install::path_first_binary();
    let findings = crate::install::diagnose_drift(
        &current,
        &preferred,
        plist.as_deref(),
        path_first.as_deref(),
    );
    if findings.is_empty() {
        println!("install.paths: OK");
        return CheckStatus::Ok;
    }
    for finding in &findings {
        println!("install.paths: {}", crate::install::render_drift(finding));
    }
    CheckStatus::Warning
}

fn check_permissions() -> CheckStatus {
    let mut status = CheckStatus::Ok;
    if accessibility_trusted() {
        println!("permissions.accessibility: OK");
    } else {
        println!(
            "permissions.accessibility: {}",
            tr("cli.doctor.permission_accessibility_missing", &[])
        );
        println!(
            "hint: {}",
            tr("cli.doctor.hint_permission_accessibility", &[])
        );
        status = CheckStatus::Error;
    }
    match microphone_authorization() {
        Some(MicrophoneAuthorization::Authorized) => println!("permissions.microphone: OK"),
        Some(MicrophoneAuthorization::NotDetermined) => {
            println!(
                "permissions.microphone: {}",
                tr("cli.doctor.permission_microphone_not_determined", &[])
            );
            println!(
                "hint: {}",
                tr("cli.doctor.hint_permission_microphone_prompt", &[])
            );
            status = CheckStatus::Error;
        }
        Some(MicrophoneAuthorization::Denied) => {
            println!(
                "permissions.microphone: {}",
                tr("cli.doctor.permission_microphone_denied", &[])
            );
            println!(
                "hint: {}",
                tr("cli.doctor.hint_permission_microphone_grant", &[])
            );
            status = CheckStatus::Error;
        }
        Some(MicrophoneAuthorization::Restricted) => {
            println!(
                "permissions.microphone: {}",
                tr("cli.doctor.permission_microphone_restricted", &[])
            );
            status = CheckStatus::Error;
        }
        None => {
            println!(
                "permissions.microphone: {}",
                tr("cli.doctor.permission_microphone_unknown", &[])
            );
            println!(
                "hint: {}",
                tr("cli.doctor.hint_permission_microphone_grant", &[])
            );
            status = CheckStatus::Error;
        }
    }
    status
}

fn format_duration(ms: u64) -> String {
    let secs = ms / 1000;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h{m}m{s}s")
    } else if m > 0 {
        format!("{m}m{s}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[test]
    fn asr_runtime_probe_line_mentions_minimal_session() {
        let profiles = vec!["default".to_string(), "coding".to_string()];

        let line = super::asr_runtime_probe_line("doubao", &profiles);

        assert_eq!(
            line,
            "asr.doubao.runtime: probing minimal session for profiles=[default, coding]"
        );
    }

    #[test]
    fn doctor_report_fails_when_required_check_has_error() {
        let mut report = super::DoctorReport::default();

        report.record(super::CheckStatus::Ok);
        report.record(super::CheckStatus::Error);

        assert!(report.into_result().is_err());
    }

    #[test]
    fn doctor_report_counts_warnings_and_deduplicates_next_steps() {
        let mut report = super::DoctorReport::default();

        report.record(super::CheckStatus::Warning);
        report.add_next_step("cli.doctor.next_step_runtime");
        report.add_next_step("cli.doctor.next_step_runtime");

        assert_eq!(report.errors, 0);
        assert_eq!(report.warnings, 1);
        assert_eq!(report.next_steps, ["cli.doctor.next_step_runtime"]);
    }

    #[test]
    fn doctor_timeouts_match_cli_contract() {
        assert_eq!(super::DAEMON_STATUS_TIMEOUT, Duration::from_secs(1));
        assert_eq!(super::RUNTIME_CHECK_TIMEOUT, Duration::from_secs(15));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn uds_check_runs_inside_cli_runtime() {
        let _ = super::check_uds().await;
    }
}
