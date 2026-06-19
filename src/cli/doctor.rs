use anyhow::{Context, Result};
use clap::Args;
use objc2::msg_send;
use objc2::runtime::AnyClass;
use objc2_foundation::ns_string;
use std::time::Duration;

use crate::asr::types::{LanguageMode, SessionCtx};
use crate::asr::AsrProvider;
use crate::config::diagnostics::{AsrRuntimeTarget, LlmRuntimeTarget};
use crate::ipc::protocol::{Command, Event};
use crate::post::llm::LlmCleanup;

const DAEMON_STATUS_TIMEOUT: Duration = Duration::from_secs(1);
const RUNTIME_CHECK_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Include explicit runtime checks for configured ASR and LLM components.
    #[arg(long)]
    pub runtime: bool,
}

pub fn run(args: DoctorArgs) -> Result<()> {
    let mut report = DoctorReport::default();
    println!("shuo doctor");
    println!("version: {}", env!("CARGO_PKG_VERSION"));
    report.record(check_config());
    report.record(check_i18n());
    report.record(check_hotkey());
    report.record(check_microphone_input());
    report.record(check_uds());
    report.record(check_launchd());
    report.record(check_permissions());
    if args.runtime {
        report.record(check_runtime());
    } else {
        println!("runtime: skipped (run `shuo doctor --runtime` to test configured ASR/LLM runtime paths)");
    }
    report.into_result()
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
}

impl DoctorReport {
    fn record(&mut self, status: CheckStatus) {
        if status == CheckStatus::Error {
            self.errors += 1;
        }
    }

    fn into_result(self) -> Result<()> {
        if self.errors > 0 {
            anyhow::bail!("doctor found {} blocking issue(s)", self.errors);
        }
        Ok(())
    }
}

fn check_i18n() -> CheckStatus {
    let diagnostics = crate::i18n::diagnostics::diagnose_embedded();
    if diagnostics.is_empty() {
        println!("i18n.embedded: OK");
        return CheckStatus::Ok;
    }
    println!("i18n.embedded: ERROR {} diagnostics", diagnostics.len());
    for diagnostic in diagnostics {
        println!("i18n.embedded: {diagnostic:?}");
    }
    CheckStatus::Error
}

fn check_config() -> CheckStatus {
    let report = crate::config::diagnostics::run_local();
    println!(
        "config.local: checked {} files under {}",
        report.files_checked,
        report.root.display()
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
            println!("effective config:");
            println!("  hotkey.trigger = {:?}", cfg.hotkey.trigger);
            println!("  hotkey.cancel = {:?}", cfg.hotkey.cancel);
            println!("  post.timeout_ms = {}", cfg.post.timeout_ms);
            println!("  voice.stop_delay_ms = {}", cfg.voice.stop_delay_ms);
            println!("  voice.record_audio = {}", cfg.voice.record_audio);
            println!("  voice.auto_paste = {}", cfg.voice.auto_paste);
            println!("  dev.vad_trace = {}", cfg.dev.vad_trace);
            println!("  ui.language = {:?}", cfg.ui.language);
            println!("  ui.theme = {:?}", cfg.ui.theme);
            println!("  ui.theme_tui = {:?}", cfg.ui.theme_tui);
            println!("  ui.theme_overlay = {:?}", cfg.ui.theme_overlay);
            println!("  overlay.position = {:?}", cfg.overlay.position);
        }
        Err(e) => {
            println!("effective config: unavailable ({e:#})");
            println!("hint: edit {}", path.display());
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
            println!("hint: connect or select a default microphone in System Settings → Sound");
            CheckStatus::Error
        }
    }
}

fn check_hotkey() -> CheckStatus {
    match crate::config::load_from(&crate::config::default_path()).and_then(|cfg| {
        crate::hotkey::Bindings::parse(&cfg.hotkey.trigger, &cfg.hotkey.cancel)
            .map(|bindings| (cfg, bindings))
    }) {
        Ok((cfg, bindings)) => {
            let trigger = bindings
                .combo_for(crate::hotkey::HotkeyAction::ToggleRecord)
                .map(ToString::to_string)
                .unwrap_or_else(|| "<missing>".to_string());
            let cancel = bindings
                .combo_for(crate::hotkey::HotkeyAction::CancelRecord)
                .map(ToString::to_string)
                .unwrap_or_else(|| "<missing>".to_string());
            println!(
                "hotkey: OK trigger {:?} -> {}, cancel {:?} -> {}",
                cfg.hotkey.trigger, trigger, cfg.hotkey.cancel, cancel
            );
            CheckStatus::Ok
        }
        Err(e) => {
            println!("hotkey: ERROR {e:#}");
            println!("hint: see docs/DESIGN.md §2.4 for the supported hotkey grammar");
            CheckStatus::Error
        }
    }
}

fn check_runtime() -> CheckStatus {
    let plan = match crate::config::diagnostics::runtime_check_plan() {
        Ok(plan) => plan,
        Err(report) => {
            println!("runtime: skipped because local config diagnostics have errors");
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
        println!("runtime: no profiles found under {}", plan.root.display());
        return CheckStatus::Warning;
    }
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create doctor runtime")
    {
        Ok(rt) => rt,
        Err(e) => {
            println!("runtime: ERROR {e:#}");
            return CheckStatus::Error;
        }
    };
    rt.block_on(async {
        let mut status = CheckStatus::Ok;
        for target in plan.asr_targets() {
            if check_asr_runtime(target).await == CheckStatus::Error {
                status = CheckStatus::Error;
            }
        }
        let llm_targets = plan.llm_targets();
        if llm_targets.is_empty() {
            println!("llm.runtime: no referenced LLM components");
        } else {
            for target in llm_targets {
                if check_llm_runtime(target).await == CheckStatus::Error {
                    status = CheckStatus::Error;
                }
            }
        }
        status
    })
}

async fn check_asr_runtime(target: AsrRuntimeTarget) -> CheckStatus {
    let ctx = SessionCtx {
        language: LanguageMode::Multilingual {
            hint: vec!["zh-CN".to_string(), "en-US".to_string()],
        },
        hotwords: target.hotwords.clone(),
    };
    match target.provider.as_str() {
        "apple" => match crate::asr::providers::apple::AppleProvider::new_with_overrides(Some(
            &target.overrides,
        )) {
            Ok(provider) => {
                let caps = provider.caps();
                println!("{}", asr_runtime_probe_line("apple", &target.profiles));
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
                            "hint: edit {} or verify macOS SpeechAnalyzer availability",
                            crate::asr::providers::apple::config_path().display()
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
                    "hint: edit {}",
                    crate::asr::providers::apple::config_path().display()
                );
                CheckStatus::Error
            }
        },
        "doubao" => match crate::asr::providers::doubao::DoubaoProvider::new_with_overrides(Some(
            &target.overrides,
        )) {
            Ok(provider) => {
                let caps = provider.caps();
                println!("{}", asr_runtime_probe_line("doubao", &target.profiles));
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
                            "hint: edit {} and verify app_key/access_key",
                            crate::asr::providers::doubao::config_path().display()
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
                    "hint: edit {}",
                    crate::asr::providers::doubao::config_path().display()
                );
                CheckStatus::Error
            }
        },
        other => {
            println!(
                "asr.{other}.runtime: ERROR profiles=[{}] unsupported provider",
                target.profiles.join(", ")
            );
            CheckStatus::Error
        }
    }
}

fn asr_runtime_probe_line(provider: &str, profiles: &[String]) -> String {
    format!(
        "asr.{provider}.runtime: probing minimal session for profiles=[{}]",
        profiles.join(", ")
    )
}

async fn check_llm_runtime(target: LlmRuntimeTarget) -> CheckStatus {
    let dirs = crate::config::post::PostDirs {
        rule: crate::config::post::default_dir().join("rule"),
        llm: crate::config::post::default_dir().join("llm"),
    };
    let cfg = match crate::config::post::load_llm_config(&target.id, &dirs, &target.overrides) {
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
                "llm.runtime: ERROR profiles=[{}] component={} provider={} timed out after {}s",
                target.profiles.join(", "),
                target.id,
                provider,
                RUNTIME_CHECK_TIMEOUT.as_secs()
            );
            println!("hint: edit post/llm component or profile [post.llm] override");
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
            println!("hint: edit post/llm component or profile [post.llm] override");
            CheckStatus::Error
        }
    }
}

fn check_uds() -> CheckStatus {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create doctor runtime")
    {
        Ok(rt) => rt,
        Err(e) => {
            println!("daemon: ERROR {e:#}");
            return CheckStatus::Error;
        }
    };
    match rt.block_on(tokio::time::timeout(
        DAEMON_STATUS_TIMEOUT,
        query_daemon_status(),
    )) {
        Ok(Ok(Some(status))) => {
            println!("{status}");
            CheckStatus::Ok
        }
        Ok(Ok(None)) => {
            println!(
                "daemon: not running ({} not reachable)",
                crate::ipc::server::default_socket_path().display()
            );
            CheckStatus::Warning
        }
        Ok(Err(e)) => {
            println!("daemon: ERROR {e:#}");
            CheckStatus::Error
        }
        Err(_) => {
            println!(
                "daemon: ERROR status query timed out after {}s",
                DAEMON_STATUS_TIMEOUT.as_secs()
            );
            CheckStatus::Error
        }
    }
}

async fn query_daemon_status() -> Result<Option<String>> {
    let socket = crate::ipc::server::default_socket_path();
    let mut client = match crate::ipc::client::IpcClient::connect(&socket).await {
        Ok(client) => client,
        Err(_) => return Ok(None),
    };
    client.send(&Command::DaemonStatus).await?;
    while let Some(event) = client.recv().await? {
        match event {
            Event::DaemonStatus {
                pid,
                uptime_ms,
                state,
                recording_id,
            } => {
                return Ok(Some(format!(
                    "daemon: OK pid={pid} uptime={} state={state:?} recording={}",
                    format_duration(uptime_ms),
                    recording_id.as_deref().unwrap_or("-")
                )));
            }
            Event::Error { kind, msg, .. } => anyhow::bail!("{kind}: {msg}"),
            _ => {}
        }
    }
    Ok(None)
}

fn check_launchd() -> CheckStatus {
    let path = crate::cli::service::plist_path();
    if path.exists() {
        println!("launchd.plist: installed {}", path.display());
        CheckStatus::Ok
    } else {
        println!("launchd.plist: not installed {}", path.display());
        CheckStatus::Warning
    }
}

fn check_permissions() -> CheckStatus {
    let mut status = CheckStatus::Ok;
    if accessibility_trusted() {
        println!("permissions.accessibility: OK");
    } else {
        println!("permissions.accessibility: missing or not granted");
        println!("hint: grant the terminal app that starts shuo in System Settings > Privacy & Security > Accessibility");
        status = CheckStatus::Error;
    }
    match microphone_authorization() {
        Some(MicrophoneAuthorization::Authorized) => println!("permissions.microphone: OK"),
        Some(MicrophoneAuthorization::NotDetermined) => {
            println!("permissions.microphone: not determined");
            println!("hint: start recording once from the terminal app that runs shuo to trigger the system prompt");
            status = CheckStatus::Error;
        }
        Some(MicrophoneAuthorization::Denied) => {
            println!("permissions.microphone: denied");
            println!("hint: grant the terminal app that starts shuo in System Settings > Privacy & Security > Microphone");
            status = CheckStatus::Error;
        }
        Some(MicrophoneAuthorization::Restricted) => {
            println!("permissions.microphone: restricted by system policy");
            status = CheckStatus::Error;
        }
        None => {
            println!("permissions.microphone: unknown");
            println!("hint: grant the terminal app that starts shuo in System Settings > Privacy & Security > Microphone");
            status = CheckStatus::Error;
        }
    }
    status
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MicrophoneAuthorization {
    NotDetermined,
    Restricted,
    Denied,
    Authorized,
}

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {}

fn microphone_authorization() -> Option<MicrophoneAuthorization> {
    let class = AnyClass::get(c"AVCaptureDevice")?;
    let status: isize =
        unsafe { msg_send![class, authorizationStatusForMediaType: ns_string!("soun")] };
    match status {
        0 => Some(MicrophoneAuthorization::NotDetermined),
        1 => Some(MicrophoneAuthorization::Restricted),
        2 => Some(MicrophoneAuthorization::Denied),
        3 => Some(MicrophoneAuthorization::Authorized),
        _ => None,
    }
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
    fn doctor_timeouts_match_cli_contract() {
        assert_eq!(super::DAEMON_STATUS_TIMEOUT, Duration::from_secs(1));
        assert_eq!(super::RUNTIME_CHECK_TIMEOUT, Duration::from_secs(15));
    }
}
