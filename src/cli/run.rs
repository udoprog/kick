use std::collections::HashSet;
use std::io::Write;

use anyhow::{bail, Result};
use clap::Parser;
use termcolor::{ColorChoice, StandardStream};

use crate::command_system::{Batch, BatchConfig, BatchOptions, CommandSystem};
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::shell::Shell;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
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
    /// Matrix values to ignore when running a Github workflows job.
    #[arg(long, value_name = "value")]
    ignore_matrix: Vec<String>,
    /// Only runs command on the current OS.
    ///
    /// When loading workflows, this causes the `runs-on` directive to be
    /// effectively ignored.
    #[arg(long)]
    same_os: bool,
    /// The default shell to use when printing command invocations.
    ///
    /// By default this is `bash` for unix-like environments and `powershell`
    /// for windows.
    #[arg(long, value_name = "shell")]
    shell: Option<Shell>,
    /// Arguments to pass to the command to run.
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "command"
    )]
    command: Vec<String>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut o = StandardStream::stdout(ColorChoice::Auto);

    with_repos!(
        cx,
        "run commands",
        format_args!("for: {opts:?}"),
        |cx, repo| { run(&mut o, cx, repo, opts) }
    );

    Ok(())
}

#[tracing::instrument(skip_all)]
fn run(o: &mut StandardStream, cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let repo_path = cx.to_path(repo.path());

    let mut batches = Vec::new();
    let mut system = CommandSystem::new(cx);

    for i in &opts.ignore_matrix {
        system.ignore_matrix_variable(i);
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

    if opts.workflow.is_some() || opts.job.is_some() || opts.list_jobs {
        let mut workflows = system.load_repo_workflows(repo)?;
        workflows.synchronize(cx)?;

        if opts.list_jobs {
            for (workflow, jobs) in workflows.iter() {
                writeln!(o, "Workflow: {}", workflow.id())?;

                for job in jobs {
                    if let Some(name) = &job.name {
                        writeln!(o, "  Job: {} ({})", job.id, name)?;
                    } else {
                        writeln!(o, "  Job: {}", job.id)?;
                    }
                }
            }
        }

        if opts.workflow.is_some() || opts.job.is_some() {
            for (workflow, jobs) in workflows.iter() {
                if !all_workflows && !filter_workflows.contains(&workflow.id) {
                    continue;
                }

                for job in jobs {
                    if !all_jobs && !filter_jobs.contains(&job.id) {
                        continue;
                    }

                    for (matrix, steps) in &job.matrices {
                        match workflows.build_batch_from_step(cx, matrix, steps, opts.same_os) {
                            Ok(batch) => {
                                batches.push(batch);
                            }
                            Err(error) => {
                                tracing::warn!(
                                    ?workflow.id,
                                    ?job.id,
                                    ?matrix,
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

    if let [command, args @ ..] = &opts.command[..] {
        batches.push(Batch::command(command, args));
    }

    let default_shell = opts.shell.unwrap_or_else(|| cx.os.shell());
    let mut c = BatchConfig::new(cx, &repo_path, default_shell);
    c.add_opts(&opts.batch_opts)?;

    if opts.first_os || opts.each_os {
        if opts.first_os && opts.each_os {
            bail!("Cannot specify both --first-os and --each-os");
        }

        let limit = if opts.first_os { 1 } else { usize::MAX };

        for os in cx.config.os(repo).into_iter().take(limit) {
            c.add_os(os)?;
        }
    }

    for batch in batches {
        batch.commit(o, &c)?;
    }

    Ok(())
}
