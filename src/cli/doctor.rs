use anyhow::{Context, Result};
use clap::Args;
use objc2::msg_send;
use objc2::runtime::AnyClass;
use objc2_foundation::ns_string;

use crate::asr::types::{LanguageMode, SessionCtx};
use crate::asr::AsrProvider;
use crate::config::diagnostics::{AsrRuntimeTarget, LlmRuntimeTarget};
use crate::ipc::protocol::{Command, Event};
use crate::post::llm::LlmCleanup;

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Include explicit runtime checks for configured ASR and LLM components.
    #[arg(long)]
    pub runtime: bool,
}

pub fn run(args: DoctorArgs) -> Result<()> {
    println!("shuo doctor");
    println!("version: {}", env!("CARGO_PKG_VERSION"));
    check_config();
    check_i18n();
    check_hotkey();
    check_microphone_input();
    check_uds();
    check_launchd();
    check_permissions();
    if args.runtime {
        check_runtime();
    } else {
        println!("runtime: skipped (run `shuo doctor --runtime` to test configured ASR/LLM runtime paths)");
    }
    Ok(())
}

fn check_i18n() {
    let diagnostics = crate::i18n::diagnostics::diagnose_embedded();
    if diagnostics.is_empty() {
        println!("i18n.embedded: OK");
        return;
    }
    println!("i18n.embedded: ERROR {} diagnostics", diagnostics.len());
    for diagnostic in diagnostics {
        println!("i18n.embedded: {diagnostic:?}");
    }
}

fn check_config() {
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
    if report.has_errors() {
        println!("config.local: ERROR");
    } else {
        println!("config.local: OK");
    }

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
        }
    }
}

fn check_microphone_input() {
    match crate::voice::recorder::probe_default_input() {
        Ok(info) => {
            let name = info.name.unwrap_or_else(|| "<unknown>".to_string());
            println!(
                "microphone.input: OK {name} ({}Hz, {}ch, {:?})",
                info.sample_rate, info.channels, info.sample_format
            );
        }
        Err(e) => {
            println!("microphone.input: ERROR {e:#}");
            println!("hint: connect or select a default microphone in System Settings → Sound");
        }
    }
}

fn check_hotkey() {
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
        }
        Err(e) => {
            println!("hotkey: ERROR {e:#}");
            println!("hint: see docs/DESIGN.md §2.4 for the supported hotkey grammar");
        }
    }
}

fn check_runtime() {
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
            return;
        }
    };
    if plan.is_empty() {
        println!("runtime: no profiles found under {}", plan.root.display());
        return;
    }
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create doctor runtime")
    {
        Ok(rt) => rt,
        Err(e) => {
            println!("runtime: ERROR {e:#}");
            return;
        }
    };
    rt.block_on(async {
        for target in plan.asr_targets() {
            check_asr_runtime(target).await;
        }
        let llm_targets = plan.llm_targets();
        if llm_targets.is_empty() {
            println!("llm.runtime: no referenced LLM components");
        } else {
            for target in llm_targets {
                check_llm_runtime(target).await;
            }
        }
    });
}

async fn check_asr_runtime(target: AsrRuntimeTarget) {
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
                    }
                }
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
            }
        },
        "doubao" => match crate::asr::providers::doubao::DoubaoProvider::new_with_overrides(Some(
            &target.overrides,
        )) {
            Ok(provider) => {
                let caps = provider.caps();
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
                    }
                }
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
            }
        },
        other => {
            println!(
                "asr.{other}.runtime: ERROR profiles=[{}] unsupported provider",
                target.profiles.join(", ")
            );
        }
    }
}

async fn check_llm_runtime(target: LlmRuntimeTarget) {
    let dirs = crate::config::post::PostDirs {
        rule: crate::config::post::default_dir().join("rule"),
        llm: crate::config::post::default_dir().join("llm"),
    };
    match crate::config::post::load_llm_config(&target.id, &dirs, &target.overrides) {
        Ok(cfg) => {
            let provider = cfg.provider_name.clone();
            let checker = LlmCleanup::new(cfg);
            match checker.check_runtime().await {
                Ok(()) => println!(
                    "llm.runtime: OK profiles=[{}] component={} provider={}",
                    target.profiles.join(", "),
                    target.id,
                    provider
                ),
                Err(e) => {
                    println!(
                        "llm.runtime: ERROR profiles=[{}] component={} provider={} {e}",
                        target.profiles.join(", "),
                        target.id,
                        provider
                    );
                    println!("hint: edit post/llm component or profile [post.llm] override");
                }
            }
        }
        Err(e) => {
            println!(
                "llm.runtime: ERROR profiles=[{}] component={} {e:#}",
                target.profiles.join(", "),
                target.id
            );
        }
    }
}

fn check_uds() {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create doctor runtime")
    {
        Ok(rt) => rt,
        Err(e) => {
            println!("daemon: ERROR {e:#}");
            return;
        }
    };
    match rt.block_on(query_daemon_status()) {
        Ok(Some(status)) => println!("{status}"),
        Ok(None) => println!(
            "daemon: not running ({} not reachable)",
            crate::ipc::server::default_socket_path().display()
        ),
        Err(e) => println!("daemon: ERROR {e:#}"),
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

fn check_launchd() {
    let path = crate::cli::service::plist_path();
    if path.exists() {
        println!("launchd.plist: installed {}", path.display());
    } else {
        println!("launchd.plist: not installed {}", path.display());
    }
}

fn check_permissions() {
    if accessibility_trusted() {
        println!("permissions.accessibility: OK");
    } else {
        println!("permissions.accessibility: missing or not granted");
        println!("hint: grant the terminal app that starts shuo in System Settings > Privacy & Security > Accessibility");
    }
    match microphone_authorization() {
        Some(MicrophoneAuthorization::Authorized) => println!("permissions.microphone: OK"),
        Some(MicrophoneAuthorization::NotDetermined) => {
            println!("permissions.microphone: not determined");
            println!("hint: start recording once from the terminal app that runs shuo to trigger the system prompt");
        }
        Some(MicrophoneAuthorization::Denied) => {
            println!("permissions.microphone: denied");
            println!("hint: grant the terminal app that starts shuo in System Settings > Privacy & Security > Microphone");
        }
        Some(MicrophoneAuthorization::Restricted) => {
            println!("permissions.microphone: restricted by system policy");
        }
        None => {
            println!("permissions.microphone: unknown");
            println!("hint: grant the terminal app that starts shuo in System Settings > Privacy & Security > Microphone");
        }
    }
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
