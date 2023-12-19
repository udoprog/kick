use anyhow::{anyhow, Context, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::process::Command;
use crate::repo_sets::RepoSet;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Command to run.
    command: Vec<String>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let Some((command, args)) = opts.command.split_first() else {
        return Err(anyhow!("missing command"));
    };

    let mut good = RepoSet::default();
    let mut bad = RepoSet::default();

    for repo in cx.repos() {
        r#for(cx, repo, command, args, &mut good, &mut bad).with_context(cx.context(repo))?;
    }

    let hint = format!("for: {:?}", opts);
    cx.sets.save("good", good, &hint);
    cx.sets.save("bad", bad, &hint);
    Ok(())
}

#[tracing::instrument(name = "for", skip_all, fields(path = repo.path().as_str()))]
fn r#for(
    cx: &Ctxt<'_>,
    repo: &Repo,
    command: &str,
    args: &[String],
    good: &mut RepoSet,
    bad: &mut RepoSet,
) -> Result<()> {
    let mut command = Command::new(command);
    command.args(args);
    command.current_dir(cx.to_path(repo.path()));

    tracing::info!("{}", command.display());

    if command.status()?.success() {
        good.insert(repo);
    } else {
        tracing::warn!(?command, "command failed");
        bad.insert(repo);
    }

    Ok(())
}
