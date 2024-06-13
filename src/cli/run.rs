use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Write;
use std::path::Path;
use std::str;

use anyhow::{anyhow, bail, ensure, Context, Result};
use clap::Parser;
use nondestructive::yaml::Mapping;
use relative_path::RelativePathBuf;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use crate::config::Os;
use crate::ctxt::Ctxt;
use crate::model::{Repo, ShellFlavor};
use crate::process::Command;
use crate::system::Wsl;
use crate::workflows::{Eval, Job, Matrix, Workflow, Workflows};

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
    /// Shell flavor to use for local shell.
    ///
    /// By default this is `bash` for unix-like environments and `powershell`
    /// for windows.
    #[arg(long, value_name = "<lavor")]
    flavor: Option<ShellFlavor>,
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
    let mut batches = Vec::new();

    let mut ignore = HashSet::new();

    for i in &opts.ignore_matrix {
        ignore.insert(i.clone());
    }

    let mut jobs = HashSet::new();
    jobs.extend(opts.job.clone());

    if !jobs.is_empty() {
        let workflows = Workflows::new(cx, repo)?;

        for id in workflows.ids() {
            let Some(workflow) = workflows.open(id)? else {
                continue;
            };

            workflow_to_batches(
                cx,
                &mut batches,
                &workflow,
                &jobs,
                &ignore,
                opts.ignore_runs_on,
            )
            .with_context(|| {
                anyhow!("{}: Workflow `{id}`", cx.to_path(&workflow.path).display())
            })?;
        }
    }

    if let [command, rest @ ..] = &opts.command[..] {
        batches.push(CommandBatch {
            commands: vec![RunCommand {
                name: None,
                run: Run::Command {
                    command: command.clone(),
                    args: rest.to_vec(),
                },
                env: BTreeMap::new(),
                skipped: None,
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
            write!(o, "# In ")?;

            o.set_color(&colors.title)?;
            write!(o, "{}", path.display())?;
            o.reset()?;

            if let Some(name) = runner.name() {
                write!(o, " using ")?;

                o.set_color(&colors.title)?;
                write!(o, "{name}")?;
                o.reset()?;
            }

            if let Some(matrix) = &batch.matrix {
                write!(o, " ")?;

                o.set_color(&colors.matrix)?;
                write!(o, "{}", matrix.display())?;
                o.reset()?;
            }

            writeln!(o)?;

            for (index, run) in batch.commands.iter().enumerate() {
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

                if let Some(name) = &run.name {
                    write!(o, "# ")?;
                    o.set_color(&colors.title)?;
                    write!(o, "{name}")?;
                    o.reset()?;
                    write!(o, ": ")?;
                } else {
                    write!(o, "# ")?;
                    o.set_color(&colors.title)?;
                    write!(o, "Step {} / {}", index + 1, batch.commands.len())?;
                    o.reset()?;
                    write!(o, ": ")?;
                }

                write!(o, "{}", runner.command.display_with(flavor))?;

                if let Some(skipped) = &run.skipped {
                    write!(o, " (Skipped: ")?;
                    o.set_color(&colors.skip_cond)?;
                    write!(o, "{skipped}")?;
                    o.reset()?;
                    write!(o, ")")?;
                }

                writeln!(o)?;

                if opts.verbose || opts.dry_run {
                    match &flavor {
                        ShellFlavor::Sh => {
                            for (key, value) in &runner.command.env {
                                write!(
                                    o,
                                    r#"{}="{}" "#,
                                    key.to_string_lossy(),
                                    value.to_string_lossy()
                                )?;
                            }

                            write!(o, "{}", runner.command.display_with(flavor))?;
                            writeln!(o)?;
                        }
                        ShellFlavor::Powershell => {
                            if !runner.command.env.is_empty() {
                                writeln!(o, "powershell -Command {{")?;

                                for (key, value) in &runner.command.env {
                                    writeln!(
                                        o,
                                        r#"  $Env:{}="{}";"#,
                                        key.to_string_lossy(),
                                        value.to_string_lossy()
                                    )?;
                                }

                                writeln!(o, "  {}", runner.command.display_with(flavor))?;
                                writeln!(o, "}}")?;
                            } else {
                                write!(o, "{}", runner.command.display_with(flavor))?;
                                writeln!(o)?;
                            }
                        }
                    }
                }

                if run.skipped.is_none() && !opts.dry_run {
                    let status = runner.command.status()?;
                    ensure!(status.success(), status);
                }
            }
        }
    }

    Ok(())
}

fn job_to_batches(
    cx: &Ctxt<'_>,
    batches: &mut Vec<CommandBatch>,
    job: &Job<'_>,
    ignore: &HashSet<String>,
    ignore_runs_on: bool,
    eval: &Eval<'_>,
) -> Result<()> {
    let runs_on = job.runs_on()?;

    for matrix in job.matrices(ignore)? {
        let eval = eval.with_matrix(&matrix);

        let runner = if ignore_runs_on {
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

            let runner = match RunnerKind::from_os(cx, &os) {
                Ok(runner) => runner,
                Err(error) => {
                    tracing::warn!("{error}");
                    continue;
                }
            };

            Some(runner)
        };

        let Some(steps) = job.value.get("steps").and_then(|steps| steps.as_sequence()) else {
            continue;
        };

        let mut commands = Vec::new();

        let mut env = eval.env();
        env.extend(extract_env(&eval, &job.value)?);
        // Update environment.
        let eval = eval.with_env(&env);

        for step in steps {
            let Some(step) = step.as_mapping() else {
                continue;
            };

            let mut env = eval.env();
            env.extend(extract_env(&eval, &step)?);
            // Update environment.
            let eval = eval.with_env(&env);

            let working_directory = match step.get("working-directory").and_then(|v| v.as_str()) {
                Some(dir) => Some(RelativePathBuf::from(eval.eval(dir)?.into_owned())),
                None => None,
            };

            let mut skipped = None;

            if let Some(expr) = step.get("if").and_then(|v| v.as_str()) {
                if !eval.test(expr)? {
                    skipped = Some(expr.to_owned());
                }
            }

            if let Some(rust_toolchain) = rust_toolchain(&step, &eval)? {
                let Some(rust_version) = rust_toolchain.version else {
                    bail!("uses: */rust-toolchain is specified, but cannot determine version")
                };

                if rust_toolchain.components.is_some() || rust_toolchain.targets.is_some() {
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
                        skipped: skipped.clone(),
                        working_directory: None,
                    });
                }

                commands.push(RunCommand {
                    name: None,
                    run: Run::Command {
                        command: "rustup".into(),
                        args: vec![String::from("default"), String::from(rust_version.as_ref())],
                    },
                    env: BTreeMap::new(),
                    skipped: skipped.clone(),
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
                    env: eval.env(),
                    skipped,
                    working_directory,
                });
            }
        }

        batches.push(CommandBatch {
            commands,
            runner,
            matrix: if !matrix.is_empty() {
                Some(matrix)
            } else {
                None
            },
        })
    }

    Ok(())
}

/// Convert a workflow into batches.
fn workflow_to_batches(
    cx: &Ctxt<'_>,
    batches: &mut Vec<CommandBatch>,
    workflow: &Workflow,
    jobs: &HashSet<String>,
    ignore: &HashSet<String>,
    ignore_runs_on: bool,
) -> Result<()> {
    let eval = Eval::new();
    let env = workflow.env(&eval)?;
    let eval = eval.with_env(&env);

    for (job_name, job) in workflow.jobs() {
        let job_name = str::from_utf8(job_name).context("Bad job name")?;

        if !jobs.contains(job_name) {
            continue;
        }

        job_to_batches(cx, batches, &job, ignore, ignore_runs_on, &eval)
            .with_context(|| anyhow!("Job `{job_name}`"))?;
    }

    Ok(())
}

fn default_flavor(os: &Os) -> ShellFlavor {
    match os {
        Os::Windows => ShellFlavor::Powershell,
        _ => ShellFlavor::Sh,
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
    matrix: Option<Matrix>,
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
    skipped: Option<String>,
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
                    bail!("WSL not available");
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
                    bail!("PowerShell not available");
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

struct Colors {
    skip_cond: ColorSpec,
    title: ColorSpec,
    matrix: ColorSpec,
}

impl Colors {
    fn new() -> Self {
        let mut skip_cond = ColorSpec::new();
        skip_cond.set_fg(Some(Color::Red));
        skip_cond.set_bold(true);

        let mut title = ColorSpec::new();
        title.set_fg(Some(Color::White));
        title.set_bold(true);

        let mut matrix = ColorSpec::new();
        matrix.set_fg(Some(Color::Yellow));

        Self {
            skip_cond,
            title,
            matrix,
        }
    }
}
