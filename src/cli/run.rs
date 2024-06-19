use std::io::Write;

use anyhow::{bail, Result};
use clap::Parser;
use termcolor::{ColorChoice, StandardStream};

use crate::command_system::{Colors, CommandSystem, RunOn};
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::shell::Shell;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
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
    /// Run the command using the specified execution methods.
    ///
    /// Available methods are:
    /// * `same` (default).
    /// * `wsl` - to run the command over WSL.
    #[arg(long)]
    run_on: Vec<RunOn>,
    /// Environment variables to pass to the command to run. Only specifying
    /// `<key>` means that the specified environment variable should be passed
    /// through.
    ///
    /// For WSL, this constructs the WSLENV environment variable, which dictates
    /// what environments are passed in.
    #[arg(long, short = 'E', value_name = "key[=value]")]
    env: Vec<String>,
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
    /// Print verbose information about the command being run.
    #[arg(long)]
    verbose: bool,
    /// Don't actually run any commands, just print what would be done.
    #[arg(long)]
    dry_run: bool,
    /// The default shell to use when printing command invocations.
    ///
    /// By default this is `bash` for unix-like environments and `powershell`
    /// for windows.
    #[arg(long, value_name = "<lavor")]
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

    let colors = Colors::new();

    with_repos!(
        cx,
        "run commands",
        format_args!("for: {opts:?}"),
        |cx, repo| { run(&mut o, &colors, cx, repo, opts) }
    );

    Ok(())
}

#[tracing::instrument(skip_all)]
fn run(
    o: &mut StandardStream,
    colors: &Colors,
    cx: &Ctxt<'_>,
    repo: &Repo,
    opts: &Opts,
) -> Result<()> {
    let repo_path = cx.to_path(repo.path());

    let mut system = CommandSystem::new(cx, colors);

    if opts.verbose {
        system.verbose();
    }

    if opts.dry_run {
        system.dry_run();
    }

    for i in &opts.ignore_matrix {
        system.ignore_matrix_variable(i);
    }

    for env in &opts.env {
        system.parse_env(env)?;
    }

    if let Some(workflow) = &opts.workflow {
        system.add_workflow(workflow);
    }

    if let Some(job) = &opts.job {
        system.add_job(job);
    }

    if opts.same_os {
        system.same_os();
    }

    if opts.workflow.is_some() || opts.job.is_some() || opts.list_jobs {
        let workflows = system.load_workflows(repo)?;

        if opts.list_jobs {
            for (workflow, jobs) in workflows {
                writeln!(o, "Workflow: {}", workflow.id())?;

                for job in jobs {
                    writeln!(o, "  Job: {}", job.name)?;
                }
            }
        }
    }

    if let [command, rest @ ..] = &opts.command[..] {
        system.add_command(command, rest);
    }

    if opts.first_os || opts.each_os {
        if opts.first_os && opts.each_os {
            bail!("Cannot specify both --first-os and --each-os");
        }

        let limit = if opts.first_os { 1 } else { usize::MAX };

        for os in cx.config.os(repo).into_iter().take(limit) {
            system.add_os(os)?;
        }
    }

    for &run_on in &opts.run_on {
        system.add_run_on(run_on)?;
    }

    let default_shell = opts.shell.unwrap_or_else(|| cx.os.shell());
    system.commit(o, &repo_path, default_shell)?;
    Ok(())
}
