use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::path::Path;

use anyhow::{bail, ensure, Result};
use clap::{Parser, ValueEnum};
use nondestructive::yaml::Mapping;
use relative_path::RelativePathBuf;

use crate::config::Os;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::process::Command;
use crate::system::Wsl;
use crate::workflows::{Eval, Workflows};

#[derive(Default, Debug, Clone, Copy, ValueEnum)]
enum Flavor {
    #[default]
    Sh,
    Powershell,
}

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
    #[arg(long, short = 'E', value_name = "key[=value]")]
    env: Vec<String>,
    /// Run all commands associated with a Github workflows job.
    #[arg(long)]
    job: Option<String>,
    /// Matrix values to ignore when running a Github workflows job.
    #[arg(long, value_name = "value")]
    ignore_matrix: Vec<String>,
    /// Ignore `runs-on` specification in github workflow.
    #[arg(long)]
    ignore_runs_on: bool,
    /// Print verbose information about the command being run.
    #[arg(long)]
    verbose: bool,
    /// Don't actually run any commands, just print what would be done.
    #[arg(long)]
    dry_run: bool,
    /// Shell flavor for verbose output.
    #[arg(long, value_name = "<lavor")]
    flavor: Option<Flavor>,
    /// Arguments to pass to the command to run.
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "command"
    )]
    command: Vec<String>,
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
    let mut batches = Vec::new();

    let mut ignore = HashSet::new();

    for i in &opts.ignore_matrix {
        ignore.insert(i.clone());
    }

    if let Some(to_run) = &opts.job {
        let workflows = Workflows::new(cx, repo)?;

        for id in workflows.ids() {
            if let Some(workflow) = workflows.open(id)? {
                let env = workflow.env();

                for (job_name, job) in workflow.jobs() {
                    if job_name != to_run {
                        continue;
                    }

                    let runs_on = job.runs_on()?;

                    for matrix in job.matrices(&ignore)? {
                        let eval = Eval::new(&env, &matrix);

                        let runner = if opts.ignore_runs_on {
                            None
                        } else {
                            let runs_on = eval.eval(runs_on)?;
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

                        let mut commands = Vec::new();

                        let mut env = env.clone();
                        env.extend(extract_env(&eval, &job.value)?);

                        // Update environment.
                        let eval = Eval::new(&env, eval.matrix);

                        for step in steps {
                            let Some(step) = step.as_mapping() else {
                                continue;
                            };

                            let mut env = env.clone();
                            env.extend(extract_env(&eval, &step)?);

                            // Update environment.
                            let eval = Eval::new(&env, eval.matrix);

                            let working_directory =
                                match step.get("working-directory").and_then(|v| v.as_str()) {
                                    Some(dir) => {
                                        Some(RelativePathBuf::from(eval.eval(dir)?.into_owned()))
                                    }
                                    None => None,
                                };

                            let mut skipped = false;

                            if let Some(expr) = step.get("if").and_then(|v| v.as_str()) {
                                skipped = !eval.test(expr)?;
                            }

                            if let Some(rust_toolchain) = rust_toolchain(&step, &eval)? {
                                let Some(rust_version) = rust_toolchain.version else {
                                    bail!("uses: */rust-toolchain is specified, but cannot determine version")
                                };

                                if rust_toolchain.components.is_some()
                                    || rust_toolchain.targets.is_some()
                                {
                                    let mut args = vec![
                                        String::from("toolchain"),
                                        String::from("install"),
                                        String::from(rust_version.as_ref()),
                                    ];

                                    if let Some(c) = rust_toolchain.components {
                                        args.push(String::from("-c"));
                                        args.push(String::from(c.as_ref()));
                                    }

                                    if let Some(t) = rust_toolchain.targets {
                                        args.push(String::from("-t"));
                                        args.push(String::from(t.as_ref()));
                                    }

                                    args.extend([
                                        String::from("--profile"),
                                        String::from("minimal"),
                                        String::from("--no-self-update"),
                                    ]);

                                    commands.push(RunCommand {
                                        name: None,
                                        run: Run::Command {
                                            command: "rustup".into(),
                                            args,
                                        },
                                        env: BTreeMap::new(),
                                        skipped,
                                        working_directory: None,
                                    });
                                }

                                commands.push(RunCommand {
                                    name: None,
                                    run: Run::Command {
                                        command: "rustup".into(),
                                        args: vec![
                                            String::from("default"),
                                            String::from(rust_version.as_ref()),
                                        ],
                                    },
                                    env: BTreeMap::new(),
                                    skipped,
                                    working_directory: None,
                                });
                            }

                            if let Some(run) = step.get("run").and_then(|run| run.as_str()) {
                                let name = match step.get("name").and_then(|v| v.as_str()) {
                                    Some(name) => Some(eval.eval(name)?.into_owned()),
                                    None => None,
                                };

                                let script = eval.eval(run)?.into_owned();

                                commands.push(RunCommand {
                                    name,
                                    run: Run::Shell { script },
                                    env,
                                    skipped,
                                    working_directory,
                                });
                            }
                        }

                        batches.push(CommandBatch {
                            commands,
                            runner,
                            matrix: if matrix.is_empty() {
                                None
                            } else {
                                Some(format!("{matrix:?}"))
                            },
                        })
                    }
                }
            }
        }
    } else {
        let [command, rest @ ..] = &opts.command[..] else {
            bail!("No command specified");
        };

        batches.push(CommandBatch {
            commands: vec![RunCommand {
                name: None,
                run: Run::Command {
                    command: command.clone(),
                    args: rest.to_vec(),
                },
                env: BTreeMap::new(),
                skipped: false,
                working_directory: None,
            }],
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

    let flavor = opts.flavor.unwrap_or_else(|| default_flavor(&cx.os));

    let path = cx.to_path(repo.path());

    for batch in batches {
        for runner in batch.runners(&argument_runners) {
            let name = Opt(" ", runner.name(), "");
            let matrix = Opt(" ", batch.matrix.as_deref(), "");
            println!("# {}:{name}{matrix}", path.display());

            for run in &batch.commands {
                let modified;

                let path = match &run.working_directory {
                    Some(working_directory) => {
                        modified = working_directory.to_logical_path(&path);
                        &modified
                    }
                    None => &path,
                };

                let mut runner = runner.build(cx, opts, path, run)?;

                for e in &opts.env {
                    if let Some((key, value)) = e.split_once('=') {
                        runner.command.env(key, value);
                    }
                }

                for (key, value) in &run.env {
                    runner.command.env(key, value);
                }

                if let Some((key, value)) = runner.extra_env {
                    if !value.is_empty() {
                        runner.command.env(key, value);
                    }
                }

                let skipped = Opt(" ", run.skipped.then_some(" (skipped)"), "");

                if let Some(name) = &run.name {
                    println!("# {name}{skipped}");
                } else {
                    println!("# {}{skipped}", runner.command.display());
                }

                if opts.verbose || opts.dry_run {
                    match &flavor {
                        Flavor::Sh => {
                            for (key, value) in &runner.command.env {
                                println!(
                                    r#"export {}="{}""#,
                                    key.to_string_lossy(),
                                    value.to_string_lossy()
                                );
                            }

                            println!("{}", runner.command.display());
                        }
                        Flavor::Powershell => {
                            if !runner.command.env.is_empty() {
                                println!("powershell -Command {{");

                                for (key, value) in &runner.command.env {
                                    println!(
                                        r#"  $env:{}="{}";"#,
                                        key.to_string_lossy(),
                                        value.to_string_lossy()
                                    );
                                }

                                println!("  {}", runner.command.display());
                                println!("}}");
                            } else {
                                println!("{}", runner.command.display());
                            }
                        }
                    }
                }

                if !run.skipped && !opts.dry_run {
                    let status = runner.command.status()?;
                    ensure!(status.success(), status);
                }
            }
        }
    }

    Ok(())
}

fn default_flavor(os: &Os) -> Flavor {
    match os {
        Os::Windows => Flavor::Powershell,
        _ => Flavor::Sh,
    }
}

struct RustToolchain<'a> {
    version: Option<Cow<'a, str>>,
    components: Option<Cow<'a, str>>,
    targets: Option<Cow<'a, str>>,
}

/// Extract a rust version from a `rust-toolchain` job.
fn rust_toolchain<'a>(step: &Mapping<'a>, eval: &Eval<'_>) -> Result<Option<RustToolchain<'a>>> {
    let Some(uses) = step.get("uses").and_then(|v| v.as_str()) else {
        return Ok(None);
    };

    let uses = eval.eval(uses)?;

    let Some((head, version)) = uses.split_once('@') else {
        return Ok(None);
    };

    let Some((_, "rust-toolchain")) = head.split_once('/') else {
        return Ok(None);
    };

    let version = match extract_with(step, eval, "toolchain")? {
        Some(toolchain) => Some(toolchain),
        None => Some(Cow::Owned(version.to_owned())),
    };

    let components = extract_with(step, eval, "components")?;
    let targets = extract_with(step, eval, "targets")?;

    Ok(Some(RustToolchain {
        version,
        components,
        targets,
    }))
}

/// Extract explicitly specified toolchain version.
fn extract_with<'a>(
    step: &Mapping<'a>,
    eval: &Eval<'_>,
    key: &str,
) -> Result<Option<Cow<'a, str>>> {
    let Some(with) = step.get("with").and_then(|v| v.as_mapping()) else {
        return Ok(None);
    };

    let Some(components) = with.get(key).and_then(|v| v.as_str()) else {
        return Ok(None);
    };

    Ok(Some(eval.eval(components)?))
}

fn extract_env(eval: &Eval<'_>, m: &Mapping<'_>) -> Result<BTreeMap<String, String>> {
    let mut env = BTreeMap::new();

    let Some(m) = m.get("env").and_then(|v| v.as_mapping()) else {
        return Ok(env);
    };

    for (key, value) in m {
        let Some(value) = value.as_str() else {
            continue;
        };

        let value = eval.eval(value)?;
        env.insert(key.to_string(), value.into_owned());
    }

    Ok(env)
}

struct CommandBatch {
    commands: Vec<RunCommand>,
    runner: Option<RunnerKind>,
    matrix: Option<String>,
}

impl CommandBatch {
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

enum Run {
    Shell { script: String },
    Command { command: String, args: Vec<String> },
}

struct RunCommand {
    name: Option<String>,
    run: Run,
    env: BTreeMap<String, String>,
    skipped: bool,
    working_directory: Option<RelativePathBuf>,
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
            Self::Same => setup_same(cx, path, command, &cx.os),
            Self::Wsl => {
                let Some(wsl) = cx.system.wsl.first() else {
                    bail!("No WSL available");
                };

                Ok(setup_wsl(path, wsl, opts, command))
            }
        }
    }

    fn name(&self) -> Option<&str> {
        match *self {
            Self::Same => None,
            Self::Wsl => Some("WSL"),
        }
    }
}

struct Runner {
    command: Command,
    extra_env: Option<(&'static str, String)>,
}

impl Runner {
    fn new(command: Command) -> Self {
        Self {
            command,
            extra_env: None,
        }
    }
}

fn setup_same(cx: &Ctxt, path: &Path, run: &RunCommand, os: &Os) -> Result<Runner> {
    match &run.run {
        Run::Shell { script } => match os {
            Os::Windows => {
                let Some(powershell) = cx.system.powershell.first() else {
                    bail!("No powershell available");
                };

                Ok(Runner::new(powershell.command(path, script)))
            }
            Os::Linux | Os::Mac => {
                let mut c = Command::new("bash");
                c.args(["-i", "-c", script]);
                c.current_dir(path);
                Ok(Runner::new(c))
            }
            Os::Other(..) => bail!("Cannot run shell script on {os:?}"),
        },
        Run::Command { command, args } => {
            let mut c = Command::new(command);
            c.args(args);
            c.current_dir(path);
            Ok(Runner::new(c))
        }
    }
}

fn setup_wsl(path: &Path, wsl: &Wsl, opts: &Opts, run: &RunCommand) -> Runner {
    let mut c = wsl.shell(path);

    match &run.run {
        Run::Shell { script } => {
            c.args(["bash", "-i", "-c", script]);
        }
        Run::Command { command, args } => {
            c.arg(command);
            c.args(args);
        }
    }

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

    for e in run.env.keys() {
        if !wslenv.is_empty() {
            wslenv.push(':');
        }

        wslenv.push_str(e);
    }

    let mut runner = Runner::new(c);
    runner.extra_env = Some(("WSLENV", wslenv));
    runner
}

struct Opt<T>(&'static str, Option<T>, &'static str);

impl<T> fmt::Display for Opt<T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Self(prefix, Some(ref value), suffix) = *self {
            write!(f, "{prefix}{value}{suffix}")?;
        }

        Ok(())
    }
}
