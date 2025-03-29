mod release;
mod workflows;

use core::fmt;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::ctxt::Ctxt;
use crate::Repo;

use super::{with_repos_async, PARALLELISM};

trait WithRepos<'repo> {
    async fn run<T>(
        self,
        what: impl fmt::Display,
        hint: impl fmt::Display,
        f: impl AsyncFn(&Ctxt<'repo>, &'repo Repo) -> Result<T>,
        report: impl FnMut(T) -> Result<()>,
    ) -> Result<()>;
}

struct WithParallelism<'a, 'repo> {
    cx: &'a mut Ctxt<'repo>,
    parallelism: usize,
}

impl<'repo> WithRepos<'repo> for WithParallelism<'_, 'repo> {
    #[inline]
    async fn run<T>(
        self,
        what: impl fmt::Display,
        hint: impl fmt::Display,
        f: impl AsyncFn(&Ctxt<'repo>, &'repo Repo) -> Result<T>,
        report: impl FnMut(T) -> Result<()>,
    ) -> Result<()> {
        with_repos_async(self.cx, what, hint, self.parallelism, f, report).await
    }
}

#[derive(Debug, Parser)]
pub(crate) struct Opts {
    /// The number of repositories to read in parallel.
    #[arg(long, default_value = PARALLELISM, value_name = "count")]
    parallelism: usize,
    /// Command to use.
    #[command(subcommand, name = "action")]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Workflows(self::workflows::Opts),
    Release(self::release::Opts),
}

pub(crate) async fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let client = cx.octokit()?;

    let with_repos = WithParallelism {
        cx,
        parallelism: opts.parallelism,
    };

    match &opts.command {
        Command::Workflows(opts) => {
            self::workflows::entry(opts, with_repos, &client).await?;
        }
        Command::Release(opts) => {
            self::release::entry(opts, with_repos, &client).await?;
        }
    }

    Ok(())
}
