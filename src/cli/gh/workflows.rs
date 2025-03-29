use std::io::Write;

use anyhow::{bail, ensure, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::octokit;

use super::WithRepos;

#[derive(Debug, Default, Parser)]
pub(super) struct Opts {
    /// List all workflows.
    #[arg(long)]
    list: bool,
    /// Enable github workflows.
    #[arg(long)]
    enable: bool,
    /// Disable github workflows.
    #[arg(long)]
    disable: bool,
}

pub(super) async fn entry(
    opts: &Opts,
    with_repos: impl WithRepos<'_>,
    client: &octokit::Client,
) -> Result<()> {
    with_repos
        .run(
            "Github API (release)",
            format_args!("Github API (release): {opts:?}"),
            async |cx, repo| run(cx, repo, opts, client).await,
            |_| Ok(()),
        )
        .await?;

    Ok(())
}

async fn run(_: &Ctxt<'_>, repo: &Repo, opts: &Opts, client: &octokit::Client) -> Result<()> {
    let Some(r) = repo.repo() else {
        return Ok(());
    };

    let Some(workflows) = client.workflows_list(r.owner, r.name).await? else {
        bail!("No workflows found");
    };

    if opts.list {
        println!("{}:", repo.path());

        let mut o = std::io::stdout().lock();

        for w in &workflows.workflows {
            serde_json::to_writer_pretty(&mut o, &w)?;
            writeln!(o)?;
        }
    }

    if opts.enable {
        ensure!(
            !workflows.workflows.is_empty(),
            "{}: No workflows to enable",
            repo.path()
        );

        for w in &workflows.workflows {
            let (status, body) = client.workflows_enable(r.owner, r.name, w.id).await?;

            println!(
                "{}: Enabling workflow `{}`: {status}: {body}",
                repo.path(),
                w.name
            );
        }
    }

    if opts.disable {
        ensure!(
            !workflows.workflows.is_empty(),
            "{}: No workflows to disable",
            repo.path()
        );

        for w in &workflows.workflows {
            let (status, body) = client.workflows_disable(r.owner, r.name, w.id).await?;

            println!(
                "{}: Disabling workflow `{}`: {status}: {body}",
                repo.path(),
                w.name
            );
        }
    }

    Ok(())
}
