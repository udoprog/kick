use anyhow::{bail, Result};
use clap::Parser;

use crate::cli::WithRepos;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::process::Command;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Pass `--workspace` to `cargo upgrade`.
    #[arg(long)]
    workspace: bool,
    /// Pass `--exclude <package>` to `cargo upgrade`.
    #[arg(long, value_name = "package")]
    exclude: Vec<String>,
    /// Extra arguments to pass to `cargo upgrade`.
    #[arg(value_name = "extra")]
    extra: Vec<String>,
}

pub(crate) fn entry<'repo>(with_repos: impl WithRepos<'repo>, opts: &Opts) -> Result<()> {
    with_repos.run("upgrade", format_args!("upgrade: {opts:?}"), |cx, repo| {
        upgrade(cx, opts, repo)
    })?;

    Ok(())
}

#[tracing::instrument(skip_all)]
fn upgrade(cx: &Ctxt<'_>, opts: &Opts, repo: &Repo) -> Result<()> {
    let current_dir = cx.to_path(repo.path());
    let upgrade = cx.config.upgrade(repo);

    let mut command = Command::new("cargo");
    command.arg("upgrade");

    for exclude in upgrade.exclude.iter().chain(&opts.exclude) {
        command.args(["--exclude", exclude]);
    }

    if opts.workspace {
        command.arg("--workspace");
    }

    for arg in &opts.extra {
        command.arg(arg);
    }

    command.current_dir(&current_dir);

    if !command.status()?.success() {
        bail!("Command failed");
    }

    Ok(())
}
