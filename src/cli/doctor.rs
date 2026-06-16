use anyhow::{Context, Result};
use clap::Args;
use objc2::msg_send;
use objc2::runtime::AnyClass;
use objc2_foundation::ns_string;

use crate::asr::AsrProvider;
use crate::ipc::protocol::{Command, Event};

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Reserved for future network checks. M5 never sends PCM.
    #[arg(long)]
    pub full: bool,
}

pub fn run(args: DoctorArgs) -> Result<()> {
    println!("shuo doctor");
    println!("version: {}", env!("CARGO_PKG_VERSION"));
    if args.full {
        println!("asr.full: skipped in M5 (no PCM is sent by doctor)");
    }

    check_config();
    check_hotkey();
    check_microphone_input();
    check_asr_provider();
    check_uds();
    check_launchd();
    check_permissions();
    Ok(())
}

fn check_config() {
    let path = crate::config::default_path();
    match crate::config::load_from(&path) {
        Ok(cfg) => {
            println!("config: OK {}", path.display());
            println!("effective config:");
            println!("  hotkey.trigger = {:?}", cfg.hotkey.trigger);
            println!("  post.timeout_ms = {}", cfg.post.timeout_ms);
            println!("  voice.stop_delay_ms = {}", cfg.voice.stop_delay_ms);
            println!("  voice.record_audio = {}", cfg.voice.record_audio);
            println!("  voice.vad_trace = {}", cfg.voice.vad_trace);
            println!("  voice.auto_paste = {}", cfg.voice.auto_paste);
            println!("  ui.language = {:?}", cfg.ui.language);
            println!("  overlay.position = {:?}", cfg.overlay.position);
            println!("  overlay.glass_variant = {}", cfg.overlay.glass_variant);
        }
        Err(e) => {
            println!("config: ERROR {e:#}");
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
    match crate::config::load_from(&crate::config::default_path())
        .and_then(|cfg| crate::hotkey::parse::parse(&cfg.hotkey.trigger).map(|combo| (cfg, combo)))
    {
        Ok((cfg, combo)) => println!("hotkey: OK {:?} -> {}", cfg.hotkey.trigger, combo),
        Err(e) => {
            println!("hotkey: ERROR {e:#}");
            println!("hint: see docs/DESIGN.md §2.4 for the supported hotkey grammar");
        }
    }
}

fn check_asr_provider() {
    if crate::config::load_from(&crate::config::default_path()).is_err() {
        return;
    }
    let apps_dir = crate::app_profile::default_dir();
    let profile = match crate::app_profile::load_for_app(&apps_dir, None) {
        Ok(profile) => profile,
        Err(e) => {
            println!("asr: ERROR default app profile unreadable: {e:#}");
            println!("hint: edit {}", apps_dir.join("default.toml").display());
            return;
        }
    };
    match profile.asr.provider.as_str() {
        "doubao" => match crate::asr::providers::doubao::DoubaoProvider::new_with_overrides(Some(
            &profile.asr.overrides,
        )) {
            Ok(provider) => {
                let caps = provider.caps();
                println!(
                    "asr.doubao: OK config readable (profile={:?}, hotwords={}, overrides={}, multilingual={})",
                    profile.name,
                    caps.hotwords,
                    profile.asr.overrides.len(),
                    caps.multilingual
                );
                println!("asr.doubao: network/auth handshake not run; no PCM sent");
            }
            Err(e) => {
                println!("asr.doubao: ERROR {e:#}");
                println!(
                    "hint: edit {}",
                    crate::asr::providers::doubao::config_path().display()
                );
            }
        },
        "apple" => {
            match crate::asr::providers::apple::AppleProvider::new_with_overrides(Some(
                &profile.asr.overrides,
            )) {
                Ok(provider) => {
                    let caps = provider.caps();
                    println!(
                        "asr.apple: OK config readable (profile={:?}, hotwords={}, overrides={}, multilingual={})",
                        profile.name,
                        caps.hotwords,
                        profile.asr.overrides.len(),
                        caps.multilingual
                    );
                    println!("asr.apple: SpeechAnalyzer runtime check not run; no PCM sent");
                }
                Err(e) => {
                    println!("asr.apple: ERROR {e:#}");
                    println!(
                        "hint: edit {}",
                        crate::asr::providers::apple::config_path().display()
                    );
                }
            }
        }
        other => {
            println!("asr: ERROR unsupported provider {other:?}");
            println!("hint: use app profile [asr] provider = \"doubao\" or \"apple\"");
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
