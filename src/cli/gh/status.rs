use std::fmt;
use std::io::Write;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, TimeZone};
use clap::Parser;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use crate::cli::WithRepos;
use crate::ctxt::Ctxt;
use crate::model::{Repo, RepoPath};
use crate::octokit;
use crate::once::Once;

#[derive(Debug, Default, Parser)]
pub(super) struct Opts {
    /// Include information on individual jobs.
    #[arg(long)]
    jobs: bool,
}

pub(super) async fn entry(
    opts: &Opts,
    with_repos: &mut WithRepos<'_>,
    client: &octokit::Client,
) -> Result<()> {
    let today = Once::new(Local::now);

    with_repos
        .run_async(
            "Github API (status)",
            format_args!("Github API (status): {opts:?}"),
            async |cx, repo| do_status(cx, repo, opts, client).await,
            |outcome| {
                let mut o = StandardStream::stdout(ColorChoice::Auto);
                outcome.display(&mut o, &today)
            },
        )
        .await?;

    Ok(())
}

async fn do_status<'repo>(
    cx: &Ctxt<'_>,
    repo: &'repo Repo,
    opts: &Opts,
    client: &octokit::Client,
) -> Result<Outcome<'repo>> {
    let sha = match cx.system.git.first() {
        Some(git) => {
            let sha = git
                .rev_parse(cx.to_path(repo.path()), "HEAD")
                .context("Getting head commit")?;
            Some(sha.trim().to_owned())
        }
        None => None,
    };

    let mut conclusions = Vec::new();

    if let Some(repo_path) = repo.repo() {
        for id in cx.config.workflows(repo)?.into_keys() {
            conclusions.push(status(id, opts, repo_path, client).await?);
        }
    }

    Ok(Outcome {
        repo,
        sha,
        conclusions,
    })
}

#[tracing::instrument(skip_all)]
async fn status(
    id: String,
    opts: &Opts,
    path: RepoPath<'_>,
    client: &octokit::Client,
) -> Result<Conclusion> {
    let Some(mut res) = client
        .workflow_runs(path.owner, path.name, &id, true, Some(1))
        .await?
    else {
        bail!("No workflow runs found");
    };

    let first = client
        .next_page(&mut res)
        .await?
        .into_iter()
        .flatten()
        .next();

    let Some(run) = first else {
        bail!("Not runs or jobs found");
    };

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

    Ok(Conclusion { id, run, jobs })
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
    id: String,
    run: octokit::WorkflowRun,
    jobs: Vec<octokit::Job>,
}

struct Outcome<'repo> {
    repo: &'repo Repo,
    sha: Option<String>,
    conclusions: Vec<Conclusion>,
}

impl Outcome<'_> {
    #[inline]
    fn display(
        self,
        o: &mut StandardStream,
        today: &Once<DateTime<Local>, impl Fn() -> DateTime<Local>>,
    ) -> Result<()> {
        let failure_color = {
            let mut c = ColorSpec::new();
            c.set_fg(Some(Color::Red));
            c.set_bold(true);
            c
        };

        let success_color = {
            let mut c = ColorSpec::new();
            c.set_fg(Some(Color::Green));
            c.set_bold(true);
            c
        };

        let mut ok = true;

        writeln!(o, "{}: {}", self.repo.path(), self.repo.url())?;

        for Conclusion { id, run, jobs } in self.conclusions {
            let failure = run.conclusion.as_deref() == Some("failure");

            ok &= !failure;

            let octokit::WorkflowRun {
                head_sha,
                head_branch,
                updated_at,
                status,
                ..
            } = run;

            let updated_at = FormatTime::new(today.get(), Some(updated_at.with_timezone(&Local)));

            let head = if self.sha.as_deref() == Some(head_sha.as_str()) {
                "*"
            } else {
                ""
            };

            let status = run.conclusion.as_deref().unwrap_or(&status);

            write!(o, "  Workflow `{id}` (")?;

            let color = if failure {
                &failure_color
            } else {
                &success_color
            };

            o.set_color(color)?;
            write!(o, "{status}")?;
            o.reset()?;

            writeln!(o, "):")?;
            writeln!(
                o,
                "    git: {head}{sha} ({head_branch})",
                sha = short(&head_sha)
            )?;
            writeln!(o, "    time: {updated_at}")?;

            for job in jobs {
                let octokit::Job {
                    name,
                    html_url,
                    status,
                    conclusion,
                    started_at,
                    completed_at,
                    ..
                } = job;

                let failure = conclusion.as_deref() == Some("failure");
                let status = conclusion.as_deref().unwrap_or(&status);
                let started =
                    FormatTime::new(today.get(), started_at.map(|d| d.with_timezone(&Local)));
                let completed =
                    FormatTime::new(today.get(), completed_at.map(|d| d.with_timezone(&Local)));

                write!(o, "    Job `{name}` (")?;

                let color = if failure {
                    &failure_color
                } else {
                    &success_color
                };

                o.set_color(color)?;
                write!(o, "{status}")?;
                o.reset()?;

                writeln!(o, "):")?;
                writeln!(o, "      url: {html_url}")?;
                writeln!(o, "      time: {started} - {completed}")?;
            }
        }

        if !ok {
            bail!("Status is not OK")
        }

        Ok(())
    }
}
