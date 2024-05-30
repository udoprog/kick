use std::borrow::Cow;
use std::collections::{BTreeSet, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::path::Path;

use anyhow::{bail, ensure, Result};
use clap::Parser;
use nondestructive::yaml::Mapping;

use crate::config::Os;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::process::Command;
use crate::workflows::{Matrix, Workflows};
use crate::wsl::Wsl;

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
    /// Environment variables to pass to the command to run. Only specifying
    /// `<key>` means that the specified environment variable should be passed
    /// through.
    ///
    /// For WSL, this constructs the WSLENV environment variable, which dictates
    /// what environments are passed in.
    #[arg(long, short = 'E', value_name = "<key>[=<value>]")]
    env: Vec<String>,
    /// Run all commands associated with a Github workflows job.
    #[arg(long)]
    job: Option<String>,
    /// Matrix values to ignore when running a Github workflows job.
    #[arg(long, value_name = "<value>")]
    ignore_matrix: Vec<String>,
    /// Ignore `runs-on` specification in github workflow.
    #[arg(long)]
    ignore_runs_on: bool,
    /// Arguments to pass to the command to run.
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "command"
    )]
    command: Vec<OsString>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    with_repos!(
        cx,
        "run commands",
        format_args!("for: {opts:?}"),
        |cx, repo| { run(cx, repo, opts) }
    );

    Ok(())
}

#[tracing::instrument(skip_all)]
fn run(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let mut commands = Vec::new();

    let mut ignore = HashSet::new();

    for i in &opts.ignore_matrix {
        ignore.insert(i.clone());
    }

    if let Some(to_run) = &opts.job {
        let workflows = Workflows::new(cx, repo)?;

        for id in workflows.ids() {
            if let Some(workflow) = workflows.open(id)? {
                for (job_name, job) in workflow.jobs() {
                    if job_name != to_run {
                        continue;
                    }

                    let runs_on = job.runs_on()?;

                    for matrix in job.matrices(&ignore)? {
                        let runner = if opts.ignore_runs_on {
                            None
                        } else {
                            let runs_on = matrix.eval(runs_on);
                            let runs_on = runs_on.as_ref();

                            let os = match runs_on.split_once('-') {
                                Some(("ubuntu", _)) => Os::Linux,
                                Some(("windows", _)) => Os::Windows,
                                Some(("macos", _)) => Os::Mac,
                                _ => bail!("Unsupported runs-on: {runs_on}"),
                            };

                            let runner = RunnerKind::from_os(cx, &os)?;
                            Some(runner)
                        };

                        let Some(steps) =
                            job.value.get("steps").and_then(|steps| steps.as_sequence())
                        else {
                            continue;
                        };

                        for step in steps {
                            let Some(step) = step.as_mapping() else {
                                continue;
                            };

                            if let Some(rust_version) = extract_rust_version(&step, &matrix) {
                                commands.push(RunCommand {
                                    command: "rustup".into(),
                                    args: vec![
                                        OsString::from("default"),
                                        OsString::from(rust_version.as_ref()),
                                    ],
                                    runner,
                                    matrix: if matrix.is_empty() {
                                        None
                                    } else {
                                        Some(format!("{matrix:?}"))
                                    },
                                });
                            }

                            if let Some(run) = step.get("run").and_then(|run| run.as_str()) {
                                let run = matrix.eval(run);

                                let mut it = run.split_whitespace();

                                let Some(command) = it.next() else {
                                    continue;
                                };

                                let args = it.map(OsString::from).collect::<Vec<_>>();

                                commands.push(RunCommand {
                                    command: command.into(),
                                    args,
                                    runner,
                                    matrix: if matrix.is_empty() {
                                        None
                                    } else {
                                        Some(format!("{matrix:?}"))
                                    },
                                });
                            }
                        }
                    }
                }
            }
        }
    } else {
        let [command, rest @ ..] = &opts.command[..] else {
            bail!("No command specified");
        };

        commands.push(RunCommand {
            command: command.clone(),
            args: rest.to_vec(),
            runner: None,
            matrix: None,
        });
    }

    let mut argument_runners = Vec::new();

    if opts.first_os || opts.each_os {
        if opts.first_os && opts.each_os {
            bail!("Cannot specify both --first-os and --each-os");
        }

        let limit = if opts.first_os { 1 } else { usize::MAX };

        for os in cx.config.os(repo).into_iter().take(limit) {
            let runner = RunnerKind::from_os(cx, os)?;
            argument_runners.push(runner);
        }
    }

    let path = cx.to_path(repo.path());

    for run in commands {
        for runner in run.runners(&argument_runners) {
            let mut runner = runner.build(cx, opts, &path, &run)?;

            for e in &opts.env {
                if let Some((key, value)) = e.split_once('=') {
                    runner.command.env(key, value);
                }
            }

            if let Some((key, value)) = runner.extra_env {
                runner.command.env(key, value);
            }

            let name = Optional(runner.name);
            let matrix = Optional(run.matrix.as_deref());
            println!(
                "{}{name}{matrix}: {}",
                path.display(),
                runner.command.display()
            );

            let status = runner.command.status()?;
            ensure!(status.success(), status);
        }
    }

    Ok(())
}

/// Extract a rust version.
fn extract_rust_version<'a>(step: &Mapping<'a>, matrix: &Matrix) -> Option<Cow<'a, str>> {
    let uses = step.get("uses")?.as_str()?;
    let uses = matrix.eval(uses);

    let (head, version) = uses.split_once('@')?;

    let (_, "rust-toolchain") = head.split_once('/')? else {
        return None;
    };

    let "master" = version else {
        return Some(Cow::Owned(version.to_owned()));
    };

    let with = step.get("with")?.as_mapping()?;
    let toolchain = with.get("toolchain")?.as_str()?;
    Some(matrix.eval(toolchain))
}

struct RunCommand {
    command: OsString,
    args: Vec<OsString>,
    runner: Option<RunnerKind>,
    matrix: Option<String>,
}

impl RunCommand {
    fn runners(&self, opts: &[RunnerKind]) -> BTreeSet<RunnerKind> {
        let mut set = BTreeSet::new();
        set.extend(opts.iter().copied());
        set.extend(self.runner);

        if set.is_empty() {
            set.insert(RunnerKind::Same);
        }

        set
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum RunnerKind {
    Same,
    Wsl,
}

impl RunnerKind {
    fn from_os(cx: &Ctxt<'_>, os: &Os) -> Result<Self> {
        if cx.os == *os {
            return Ok(Self::Same);
        }

        if cx.os == Os::Windows && *os == Os::Linux && cx.system.wsl.first().is_some() {
            return Ok(Self::Wsl);
        }

        bail!(
            "No supported runner for {os:?} on current system {:?}",
            cx.os
        );
    }

    fn build(&self, cx: &Ctxt, opts: &Opts, path: &Path, command: &RunCommand) -> Result<Runner> {
        match *self {
            Self::Same => Ok(setup_same(path, command)),
            Self::Wsl => {
                let Some(wsl) = cx.system.wsl.first() else {
                    bail!("No WSL available");
                };

                Ok(setup_wsl(path, wsl, opts, command))
            }
        }
    }
}

struct Runner {
    command: Command,
    name: Option<&'static str>,
    extra_env: Option<(&'static str, String)>,
}

impl Runner {
    fn new(command: Command) -> Self {
        Self {
            command,
            name: None,
            extra_env: None,
        }
    }
}

fn setup_same(path: &Path, run: &RunCommand) -> Runner {
    let mut c = Command::new(&run.command);
    c.args(&run.args);
    c.current_dir(path);
    Runner::new(c)
}

fn setup_wsl(path: &Path, wsl: &Wsl, opts: &Opts, run: &RunCommand) -> Runner {
    let mut c = wsl.shell(path);
    c.arg(&run.command);
    c.args(&run.args);

    let mut wslenv = String::new();

    for e in &opts.env {
        if !wslenv.is_empty() {
            wslenv.push(':');
        }

        if let Some((key, _)) = e.split_once('=') {
            wslenv.push_str(key);
        } else {
            wslenv.push_str(e);
        }
    }

    let mut runner = Runner::new(c);
    runner.name = Some("WSL");
    runner.extra_env = Some(("WSLENV", wslenv));
    runner
}

struct Optional<T>(Option<T>);

impl<T> fmt::Display for Optional<T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(value) = &self.0 {
            write!(f, " {}", value)?;
        }

        Ok(())
    }
}
