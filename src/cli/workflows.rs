use std::io::Write;

use anyhow::{bail, ensure, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::octokit;

#[derive(Debug, Default, Parser)]
pub(crate) struct Opts {
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

pub(crate) async fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let client = cx.octokit()?;

    with_repos!(
        cx,
        "workflows",
        format_args!("workflows: {opts:?}"),
        |cx, repo| do_workflows(cx, repo, opts, &client).await,
    );

    Ok(())
}

async fn do_workflows(
    _: &Ctxt<'_>,
    repo: &Repo,
    opts: &Opts,
    client: &octokit::Client,
) -> Result<()> {
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
            let status = client.workflows_enable(r.owner, r.name, w.id).await?;
            println!(
                "{}: Enabling workflow `{}`: {status:?}",
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
            let status = client.workflows_disable(r.owner, r.name, w.id).await?;
            println!(
                "{}: Disabling workflow `{}`: {status:?}",
                repo.path(),
                w.name
            );
        }
    }

    Ok(())
}
