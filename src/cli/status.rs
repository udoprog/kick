use std::fmt;
use std::io::Write;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Local, TimeZone};
use clap::Parser;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use crate::ctxt::Ctxt;
use crate::model::{Repo, RepoPath};
use crate::octokit;

const PARALLELISM: &str = "8";

#[derive(Debug, Default, Parser)]
pub(crate) struct Opts {
    /// The number of repositories to read in parallel.
    #[arg(long, default_value = PARALLELISM, value_name = "count")]
    parallelism: usize,
    /// Include information on individual jobs.
    #[arg(long)]
    jobs: bool,
}

pub(crate) async fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let client = cx.octokit()?;
    let today = Local::now();

    let mut o = StandardStream::stdout(ColorChoice::Auto);

    async_with_repos!(
        cx,
        "get build status",
        format_args!("status: {opts:?}"),
        opts.parallelism,
        async |cx, repo| do_status(cx, repo, opts, &client).await?,
        |outcome| outcome.display(&mut o, today),
    );

    Ok(())
}

async fn do_status<'repo>(
    cx: &Ctxt<'_>,
    repo: &'repo Repo,
    opts: &Opts,
    client: &octokit::Client,
) -> Result<Outcome<'repo>> {
    let workflows = cx.config.workflows(repo)?;

    if workflows.is_empty() {
        return Ok(Outcome::ignore(repo));
    }

    let Some(repo_path) = repo.repo() else {
        return Ok(Outcome::ignore(repo));
    };

    let mut statuses = Vec::new();
    let mut ok = true;

    for id in workflows.into_keys() {
        let mut conclusions = Vec::new();
        ok &= status(cx, &id, opts, repo, repo_path, client, &mut conclusions).await?;
        statuses.push(Status { id, conclusions });
    }

    Ok(Outcome::output(repo, !ok, statuses))
}

#[tracing::instrument(skip_all)]
async fn status(
    cx: &Ctxt<'_>,
    id: &str,
    opts: &Opts,
    repo: &Repo,
    path: RepoPath<'_>,
    client: &octokit::Client,
    conclusions: &mut Vec<Conclusion>,
) -> Result<bool> {
    let sha;

    let sha = match cx.system.git.first() {
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
        bail!("No workflow runs found");
    };

    let mut remaining = 1;
    let mut ok = true;

    while let Some(runs) = client.next_page(&mut res).await? {
        if remaining == 0 {
            break;
        }

        for run in runs.into_iter().take(remaining) {
            remaining -= 1;

            let failure = run.conclusion.as_deref() == Some("failure");

            let jobs = 'jobs: {
                if !(opts.jobs || failure) {
                    break 'jobs Vec::new();
                }

                let Some(jobs_url) = &run.jobs_url else {
                    break 'jobs Vec::new();
                };

                let Some(jobs) = client.get::<octokit::Jobs>(jobs_url).await? else {
                    break 'jobs Vec::new();
                };

                jobs.jobs
            };

            conclusions.push(Conclusion {
                failure,
                sha: sha.map(str::to_owned),
                run: Box::new(run),
                jobs,
            });

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
    #[inline]
    fn new(today: DateTime<T>, date: Option<DateTime<T>>) -> Self {
        Self { today, date }
    }
}

impl<T> fmt::Display for FormatTime<T>
where
    T: TimeZone,
{
    #[inline]
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

struct Conclusion {
    failure: bool,
    sha: Option<String>,
    run: Box<octokit::WorkflowRun>,
    jobs: Vec<octokit::Job>,
}

struct Status {
    id: String,
    conclusions: Vec<Conclusion>,
}

struct InnerOutcome {
    failure: bool,
    statuses: Vec<Status>,
}

struct Outcome<'repo> {
    repo: &'repo Repo,
    outcome: Option<InnerOutcome>,
}

impl<'repo> Outcome<'repo> {
    fn output(repo: &'repo Repo, failure: bool, statuses: Vec<Status>) -> Self {
        Self {
            repo,
            outcome: Some(InnerOutcome { failure, statuses }),
        }
    }

    fn ignore(repo: &'repo Repo) -> Self {
        Self {
            repo,
            outcome: None,
        }
    }

    fn display(self, o: &mut StandardStream, today: DateTime<Local>) -> Result<()> {
        let Some(InnerOutcome { failure, statuses }) = self.outcome else {
            return Ok(());
        };

        let failure_color = {
            let mut c = ColorSpec::new();
            c.set_fg(Some(Color::Red));
            c
        };

        writeln!(o, "{}: {}", self.repo.path(), self.repo.url())?;

        for Status { id, conclusions } in statuses {
            writeln!(o, "Workflow `{id}`:")?;

            for conclusion in conclusions {
                let Conclusion {
                    failure,
                    sha,
                    run,
                    jobs,
                } = conclusion;

                let updated_at = FormatTime::new(today, Some(run.updated_at.with_timezone(&Local)));

                let head = if sha.as_deref() == Some(run.head_sha.as_str()) {
                    "* "
                } else {
                    "  "
                };

                if failure {
                    o.set_color(&failure_color)?;
                }

                writeln!(
                    o,
                    "{head}{sha} {branch}: {updated_at}: status: {}, conclusion: {}",
                    run.status,
                    run.conclusion.as_deref().unwrap_or("*in progress*"),
                    branch = run.head_branch,
                    sha = short(&run.head_sha),
                )?;

                if failure {
                    o.reset()?;
                }

                for job in jobs {
                    let failure = job.conclusion.as_deref() == Some("failure");

                    if failure {
                        o.set_color(&failure_color)?;
                    }

                    writeln!(o, "  {}", job.name)?;
                    writeln!(o, "    {}", job.html_url)?;

                    writeln!(
                        o,
                        "    status: {status}, conclusion: {conclusion}",
                        status = job.status,
                        conclusion = job.conclusion.as_deref().unwrap_or("*in progress*"),
                    )?;

                    writeln!(
                        o,
                        "    time: {} - {}",
                        FormatTime::new(today, job.started_at.map(|d| d.with_timezone(&Local))),
                        FormatTime::new(today, job.completed_at.map(|d| d.with_timezone(&Local)))
                    )?;

                    if failure {
                        o.reset()?;
                    }
                }
            }
        }

        if failure {
            bail!("Status is not OK")
        }

        Ok(())
    }
}
