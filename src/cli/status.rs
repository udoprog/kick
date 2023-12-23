use core::fmt;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Local, TimeZone};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::{Repo, RepoPath};
use crate::octokit;

#[derive(Debug, Default, Parser)]
pub(crate) struct Opts {
    /// Include information on individual jobs.
    #[arg(long)]
    jobs: bool,
}

pub(crate) async fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let client = cx.octokit()?;
    let today = Local::now();

    with_repos!(
        cx,
        "get build status",
        format_args!("status: {opts:?}"),
        |cx, repo| do_status(cx, repo, opts, &client, today).await,
    );

    Ok(())
}

async fn do_status(
    cx: &Ctxt<'_>,
    repo: &Repo,
    opts: &Opts,
    client: &octokit::Client,
    today: DateTime<Local>,
) -> Result<()> {
    let workflows = cx.config.workflows(repo)?;

    if workflows.is_empty() {
        return Ok(());
    }

    let Some(repo_path) = repo.repo() else {
        return Ok(());
    };

    println!("{}: {}", repo.path(), repo.url());

    let mut ok = true;

    for id in workflows.into_keys() {
        println!("Workflow `{id}`:");
        ok &= status(cx, &id, opts, repo, repo_path, today, client).await?;
    }

    if !ok {
        bail!("Status is not OK")
    }

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn status(
    cx: &Ctxt<'_>,
    id: &str,
    opts: &Opts,
    repo: &Repo,
    path: RepoPath<'_>,
    today: DateTime<Local>,
    client: &octokit::Client,
) -> Result<bool> {
    let sha;

    let sha = match &cx.git {
        Some(git) => {
            sha = git
                .rev_parse(cx.to_path(repo.path()), "HEAD")
                .context("Getting head commit")?;
            Some(sha.trim())
        }
        None => None,
    };

    let Some(mut res) = client
        .workflow_runs(path.owner, path.name, id, true, Some(1))
        .await?
    else {
        println!("  Workflow `{id}` not found");
        return Ok(false);
    };

    let mut remaining = 1;
    let mut ok = true;

    while let Some(runs) = client.next_page(&mut res).await? {
        if remaining == 0 {
            break;
        }

        for run in runs.into_iter().take(remaining) {
            remaining -= 1;
            let updated_at = FormatTime::new(today, Some(run.updated_at.with_timezone(&Local)));

            let head = if sha == Some(&run.head_sha) {
                "* "
            } else {
                "  "
            };

            println!(
                " {head}{sha} {branch}: {updated_at}: status: {}, conclusion: {}",
                run.status,
                run.conclusion.as_deref().unwrap_or("*in progress*"),
                branch = run.head_branch,
                sha = short(&run.head_sha),
            );

            let failure = run.conclusion.as_deref() == Some("failure");

            if opts.jobs || failure {
                if let Some(jobs_url) = &run.jobs_url {
                    let jobs: Option<octokit::Jobs> = client.get(jobs_url).await?;

                    let Some(jobs) = jobs else {
                        continue;
                    };

                    for job in jobs.jobs {
                        println!(
                            "   {name}: {html_url}",
                            name = job.name,
                            html_url = job.html_url
                        );

                        println!(
                            "     status: {status}, conclusion: {conclusion}",
                            status = job.status,
                            conclusion = job.conclusion.as_deref().unwrap_or("*in progress*"),
                        );

                        println!(
                            "     time: {} - {}",
                            FormatTime::new(today, job.started_at.map(|d| d.with_timezone(&Local))),
                            FormatTime::new(
                                today,
                                job.completed_at.map(|d| d.with_timezone(&Local))
                            )
                        );
                    }
                }
            }

            ok &= !failure;
        }
    }

    Ok(ok)
}

fn short(string: &str) -> impl std::fmt::Display + '_ {
    if let Some(sha) = string.get(..7) {
        return sha;
    }

    string
}

struct FormatTime<T>
where
    T: TimeZone,
{
    today: DateTime<T>,
    date: Option<DateTime<T>>,
}

impl<T> FormatTime<T>
where
    T: TimeZone,
{
    fn new(today: DateTime<T>, date: Option<DateTime<T>>) -> Self {
        Self { today, date }
    }
}

impl<T> fmt::Display for FormatTime<T>
where
    T: TimeZone,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(date) = &self.date else {
            return "?".fmt(f);
        };

        if self.today.date_naive() == date.date_naive() {
            return date.time().fmt(f);
        }

        date.date_naive().fmt(f)
    }
}
