pub mod config_template;
pub mod doctor;
pub mod service;

use anyhow::Result;
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
    match command {
        Command::Doctor(args) => doctor::run(args),
        Command::ConfigTemplate(args) => config_template::run(args),
        Command::Install => service::install(),
        Command::Uninstall => service::uninstall(),
        Command::Start => service::start(),
        Command::Stop => service::stop(),
        Command::Restart => service::restart(),
        Command::Status => service::status(),
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
        assert!(help.contains("显示版本号"), "{help}");
    }
}
