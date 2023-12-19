use core::fmt;
use std::io::Write;

use anyhow::{Context, Result};
use chrono::{DateTime, Local, TimeZone, Utc};
use clap::Parser;
use reqwest::{header, Client, IntoUrl, Method, RequestBuilder, StatusCode};
use serde::{de::IntoDeserializer, Deserialize};
use url::Url;

use crate::ctxt::Ctxt;
use crate::model::{Repo, RepoPath};
use crate::repo_sets::RepoSet;

/// GitHub base URL.
const API_URL: &str = "https://api.github.com";

#[derive(Debug, Default, Parser)]
pub(crate) struct Opts {
    /// Output raw JSON response.
    #[arg(long)]
    raw_json: bool,
    /// Limit number of workspace runs to inspect.
    #[arg(long, value_name = "number")]
    limit: Option<u32>,
    /// Include information on individual jobs.
    #[arg(long)]
    jobs: bool,
}

#[derive(Debug, Deserialize)]
struct Workflow {
    status: String,
    #[serde(default)]
    conclusion: Option<String>,
    head_branch: String,
    head_sha: String,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    jobs_url: Option<Url>,
}

#[derive(Debug, Deserialize)]
struct Job {
    name: String,
    status: String,
    #[serde(default)]
    conclusion: Option<String>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    html_url: Url,
}

#[derive(Debug, Deserialize)]
struct Jobs {
    jobs: Vec<Job>,
}

#[derive(Debug, Deserialize)]
struct WorkflowRuns {
    workflow_runs: Vec<Workflow>,
}

pub(crate) async fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let client = Client::builder().build()?;
    let today = Local::now();
    let limit = opts.limit.unwrap_or(1).max(1).to_string();

    let mut good = RepoSet::default();
    let mut bad = RepoSet::default();

    for repo in cx.repos() {
        let workflows = cx.config.workflows(repo)?;

        if workflows.is_empty() {
            continue;
        }

        let Some(repo_path) = repo.repo() else {
            continue;
        };

        println!("{}: {}", repo.path(), repo.url());

        let mut ok = true;

        for id in workflows.into_keys() {
            println!("Workflow `{id}`:");
            ok &= status(cx, &id, opts, repo, repo_path, today, &client, &limit).await?;
        }

        if ok {
            good.insert(repo);
        } else {
            bad.insert(repo);
        }
    }

    let hint = format!("status: {:?}", opts);
    cx.sets.save("good", good, &hint);
    cx.sets.save("bad", bad, &hint);
    Ok(())
}

#[tracing::instrument(skip_all, fields(source = ?repo.source(), path = repo.path().as_str()))]
async fn status(
    cx: &Ctxt<'_>,
    id: &str,
    opts: &Opts,
    repo: &Repo,
    repo_path: RepoPath<'_>,
    today: DateTime<Local>,
    client: &Client,
    limit: &str,
) -> Result<bool> {
    let current_dir = cx.to_path(repo.path());
    let sha;

    let sha = match &cx.git {
        Some(git) => {
            sha = git
                .rev_parse(&current_dir, "HEAD")
                .context("Getting head commit")?;
            Some(sha.trim())
        }
        None => None,
    };

    let url = format!(
        "{API_URL}/repos/{owner}/{name}/actions/workflows/{id}.yml/runs",
        owner = repo_path.owner,
        name = repo_path.name
    );

    let req = build_request(cx, client, url)
        .query(&[("exclude_pull_requests", "true"), ("per_page", limit)]);

    let res = req.send().await?;

    tracing::trace!("  {:?}", res.headers());

    if res.status() == StatusCode::NOT_FOUND {
        println!("  Workflow `{id}` not found");
        return Ok(false);
    }

    if !res.status().is_success() {
        println!("  {}: {}", res.status(), res.text().await?);
        return Ok(false);
    }

    let runs: serde_json::Value = res.json().await?;

    if opts.raw_json {
        let mut out = std::io::stdout();
        serde_json::to_writer_pretty(&mut out, &runs)?;
        writeln!(out)?;
    }

    let runs: WorkflowRuns = WorkflowRuns::deserialize(runs.into_deserializer())?;

    if runs.workflow_runs.is_empty() {
        println!("  No runs");
        return Ok(false);
    }

    let mut ok = true;

    for run in runs.workflow_runs {
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
                let res = build_request(cx, client, jobs_url.clone()).send().await?;

                if !res.status().is_success() {
                    println!("  {}", res.text().await?);
                    continue;
                }

                let jobs: Jobs = res.json().await?;

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
                        FormatTime::new(today, job.completed_at.map(|d| d.with_timezone(&Local)))
                    );
                }
            }
        }

        ok &= !failure;
    }

    Ok(ok)
}

fn build_request<U>(cx: &Ctxt<'_>, client: &Client, url: U) -> RequestBuilder
where
    U: IntoUrl,
{
    let req = client
        .request(Method::GET, url)
        .header(header::USER_AGENT, "udoprog projects");

    match &cx.github_auth {
        Some(auth) => req.header(header::AUTHORIZATION, &format!("Bearer {auth}")),
        None => req,
    }
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
