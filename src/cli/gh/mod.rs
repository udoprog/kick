mod release;
mod status;
mod workflows;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::cli::WithRepos;

#[derive(Debug, Parser)]
pub(crate) struct Opts {
    /// Command to use.
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Get the build status of workflows.
    Status(self::status::Opts),
    /// Modify workflows.
    Workflows(self::workflows::Opts),
    /// Build a release and upload files.
    Release(self::release::Opts),
}

pub(crate) async fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    let client = with_repos.cx().octokit()?;

    match &opts.command {
        Command::Status(opts) => {
            self::status::entry(opts, with_repos, &client).await?;
        }
        Command::Workflows(opts) => {
            self::workflows::entry(opts, with_repos, &client).await?;
        }
        Command::Release(opts) => {
            self::release::entry(opts, with_repos, &client).await?;
        }
    }

    Ok(())
}
