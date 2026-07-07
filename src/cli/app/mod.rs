pub mod archive;
pub mod platform;
pub mod release;
pub mod update;
pub mod version;

use anyhow::Result;
use clap::Args;

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Check for updates without installing.
    #[arg(long)]
    pub check: bool,

    /// Allow updates across major versions.
    #[arg(long)]
    pub allow_major: bool,
}

pub async fn update(args: UpdateArgs) -> Result<()> {
    update::run(args).await
}
