use anyhow::{Context, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::process::Command;
use crate::repo_sets::RepoSet;

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

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut good = RepoSet::default();
    let mut bad = RepoSet::default();

    for repo in cx.repos() {
        upgrade(cx, opts, repo, &mut good, &mut bad).with_context(cx.context(repo))?;
    }

    let hint = format!("upgrade: {opts:?}");
    cx.sets.save("good", good, &hint);
    cx.sets.save("bad", bad, &hint);
    Ok(())
}

#[tracing::instrument(skip_all, fields(source = ?repo.source(), path = repo.path().as_str()))]
fn upgrade(
    cx: &Ctxt<'_>,
    opts: &Opts,
    repo: &Repo,
    good: &mut RepoSet,
    bad: &mut RepoSet,
) -> Result<()> {
    let current_dir = cx.to_path(repo.path());
    let upgrade = cx.config.upgrade(repo.path());

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

    if command.status()?.success() {
        good.insert(repo);
    } else {
        tracing::warn!(?command, "Command failed");
        bad.insert(repo);
    }

    Ok(())
}
