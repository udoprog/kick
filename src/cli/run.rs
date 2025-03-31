use std::collections::HashSet;
use std::io::Write;

use anyhow::{bail, Result};
use clap::Parser;
use termcolor::{ColorChoice, StandardStream};

use crate::cli::WithRepos;
use crate::commands::{Batch, BatchOptions, Session};
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::release::{Date, ReleaseOpts, Version};

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Command to run.
    #[arg(value_name = "command")]
    command: Option<String>,
    /// Arguments to pass to the run command.
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "args"
    )]
    args: Vec<String>,
    #[command(flatten)]
    batch_opts: BatchOptions,
    /// If the specified operating system is different for the repo, execute the
    /// command using a compatibility layer which is appropriate for a supported
    /// operating system.
    ///
    /// * On Windows, if a project is Linux-specific WSL will be used.
    #[arg(long)]
    first_os: bool,
    /// Executes the command over all supported operating systems.
    #[arg(long)]
    each_os: bool,
    /// Run commands associated with a Github workflow.
    #[arg(long)]
    workflow: Option<String>,
    /// Run all commands associated with a Github workflows job.
    #[arg(long)]
    job: Option<String>,
    /// List all jobs associated with a Github workflows.
    #[arg(long)]
    list_jobs: bool,
    /// Only runs command on the current OS.
    ///
    /// When loading workflows, this causes the `runs-on` directive to be
    /// effectively ignored.
    #[arg(long)]
    same_os: bool,
    /// Environment variable to set containing the version string if a version
    /// is specified.
    ///
    /// This defaults to KICK_VERSION.
    #[arg(long)]
    version_env: Option<String>,
    /// The version produced should be in MSI format.
    #[arg(long)]
    version_msi: bool,
    #[clap(flatten)]
    release: ReleaseOpts,
}

pub(crate) fn entry<'repo>(with_repos: impl WithRepos<'repo>, opts: &Opts) -> Result<()> {
    let today = Date::today()?;
    let version = opts.release.try_env_argument(with_repos.cx().env, today)?;
    let version_env = opts.version_env.as_deref().unwrap_or("KICK_VERSION");
    let version = version.as_ref().map(|version| (version_env, version));

    let mut o = StandardStream::stdout(ColorChoice::Auto);

    with_repos.run("run commands", format_args!("for: {opts:?}"), |cx, repo| {
        run(&mut o, cx, repo, opts, version)
    })?;

    Ok(())
}

#[tracing::instrument(skip_all)]
fn run(
    o: &mut StandardStream,
    cx: &Ctxt<'_>,
    repo: &Repo,
    opts: &Opts,
    version: Option<(&str, &Version)>,
) -> Result<()> {
    let mut c = opts.batch_opts.build(cx, repo)?;

    if let Some((env, version)) = version {
        if opts.version_msi {
            c.set_env(env, version.msi_version()?);
        } else {
            c.set_env(env, version.to_string());
        }
    }

    let mut all_workflows = false;
    let mut filter_workflows = HashSet::new();
    let mut all_jobs = false;
    let mut filter_jobs = HashSet::new();

    if let Some(workflow) = &opts.workflow {
        filter_workflows.insert(workflow.clone());
        all_jobs = true;
    }

    if let Some(job) = &opts.job {
        filter_jobs.insert(job.clone());
        // NB: If no workflow is specified we must enable all workflows to
        // ensure that the specified job is run.
        all_workflows = opts.workflow.is_none();
        all_jobs = false;
    }

    if opts.first_os || opts.each_os {
        if opts.first_os && opts.each_os {
            bail!("Cannot specify both --first-os and --each-os");
        }

        let limit = if opts.first_os { 1 } else { usize::MAX };

        for os in cx.config.os(repo).into_iter().take(limit) {
            c.add_os(os)?;
        }
    }

    let mut batches = Vec::new();

    if let Some(command) = &opts.command {
        batches.push(Batch::command(cx.current_os.clone(), command, &opts.args));
    }

    if opts.workflow.is_some() || opts.job.is_some() || opts.list_jobs {
        let w = c.load_github_workflows(repo)?;

        if opts.list_jobs {
            for workflow in w.iter() {
                writeln!(o, "Workflow: {}", workflow.id())?;

                for job in workflow.jobs() {
                    for matrix in job.matrices() {
                        write!(o, "  Job: {}", job.id())?;

                        if let Some(name) = matrix.name() {
                            if name.to_exposed().as_ref() != job.id() {
                                write!(o, " ({})", name)?;
                            }
                        }

                        if matrix.matrix().is_empty() {
                            writeln!(o)?;
                        } else {
                            writeln!(o, " {}", matrix.matrix().display())?;
                        }
                    }
                }
            }
        }

        if opts.workflow.is_some() || opts.job.is_some() {
            for workflow in w.iter() {
                if !all_workflows && !filter_workflows.contains(workflow.id()) {
                    continue;
                }

                for job in workflow.jobs() {
                    if !all_jobs && !filter_jobs.contains(job.id()) {
                        continue;
                    }

                    for matrix in job.matrices() {
                        match matrix.build(None, opts.same_os, &cx.current_os) {
                            Ok(batch) => {
                                batches.push(batch);
                            }
                            Err(error) => {
                                tracing::warn!(
                                    workflow.id = workflow.id(),
                                    job.id = job.id(),
                                    matrix = ?matrix.matrix(),
                                    ?error,
                                    "Failed to build job",
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    let mut session = Session::new(&c);

    for batch in batches {
        batch.commit(o, &c, &mut session)?;
    }

    Ok(())
}
