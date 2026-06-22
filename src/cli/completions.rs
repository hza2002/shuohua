use std::io::Write;

use anyhow::Result;
use clap::{Args, ValueEnum};

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// Shell to generate completions for.
    #[arg(value_enum)]
    pub shell: Shell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
}

pub fn run(args: CompletionsArgs) -> Result<()> {
    let mut stdout = std::io::stdout();
    write(args.shell, &mut stdout)
}

pub fn write(shell: Shell, out: &mut dyn Write) -> Result<()> {
    let mut command = super::localized_command();
    clap_complete::generate(shell.generator(), &mut command, "shuo", out);
    Ok(())
}

impl Shell {
    fn generator(self) -> clap_complete::Shell {
        match self {
            Shell::Zsh => clap_complete::Shell::Zsh,
            Shell::Bash => clap_complete::Shell::Bash,
            Shell::Fish => clap_complete::Shell::Fish,
        }
    }
}
