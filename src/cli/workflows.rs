use anyhow::Result;
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::octokit;

#[derive(Debug, Default, Parser)]
pub(crate) struct Opts {
    /// Enable github workflows.
    #[arg(long)]
    enable: bool,
    /// Get remote information on workflows.
    #[arg(long)]
    get: bool,
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
    cx: &Ctxt<'_>,
    repo: &Repo,
    opts: &Opts,
    client: &octokit::Client,
) -> Result<()> {
    let workflows = cx.config.workflows(repo)?;

    println!("{}: {}", repo.path(), repo.url());

    let Some(repo) = repo.repo() else {
        return Ok(());
    };

    if opts.enable {
        for id in workflows.keys() {
            let status = client.workflows_enable(repo.owner, repo.name, id).await?;
            println!("Enable workflow `{id}`: {status:?}");
        }
    }

    if opts.get {
        for id in workflows.keys() {
            let workflow = client.workflows_get(repo.owner, repo.name, id).await?;

            if let Some(workflow) = workflow {
                println!("{}", serde_json::to_string_pretty(&workflow)?);
            } else {
                println!("Workflow `{id}` not found");
            }
        }
    }

    Ok(())
}
