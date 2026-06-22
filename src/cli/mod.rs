pub mod completions;
pub mod config_template;
pub mod doctor;
pub mod service;

use anyhow::{Context, Result};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

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
    Install,
    Uninstall,
    Start,
    Stop,
    Restart,
    Status,
    Version,
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
        Command::Install => service::install(),
        Command::Uninstall => service::uninstall(),
        Command::Start => service::start(),
        Command::Stop => service::stop().await,
        Command::Restart => service::restart().await,
        Command::Status => service::status().await,
        Command::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
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
        .mut_subcommand("install", |cmd| {
            cmd.about(crate::t!("cli.help.install.about"))
        })
        .mut_subcommand("uninstall", |cmd| {
            cmd.about(crate::t!("cli.help.uninstall.about"))
        })
        .mut_subcommand("start", |cmd| cmd.about(crate::t!("cli.help.start.about")))
        .mut_subcommand("stop", |cmd| cmd.about(crate::t!("cli.help.stop.about")))
        .mut_subcommand("restart", |cmd| {
            cmd.about(crate::t!("cli.help.restart.about"))
        })
        .mut_subcommand("status", |cmd| {
            cmd.about(crate::t!("cli.help.status.about"))
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

        assert!(help.contains("安装并启动 launchd 服务"), "{help}");
        assert!(help.contains("生成 shell completion 脚本"), "{help}");
        assert!(help.contains("显示版本号"), "{help}");
    }
}
