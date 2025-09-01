mod release;
mod status;
mod workflows;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use crate::cli::{PARALLELISM, WithRepos};

#[derive(Debug, Parser)]
struct SharedOpts {
    /// The number of repositories to read in parallel.
    #[arg(long, default_value = PARALLELISM, value_name = "count")]
    parallelism: usize,
}

#[derive(Debug, Parser)]
struct InnerOpts<T>
where
    T: Args,
{
    #[command(flatten)]
    shared: SharedOpts,
    #[command(flatten)]
    inner: T,
}

#[derive(Debug, Parser)]
pub(crate) struct Opts {
    /// Command to use.
    #[command(subcommand, name = "action")]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Get the build status of workflows.
    Status(InnerOpts<self::status::Opts>),
    /// Modify workflows.
    Workflows(InnerOpts<self::workflows::Opts>),
    /// Build a release and upload files.
    Release(InnerOpts<self::release::Opts>),
}

impl Command {
    fn shared(&self) -> &SharedOpts {
        match self {
            Command::Status(opts) => &opts.shared,
            Command::Workflows(opts) => &opts.shared,
            Command::Release(opts) => &opts.shared,
        }
    }
}

pub(crate) async fn entry<'repo>(with_repos: impl WithRepos<'repo>, opts: &Opts) -> Result<()> {
    let client = with_repos.cx().octokit()?;

    let with_repos = with_repos.with_parallelism(opts.command.shared().parallelism);

    match &opts.command {
        Command::Status(opts) => {
            self::status::entry(&opts.inner, with_repos, &client).await?;
        }
        Command::Workflows(opts) => {
            self::workflows::entry(&opts.inner, with_repos, &client).await?;
        }
        Command::Release(opts) => {
            self::release::entry(&opts.inner, with_repos, &client).await?;
        }
    }

    Ok(())
}
