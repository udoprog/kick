use core::fmt;
use std::io::Write;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Local, TimeZone, Utc};
use clap::Parser;
use reqwest::{header, Client, IntoUrl, Method, RequestBuilder};
use serde::{de::IntoDeserializer, Deserialize};
use url::Url;

use crate::{ctxt::Ctxt, git};

#[derive(Default, Parser)]
pub(crate) struct Opts {
    /// Filter by the specified modules.
    #[arg(long = "module", short = 'm', name = "module")]
    modules: Vec<String>,
    /// Output raw JSON response.
    #[arg(long)]
    raw_json: bool,
    /// Limit number of workspace runs to inspect.
    #[arg(long)]
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

pub(crate) async fn entry(cx: &Ctxt<'_>, opts: &Opts) -> Result<()> {
    let client = Client::builder().build()?;
    let today = Local::now();

    let limit = opts.limit.unwrap_or(1).max(1).to_string();

    for module in &cx.modules {
        if crate::should_skip(&opts.modules, module) {
            continue;
        }

        let (Some(path), Some((owner, repo))) = (module.path, module.repo().and_then(|repo| repo.split_once('/'))) else {
            continue;
        };

        let current_dir = path.to_path(cx.root);
        let sha =
            git::rev_parse(&current_dir, "HEAD").with_context(|| anyhow!("{}", module.name))?;
        let sha = sha.trim();

        let url =
            format!("https://api.github.com/repos/{owner}/{repo}/actions/workflows/ci.yml/runs");

        let req = build_request(cx, &client, url)
            .query(&[("exclude_pull_requests", "true"), ("per_page", &limit)]);

        if let Some(url) = &module.url {
            println!("{}: {url}", module.name);
        } else {
            println!("{}: *no url*", module.name);
        }

        let res = req.send().await?;

        tracing::trace!("  {:?}", res.headers());

        if !res.status().is_success() {
            println!("  {}", res.text().await?);
            continue;
        }

        let runs: serde_json::Value = res.json().await?;

        if opts.raw_json {
            let mut out = std::io::stdout();
            serde_json::to_writer_pretty(&mut out, &runs)?;
            writeln!(out)?;
        }

        let runs: WorkflowRuns = WorkflowRuns::deserialize(runs.into_deserializer())?;

        for run in runs.workflow_runs {
            let updated_at = FormatTime::new(today, Some(run.updated_at.with_timezone(&Local)));

            let head = if sha == run.head_sha { "* " } else { "  " };

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
                    let res = build_request(cx, &client, jobs_url.clone()).send().await?;

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
                            FormatTime::new(
                                today,
                                job.completed_at.map(|d| d.with_timezone(&Local))
                            )
                        );
                    }
                }
            }
        }
    }

    Ok(())
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
