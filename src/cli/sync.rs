use core::str::FromStr;

use std::fs;

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

impl FromStr for Kind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "git" => Ok(Kind::Git),
            _ => bail!("unknown repository kind: {s}"),
        }
    }
}

#[derive(Default, Debug, Clone, Parser)]
pub(crate) struct Opts {
    /// If there is no repository present, initialize one with the given kind.
    ///
    /// This is a destructive operation that will overwrite existing files
    /// inside of any existing directories.
    #[clap(long, num_args = 0..=1, default_missing_value = "git")]
    init: Option<Kind>,
}

pub(crate) async fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    with_repos
        .run_async(
            "synchronize repos",
            format_args!("sync: {opts:?}"),
            async |cx, repo| sync(cx, repo, opts).await,
            |_| Ok(()),
        )
        .await?;

    Ok(())
}

async fn sync(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
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

        if let Some(kind) = opts.init {
            if !path.is_dir() {
                tracing::info!("creating directory at {}", path.display());
                fs::create_dir_all(&path).with_context(|| path.display().to_string())?;
            }

            match kind {
                Kind::Git => {
                    let branch = cx.config.branch(repo).context("no branch configured")?;

                    let git = cx.require_git()?;
                    git.init(&path).await?;
                    git.remote_add(&path, "origin", repo.url()).await?;
                    git.fetch(&path, "origin", branch).await?;
                    git.force_checkout(&path, branch).await?;

                    if let Some(push_url) = repo.push_url()
                        && git.remote_get_push_url(&path, "origin").await? != push_url
                    {
                        git.remote_set_push_url(&path, "origin", push_url).await?;
                    }

                    tracing::info!("initialized git repository at {}", path.display());
                    return Ok(());
                }
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
            ensure!(!git.is_dirty(&path).await?, "repository is dirty");
            let branch = cx.config.branch(repo).context("no branch configured")?;
            git.fetch(&path, "origin", branch).await?;
            let rev = git.rev_parse(&path, "FETCH_HEAD").await?;
            let outcome = git.merge_fast_forward(&path, &rev).await?;
            if !outcome.success {
                if !outcome.stdout.is_empty() {
                    tracing::error!("stdout:\n{}", outcome.stdout.trim());
                }

                if !outcome.stderr.is_empty() {
                    tracing::error!("stderr:\n{}", outcome.stderr.trim());
                }

                bail!("fast-forward merge failed");
            }

            if let Some(push_url) = repo.push_url()
                && git.remote_get_push_url(&path, "origin").await? != push_url
            {
                git.remote_set_push_url(&path, "origin", push_url).await?;
            }

            tracing::info!(?branch, ?rev, "synchronized repo");
        }
    }

    Ok(())
}
