pub mod app;
pub mod completions;
pub mod config_template;
pub mod doctor;
pub mod service;

use anyhow::{Context, Result};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(name = "shuo", version, about = "macOS voice input assistant")]
pub struct Cli {
    /// Run the long-lived daemon process.
    #[arg(long, hide = true)]
    pub daemon: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Doctor(doctor::DoctorArgs),
    /// Generate reference config templates from the built-in registry.
    ConfigTemplate(config_template::ConfigTemplateArgs),
    /// Generate shell completion scripts.
    Completions(completions::CompletionsArgs),
    #[command(subcommand)]
    Service(service::ServiceCommand),
    #[command(subcommand, hide = true)]
    Diagnostics(DiagnosticsCommand),
    Update(app::UpdateArgs),
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum DiagnosticsCommand {
    /// Initialize Silero VAD and run a one-frame silence smoke.
    SileroVad,
    /// Decode an audio file and summarize Silero VAD probabilities.
    SileroVadFile { path: PathBuf },
}

pub fn parse() -> Cli {
    init_i18n_for_cli();
    let mut matches = localized_command().get_matches();
    Cli::from_arg_matches_mut(&mut matches).unwrap_or_else(|e| e.exit())
}

pub fn run_command(command: Command) -> Result<()> {
    init_i18n_for_cli();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create CLI runtime")?;
    runtime.block_on(dispatch(command))
}

async fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Doctor(args) => doctor::run(args).await,
        Command::ConfigTemplate(args) => config_template::run(args),
        Command::Completions(args) => completions::run(args),
        Command::Service(command) => service::run(command).await,
        Command::Diagnostics(command) => run_diagnostics(command),
        Command::Update(args) => app::update(args).await,
        Command::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

fn run_diagnostics(command: DiagnosticsCommand) -> Result<()> {
    match command {
        DiagnosticsCommand::SileroVad => {
            let mut vad =
                crate::voice::silero::SileroVad::new(crate::voice::silero::SileroConfig {
                    threshold: 0.5,
                })?;
            let frames = vad.accept(&[0i16; crate::voice::silero::SileroConfig::frame_samples()]);
            let frame = frames
                .first()
                .context("Silero VAD did not emit a frame for one chunk")?;
            println!(
                "silero-vad: OK frame={:?} probability={:.6}",
                frame.frame, frame.probability
            );
            Ok(())
        }
        DiagnosticsCommand::SileroVadFile { path } => run_silero_vad_file(&path),
    }
}

fn run_silero_vad_file(path: &Path) -> Result<()> {
    let vad_cfg = crate::config::load_from(&crate::config::default_path())
        .map(|cfg| cfg.voice.vad)
        .unwrap_or_else(|_| crate::config::VoiceVadCfg::default());
    let samples = decode_audio_file_to_16k_mono(path)?;
    let mut vad = crate::voice::silero::SileroVad::new(crate::voice::silero::SileroConfig {
        threshold: vad_cfg.threshold,
    })?;
    let frames = vad.accept(&samples);
    let speech = frames
        .iter()
        .filter(|frame| matches!(frame.frame, crate::voice::vad::VadFrame::Speech))
        .count();
    let max = frames
        .iter()
        .map(|frame| frame.probability)
        .fold(0.0f32, f32::max);
    let avg = if frames.is_empty() {
        0.0
    } else {
        frames.iter().map(|frame| frame.probability).sum::<f32>() / frames.len() as f32
    };

    let effective_policy = crate::voice::vad::policy_from_config(
        &vad_cfg,
        crate::voice::silero::SileroConfig::frame_ms(),
    );
    let mut controller = crate::voice::vad::VadController::new(effective_policy);
    let mut resumes = 0;
    let mut pauses = 0;
    let mut transitions = Vec::new();
    for frame in &frames {
        match controller.accept(frame.frame) {
            crate::voice::vad::VadTransition::SpeechStarted => {
                resumes += 1;
                transitions.push(("resume", frame.start_sample, frame.probability));
            }
            crate::voice::vad::VadTransition::SilenceStarted => {
                pauses += 1;
                transitions.push(("pause", frame.start_sample, frame.probability));
            }
            crate::voice::vad::VadTransition::None => {}
        }
    }

    println!(
        "silero-vad-file: OK samples={} frames={} speech={} threshold={:.3} effective_min_start={} effective_pause_ms={} max={:.6} avg={:.6} resumes={} pauses={}",
        samples.len(),
        frames.len(),
        speech,
        vad_cfg.threshold,
        effective_policy.min_start_voiced_frames,
        effective_policy.pause_silence_ms,
        max,
        avg,
        resumes,
        pauses
    );
    for frame in frames.iter().take(20) {
        println!(
            "frame start_ms={} probability={:.6} speech={}",
            frame.start_sample * 1000 / 16_000,
            frame.probability,
            matches!(frame.frame, crate::voice::vad::VadFrame::Speech)
        );
    }
    for (kind, start_sample, probability) in transitions.iter().take(20) {
        println!(
            "transition kind={} start_ms={} probability={:.6}",
            kind,
            start_sample * 1000 / 16_000,
            probability
        );
    }
    Ok(())
}

fn decode_audio_file_to_16k_mono(path: &Path) -> Result<Vec<i16>> {
    let wav = std::env::temp_dir().join(format!("shuohua-vad-{}.wav", ulid::Ulid::new()));
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            &path.to_string_lossy(),
            "-ac",
            "1",
            "-ar",
            "16000",
            "-f",
            "wav",
            &wav.to_string_lossy(),
        ])
        .output()
        .with_context(|| "run ffmpeg for Silero VAD file diagnostic")?;
    if !output.status.success() {
        anyhow::bail!(
            "ffmpeg failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let mut reader =
        hound::WavReader::open(&wav).with_context(|| format!("read {}", wav.display()))?;
    let samples = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("decode {}", wav.display()))?;
    let _ = std::fs::remove_file(wav);
    Ok(samples)
}

fn init_i18n_for_cli() {
    let language = crate::config::load_from(&crate::config::default_path())
        .map(|cfg| cfg.ui.language)
        .unwrap_or_else(|_| "auto".to_string());
    crate::i18n::init(&language);
}

fn localized_command() -> clap::Command {
    Cli::command()
        .about(crate::t!("cli.help.about"))
        .mut_subcommand("doctor", |cmd| {
            cmd.about(crate::t!("cli.help.doctor.about"))
                .mut_arg("runtime", |arg| {
                    arg.help(crate::t!("cli.help.doctor.runtime"))
                })
        })
        .mut_subcommand("config-template", |cmd| {
            cmd.about(crate::t!("cli.help.config_template.about"))
                .mut_arg("out", |arg| {
                    arg.help(crate::t!("cli.help.config_template.out"))
                })
                .mut_arg("lang", |arg| {
                    arg.help(crate::t!("cli.help.config_template.lang"))
                })
        })
        .mut_subcommand("completions", |cmd| {
            cmd.about(crate::t!("cli.help.completions.about"))
                .mut_arg("shell", |arg| {
                    arg.help(crate::t!("cli.help.completions.shell"))
                })
        })
        .mut_subcommand("service", |cmd| {
            cmd.about(crate::t!("cli.help.service.about"))
                .mut_subcommand("install", |cmd| {
                    cmd.about(crate::t!("cli.help.service.install.about"))
                })
                .mut_subcommand("uninstall", |cmd| {
                    cmd.about(crate::t!("cli.help.service.uninstall.about"))
                })
                .mut_subcommand("start", |cmd| {
                    cmd.about(crate::t!("cli.help.service.start.about"))
                })
                .mut_subcommand("stop", |cmd| {
                    cmd.about(crate::t!("cli.help.service.stop.about"))
                })
                .mut_subcommand("restart", |cmd| {
                    cmd.about(crate::t!("cli.help.service.restart.about"))
                })
                .mut_subcommand("status", |cmd| {
                    cmd.about(crate::t!("cli.help.service.status.about"))
                })
        })
        .mut_subcommand("update", |cmd| {
            cmd.about(crate::t!("cli.help.update.about"))
                .mut_arg("allow_major", |arg| {
                    arg.help(crate::t!("cli.help.update.allow_major"))
                })
        })
        .mut_subcommand("version", |cmd| {
            cmd.about(crate::t!("cli.help.version.about"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_flags_parse_runtime() {
        let cli = Cli::try_parse_from(["shuo", "doctor", "--runtime"]).unwrap();

        match cli.command {
            Some(Command::Doctor(args)) => {
                assert!(args.runtime);
            }
            other => panic!("expected doctor command, got {other:?}"),
        }
    }

    #[test]
    fn completions_parse_shell() {
        let cli = Cli::try_parse_from(["shuo", "completions", "zsh"]).unwrap();

        match cli.command {
            Some(Command::Completions(args)) => {
                assert_eq!(args.shell, completions::Shell::Zsh);
            }
            other => panic!("expected completions command, got {other:?}"),
        }
    }

    #[test]
    fn service_subcommands_parse() {
        let cases = [
            ("install", service::ServiceCommand::Install),
            ("uninstall", service::ServiceCommand::Uninstall),
            ("start", service::ServiceCommand::Start),
            ("stop", service::ServiceCommand::Stop),
            ("restart", service::ServiceCommand::Restart),
            ("status", service::ServiceCommand::Status),
        ];

        for (name, expected) in cases {
            let cli = Cli::try_parse_from(["shuo", "service", name]).unwrap();
            match cli.command {
                Some(Command::Service(actual)) => assert_eq!(actual, expected),
                other => panic!("expected service {name}, got {other:?}"),
            }
        }
    }

    #[test]
    fn old_top_level_service_commands_are_removed() {
        for name in ["install", "uninstall", "start", "stop", "restart", "status"] {
            assert!(
                Cli::try_parse_from(["shuo", name]).is_err(),
                "{name} should no longer parse as a top-level service command"
            );
        }
    }

    #[test]
    fn update_parses_allow_major() {
        let cli = Cli::try_parse_from(["shuo", "update", "--allow-major"]).unwrap();

        match cli.command {
            Some(Command::Update(args)) => assert!(args.allow_major),
            other => panic!("expected update command, got {other:?}"),
        }
    }

    #[test]
    fn hidden_diagnostics_parse_silero_vad_smoke() {
        let cli = Cli::try_parse_from(["shuo", "diagnostics", "silero-vad"]).unwrap();

        match cli.command {
            Some(Command::Diagnostics(DiagnosticsCommand::SileroVad)) => {}
            other => panic!("expected diagnostics silero-vad, got {other:?}"),
        }
    }

    #[test]
    fn hidden_diagnostics_parse_silero_vad_file_smoke() {
        let cli =
            Cli::try_parse_from(["shuo", "diagnostics", "silero-vad-file", "sample.m4a"]).unwrap();

        match cli.command {
            Some(Command::Diagnostics(DiagnosticsCommand::SileroVadFile { path })) => {
                assert_eq!(path, PathBuf::from("sample.m4a"));
            }
            other => panic!("expected diagnostics silero-vad-file, got {other:?}"),
        }
    }

    #[test]
    fn completions_generate_zsh_script() {
        crate::i18n::init("en-US");

        let mut out = Vec::new();
        completions::write(completions::Shell::Zsh, &mut out).unwrap();
        let script = String::from_utf8(out).unwrap();

        assert!(script.contains("#compdef shuo"), "{script}");
        assert!(script.contains("_shuo()"), "{script}");
    }

    #[test]
    fn doctor_rejects_removed_network_flag() {
        assert!(Cli::try_parse_from(["shuo", "doctor", "--network"]).is_err());
    }

    #[test]
    fn doctor_rejects_removed_full_flag() {
        assert!(Cli::try_parse_from(["shuo", "doctor", "--full"]).is_err());
    }

    #[test]
    fn cli_i18n_keys_are_available() {
        crate::i18n::init("en-US");

        assert_eq!(
            crate::i18n::tr("cli.service.started", &[("label", "x".to_string())]),
            "started x"
        );
    }

    #[test]
    fn help_uses_initialized_language() {
        crate::i18n::init("zh-CN");

        let err = localized_command()
            .try_get_matches_from(["shuo", "doctor", "--help"])
            .unwrap_err();
        let help = err.to_string();

        assert!(help.contains("检查本地环境和配置"), "{help}");
        assert!(
            help.contains("检查已配置的 ASR 和 LLM 组件运行路径"),
            "{help}"
        );

        let err = localized_command()
            .try_get_matches_from(["shuo", "--help"])
            .unwrap_err();
        let help = err.to_string();

        assert!(help.contains("管理后台服务"), "{help}");
        assert!(help.contains("生成 shell completion 脚本"), "{help}");
        assert!(help.contains("显示版本号"), "{help}");
    }
}
