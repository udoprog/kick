use anyhow::{Context, Result, bail, ensure};
use clap::Parser;

use crate::Repo;
use crate::cli::WithRepos;
use crate::ctxt::Ctxt;

const DIRS: &[(&str, Kind)] = &[(".git", Kind::Git)];

#[derive(Debug, Clone, Copy)]
enum Kind {
    Git,
}

#[derive(Default, Debug, Clone, Parser)]
pub(crate) struct Opts {}

pub(crate) fn entry<'repo>(with_repos: impl WithRepos<'repo>, opts: &Opts) -> Result<()> {
    with_repos.run(
        "synchronize repos",
        format_args!("sync: {opts:?}"),
        |cx, repo| sync(cx, repo, opts),
    )?;

    Ok(())
}

fn sync(cx: &Ctxt<'_>, repo: &Repo, _: &Opts) -> Result<()> {
    let mut path = cx.to_path(repo.path());

    let kind = 'kind: {
        for (dir, kind) in DIRS {
            path.push(dir);
            let exists = path.exists();
            path.pop();

            if exists {
                break 'kind kind;
            }
        }

        let kinds = DIRS
            .iter()
            .map(|(dir, _)| *dir)
            .collect::<Vec<_>>()
            .join(", ");

        bail!("Unknown repository kind, expected one of: {kinds}");
    };

    match kind {
        Kind::Git => {
            let git = cx.require_git()?;
            ensure!(!git.is_dirty(&path)?, "repository is dirty");
            let branch = cx.config.branch(repo).context("no branch configured")?;
            git.fetch(&path, "origin", branch)?;
            let rev = git.rev_parse(&path, "FETCH_HEAD")?;
            let outcome = git.merge_fast_forward(&path, &rev)?;

            if !outcome.success {
                if !outcome.stdout.is_empty() {
                    tracing::error!("stdout:\n{}", outcome.stdout.trim());
                }

                if !outcome.stderr.is_empty() {
                    tracing::error!("stderr:\n{}", outcome.stderr.trim());
                }

                bail!("fast-forward merge failed");
            }

            tracing::info!(?branch, ?rev, "synchronized repo");
        }
    }

    Ok(())
}
