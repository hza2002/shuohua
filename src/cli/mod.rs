pub mod config_template;
pub mod doctor;
pub mod service;

use anyhow::Result;
use clap::{Parser, Subcommand};

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
    Cli::parse()
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
}
