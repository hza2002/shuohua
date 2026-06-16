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
    match command {
        Command::Doctor(args) => doctor::run(args),
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
