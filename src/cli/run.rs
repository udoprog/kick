use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufReader, Write};
use std::path::Path;
use std::rc::Rc;
use std::str;

use anyhow::{anyhow, bail, ensure, Context, Result};
use bstr::BString;
use clap::Parser;
use gix::ObjectId;
use relative_path::RelativePath;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use crate::config::Os;
use crate::ctxt::Ctxt;
use crate::github_action::GithubActionKind;
use crate::model::{Repo, ShellFlavor};
use crate::process::{Command, OsArg};
use crate::rstr::{RStr, RString};
use crate::system::Wsl;
use crate::workflows::{Eval, Job, Matrix, Step, Tree, Workflow, Workflows};

const GIT_OBJECT_ID_FILE: &str = ".git-object-id";
const WORKDIR: &str = "workdir";
const ENVS: &str = "envs";

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
    /// Only run expanded commands on the same operating system. If you're
    /// running any workflow this will ignore any commands which are not
    /// scheduled to run on the same operating system as kick is run on.
    #[arg(long)]
    same_os: bool,
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
    let repo_path = cx.to_path(repo.path());

    let mut batches = Vec::new();
    let mut ignore = HashSet::new();

    for i in &opts.ignore_matrix {
        ignore.insert(i.clone());
    }

    let mut jobs = HashSet::new();
    jobs.extend(opts.job.clone().map(RString::from));

    let default_flavor = opts
        .flavor
        .unwrap_or_else(|| default_os_shell_flavor(&cx.os));

    let mut uses = BTreeMap::<_, BTreeSet<_>>::new();

    if !jobs.is_empty() {
        let workflows = Workflows::new(cx, repo)?;

        let mut all = Vec::new();

        for workflow in workflows.workflows() {
            let workflow = workflow?;

            for job in workflow.jobs(&ignore)? {
                if !jobs.contains(&job.id) {
                    continue;
                }

                for (_, steps) in &job.matrices {
                    for step in &steps.steps {
                        if let Some((_, name)) = &step.uses {
                            let name = name.to_redacted();

                            let Some((name, version)) = name.split_once('@') else {
                                continue;
                            };

                            uses.entry(name.to_owned())
                                .or_default()
                                .insert(version.to_owned());
                        }
                    }
                }
            }

            all.push(workflow);
        }

        let runners = sync_runners(cx, &uses)?;

        for workflow in &all {
            workflow_to_batches(
                cx,
                &mut batches,
                workflow,
                &jobs,
                &ignore,
                opts.ignore_runs_on,
                &runners,
                default_flavor,
            )
            .with_context(|| {
                anyhow!(
                    "{}: Workflow `{}`",
                    cx.to_path(&workflow.path).display(),
                    workflow.id()
                )
            })?;
        }
    }

    if let [command, rest @ ..] = &opts.command[..] {
        batches.push(CommandBatch {
            commands: vec![RunCommand::command(command, rest)],
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

    for batch in batches {
        for runner in batch.runners(&argument_runners, opts.same_os) {
            write!(o, "# In ")?;

            o.set_color(&colors.title)?;
            write!(o, "{}", repo_path.display())?;
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

            let mut current_env = BTreeMap::new();

            for (index, run) in batch.commands.iter().enumerate() {
                let flavor = run.shell().unwrap_or(default_flavor);

                let modified;

                let path = match &run.working_directory {
                    Some(working_directory) => {
                        let working_directory = working_directory.to_redacted();
                        let working_directory = RelativePath::new(working_directory.as_ref());
                        modified = working_directory.to_logical_path(&repo_path);
                        &modified
                    }
                    None => &repo_path,
                };

                let mut runner = runner.build(cx, opts, path, run, &current_env)?;

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
                    write!(o, " (Skip: ")?;
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
                    if let Some(env_file) = &run.env_file {
                        let file = File::create(env_file).with_context(|| {
                            anyhow!(
                                "Failed to create temporary environment file: {}",
                                env_file.display()
                            )
                        })?;

                        file.set_len(0).with_context(|| {
                            anyhow!(
                                "Failed to truncate temporary environment file: {}",
                                env_file.display()
                            )
                        })?;
                    }

                    let status = runner.command.status()?;
                    ensure!(status.success(), status);

                    if let Some(env_file) = &run.env_file {
                        if let Ok(contents) = fs::read(env_file) {
                            for (key, value) in parse_env(&contents)? {
                                current_env.insert(key, value);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn parse_env(contents: &[u8]) -> Result<Vec<(String, String)>> {
    use bstr::ByteSlice;
    use std::io::BufRead;

    let mut out = Vec::new();

    let mut reader = BufReader::new(contents);

    let mut line = Vec::new();

    loop {
        line.clear();

        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }

        let line = line.trim_end();

        if let Some((key, value)) = line.split_once_str("=") {
            let (Ok(key), Ok(value)) = (str::from_utf8(key), str::from_utf8(value)) else {
                continue;
            };

            out.push((key.to_owned(), value.to_owned()));
        }
    }

    Ok(out)
}

fn job_to_batches(
    cx: &Ctxt<'_>,
    batches: &mut Vec<CommandBatch>,
    job: &Job,
    ignore_runs_on: bool,
    runners: &ActionRunners,
    default_flavor: ShellFlavor,
) -> Result<()> {
    'outer: for (matrix, steps) in &job.matrices {
        let (runner, default_flavor) = 'out: {
            if ignore_runs_on {
                break 'out (None, default_flavor);
            }

            let runs_on = steps.runs_on.to_redacted();

            let os = match runs_on.split_once('-').map(|(os, _)| os) {
                Some("ubuntu") => Os::Linux,
                Some("windows") => Os::Windows,
                Some("macos") => Os::Mac,
                _ => bail!("Unsupported runs-on directive: {}", steps.runs_on),
            };

            let runner = match RunnerKind::from_os(cx, &os) {
                Ok(runner) => runner,
                Err(error) => {
                    tracing::warn!("Failed to set up runner for {os}: {error}");
                    continue 'outer;
                }
            };

            (Some(runner), default_os_shell_flavor(&os))
        };

        let mut commands = Vec::new();

        for step in &steps.steps {
            if let Some((_, uses)) = &step.uses {
                let uses_redacted = uses.to_redacted();

                if !should_skip_use(uses_redacted.as_ref()) {
                    if let Some(runner) = runners.runners.get(uses_redacted.as_ref()) {
                        let env_file =
                            Rc::<Path>::from(runner.envs_dir.join(format!("env-{}", runner.id)));

                        match &runner.kind {
                            GithubActionKind::Node { main, post } => {
                                let mut env = BTreeMap::new();

                                let it = runner
                                    .defaults
                                    .iter()
                                    .map(|(k, v)| (k.clone(), RString::from(v.clone())));
                                let it = it.chain(step.with.clone());

                                for (key, value) in it {
                                    env.insert(format!("INPUT_{key}"), OsArg::from(value));
                                }

                                env.insert(
                                    String::from("GITHUB_ENV"),
                                    OsArg::from(env_file.clone()),
                                );

                                commands.push(
                                    RunCommand::command("node", [main])
                                        .with_name(Some(uses.clone()))
                                        .with_env(env.clone())
                                        .with_skipped(step.skipped.clone())
                                        .with_env_file(Some(env_file.clone())),
                                );

                                if let Some(post) = post {
                                    let args =
                                        vec![RString::from(post.to_string_lossy().into_owned())];

                                    commands.push(
                                        RunCommand::command("node", args)
                                            .with_name(Some(uses.clone()))
                                            .with_env(env.clone())
                                            .with_skipped(step.skipped.clone())
                                            .with_env_file(Some(env_file.clone())),
                                    );
                                }
                            }
                            GithubActionKind::Composite { steps } => {
                                let mut tree = Tree::new();
                                tree.insert_prefix(
                                    "inputs",
                                    runner
                                        .defaults
                                        .iter()
                                        .map(|(k, v)| (k.clone(), RString::from(v.clone()))),
                                );
                                tree.insert_prefix("inputs", step.with.clone());
                                let eval = Eval::new().with_tree(&tree);

                                for step in steps {
                                    let Some(run) = &step.run else {
                                        continue;
                                    };

                                    let script = eval.eval(run)?.into_owned();

                                    let mut env = BTreeMap::new();

                                    for (k, v) in &step.env {
                                        env.insert(
                                            k.clone(),
                                            OsArg::from(eval.eval(v)?.into_owned()),
                                        );
                                    }

                                    env.insert(
                                        String::from("GITHUB_ACTION_PATH"),
                                        OsArg::from(runner.action_path.clone()),
                                    );

                                    env.insert(
                                        String::from("GITHUB_ENV"),
                                        OsArg::from(env_file.clone()),
                                    );

                                    let flavor = match step.shell.as_deref() {
                                        Some("bash") => ShellFlavor::Sh,
                                        Some("powershell") => ShellFlavor::Powershell,
                                        Some(other) => bail!("Unsupported shell: {}", other),
                                        None => default_flavor,
                                    };

                                    commands.push(
                                        RunCommand::script(script, flavor)
                                            .with_env(env)
                                            .with_env_file(Some(env_file.clone())),
                                    );
                                }
                            }
                        }
                    }
                }
            }

            if let Some(rust_toolchain) = rust_toolchain(step)? {
                if rust_toolchain.components.is_some() || rust_toolchain.targets.is_some() {
                    let mut args = vec![
                        RString::from("toolchain"),
                        RString::from("install"),
                        RString::from(rust_toolchain.version),
                    ];

                    if let Some(c) = rust_toolchain.components {
                        args.push(RString::from("-c"));
                        args.push(RString::from(c));
                    }

                    if let Some(t) = rust_toolchain.targets {
                        args.push(RString::from("-t"));
                        args.push(RString::from(t));
                    }

                    args.extend([
                        RString::from("--profile"),
                        RString::from("minimal"),
                        RString::from("--no-self-update"),
                    ]);

                    commands.push(
                        RunCommand::command("rustup", args).with_skipped(step.skipped.clone()),
                    );
                }

                commands.push(
                    RunCommand::command("rustup", [RStr::new("default"), rust_toolchain.version])
                        .with_skipped(step.skipped.clone()),
                );
            }

            let shell = step.shell.as_deref().map(RStr::to_redacted);

            let shell = match shell.as_deref() {
                Some("bash") => ShellFlavor::Sh,
                Some("powershell") => ShellFlavor::Powershell,
                Some(other) => bail!("Unsupported shell: {}", other),
                None => default_flavor,
            };

            if let Some(script) = &step.run {
                let env = step
                    .env()
                    .iter()
                    .map(|(key, value)| (key.clone(), OsArg::from(value)))
                    .collect();

                commands.push(
                    RunCommand::script(script.clone(), shell)
                        .with_name(step.name.clone())
                        .with_env(env)
                        .with_skipped(step.skipped.clone())
                        .with_working_directory(step.working_directory.clone()),
                );
            }
        }

        batches.push(CommandBatch {
            commands,
            runner,
            matrix: if !matrix.is_empty() {
                Some(matrix.clone())
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
    workflow: &Workflow<'_>,
    jobs: &HashSet<RString>,
    ignore: &HashSet<String>,
    ignore_runs_on: bool,
    runners: &ActionRunners,
    default_flavor: ShellFlavor,
) -> Result<()> {
    for job in workflow.jobs(ignore)? {
        if !jobs.contains(&job.id) {
            continue;
        }

        job_to_batches(cx, batches, &job, ignore_runs_on, runners, default_flavor)
            .with_context(|| anyhow!("Job `{}`", job.name))?;
    }

    Ok(())
}

fn default_os_shell_flavor(os: &Os) -> ShellFlavor {
    match os {
        Os::Windows => ShellFlavor::Powershell,
        _ => ShellFlavor::Sh,
    }
}

struct RustToolchain<'a> {
    version: &'a RStr,
    components: Option<&'a RStr>,
    targets: Option<&'a RStr>,
}

/// Check if a use should be skipped.
fn should_skip_use(uses: &str) -> bool {
    let Some((head, _)) = uses.split_once('@') else {
        return true;
    };

    let Some((_, what)) = head.split_once('/') else {
        return true;
    };

    matches!(what, "checkout" | "rust-toolchain")
}

/// Extract a rust version from a `rust-toolchain` job.
fn rust_toolchain(step: &Step) -> Result<Option<RustToolchain<'_>>> {
    let Some((_, uses)) = &step.uses else {
        return Ok(None);
    };

    let Some((head, version)) = uses.split_once('@') else {
        return Ok(None);
    };

    let Some((_, what)) = head.split_once('/') else {
        return Ok(None);
    };

    if what != "rust-toolchain" {
        return Ok(None);
    }

    let version = step
        .with
        .get("toolchain")
        .map(RString::as_redact)
        .unwrap_or(version);

    let components = step.with.get("components").map(RString::as_redact);
    let targets = step.with.get("targets").map(RString::as_redact);

    Ok(Some(RustToolchain {
        version,
        components,
        targets,
    }))
}

struct CommandBatch {
    commands: Vec<RunCommand>,
    runner: Option<RunnerKind>,
    matrix: Option<Matrix>,
}

impl CommandBatch {
    fn runners(&self, opts: &[RunnerKind], same_os: bool) -> BTreeSet<RunnerKind> {
        let mut set = BTreeSet::new();
        set.extend(opts.iter().copied());
        set.extend(self.runner);

        if same_os {
            set.retain(|r| *r == RunnerKind::Same);
        }

        if set.is_empty() {
            set.insert(RunnerKind::Same);
        }

        set
    }
}

enum Run {
    Shell {
        script: RString,
        flavor: ShellFlavor,
    },
    Command {
        command: OsArg,
        args: Box<[OsArg]>,
    },
}

struct RunCommand {
    run: Run,
    name: Option<RString>,
    env: BTreeMap<String, OsArg>,
    skipped: Option<String>,
    working_directory: Option<RString>,
    env_file: Option<Rc<Path>>,
}

impl RunCommand {
    fn command<A>(command: impl Into<OsArg>, args: A) -> Self
    where
        A: IntoIterator<Item: Into<OsArg>>,
    {
        Self::with_run(Run::Command {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
        })
    }

    fn script(script: RString, flavor: ShellFlavor) -> Self {
        Self::with_run(Run::Shell { script, flavor })
    }

    // Get the shell associated with the run command, if any.
    fn shell(&self) -> Option<ShellFlavor> {
        match &self.run {
            Run::Shell { flavor, .. } => Some(*flavor),
            _ => None,
        }
    }

    fn with_run(run: Run) -> Self {
        Self {
            run,
            name: None,
            env: BTreeMap::new(),
            skipped: None,
            working_directory: None,
            env_file: None,
        }
    }

    /// Modify the name of the run command.
    #[inline]
    fn with_name(mut self, name: Option<RString>) -> Self {
        self.name = name;
        self
    }

    /// Modify the environment of the run command.
    #[inline]
    fn with_env(mut self, env: BTreeMap<String, OsArg>) -> Self {
        self.env = env;
        self
    }

    /// Modify the skipped status of the run command.
    #[inline]
    fn with_skipped(mut self, skipped: Option<String>) -> Self {
        self.skipped = skipped;
        self
    }

    /// Modify the working directory of the run command.
    #[inline]
    fn with_working_directory(mut self, working_directory: Option<RString>) -> Self {
        self.working_directory = working_directory;
        self
    }

    /// Modify the environment file of the run command.
    #[inline]
    fn with_env_file(mut self, env_file: Option<Rc<Path>>) -> Self {
        self.env_file = env_file;
        self
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

    fn build(
        &self,
        cx: &Ctxt,
        opts: &Opts,
        path: &Path,
        command: &RunCommand,
        current_env: &BTreeMap<String, String>,
    ) -> Result<Runner> {
        match *self {
            Self::Same => setup_same(cx, path, command),
            Self::Wsl => {
                let Some(wsl) = cx.system.wsl.first() else {
                    bail!("WSL not available");
                };

                Ok(setup_wsl(path, wsl, opts, command, current_env))
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

fn setup_same(cx: &Ctxt, path: &Path, run: &RunCommand) -> Result<Runner> {
    match &run.run {
        Run::Shell { script, flavor } => match flavor {
            ShellFlavor::Powershell => {
                let Some(powershell) = cx.system.powershell.first() else {
                    bail!("PowerShell not available");
                };

                let c = powershell.command(path, script);
                Ok(Runner::new(c))
            }
            ShellFlavor::Sh => {
                let mut c = Command::new("bash");
                c.args(["-i", "-c"]);
                c.arg(script);
                c.current_dir(path);
                Ok(Runner::new(c))
            }
        },
        Run::Command { command, args } => {
            let mut c = Command::new(command);
            c.args(args.as_ref());
            c.current_dir(path);
            Ok(Runner::new(c))
        }
    }
}

fn setup_wsl(
    path: &Path,
    wsl: &Wsl,
    opts: &Opts,
    run: &RunCommand,
    current_env: &BTreeMap<String, String>,
) -> Runner {
    let mut c = wsl.shell(path);

    match &run.run {
        Run::Shell { script, flavor } => match flavor {
            ShellFlavor::Powershell => {
                c.args(["powershell", "-Command"]);
                c.arg(script);
            }
            ShellFlavor::Sh => {
                c.args(["bash", "-i", "-c"]);
                c.arg(script);
            }
        },
        Run::Command { command, args } => {
            c.arg(command);
            c.args(args.as_ref());
        }
    }

    let mut seen = HashSet::new();

    let mut wslenv = String::new();

    for e in &opts.env {
        if !wslenv.is_empty() {
            wslenv.push(':');
        }

        if let Some((key, _)) = e.split_once('=') {
            if !seen.insert(key) {
                continue;
            }

            wslenv.push_str(key);
        } else {
            if !seen.insert(e) {
                continue;
            }

            wslenv.push_str(e);
        }
    }

    for e in run.env.keys().chain(current_env.keys()) {
        if !wslenv.is_empty() {
            wslenv.push(':');
        }

        if !seen.insert(e) {
            continue;
        }

        wslenv.push_str(e);
    }

    for key in current_env.keys() {
        if !wslenv.is_empty() {
            wslenv.push(':');
        }

        wslenv.push_str(key);
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

#[derive(Debug)]
struct ActionRunner {
    kind: GithubActionKind,
    action_path: Rc<Path>,
    defaults: BTreeMap<String, String>,
    envs_dir: Rc<Path>,
    id: String,
}

#[derive(Default, Debug)]
struct ActionRunners {
    runners: HashMap<String, ActionRunner>,
}

fn sync_runners(cx: &Ctxt<'_>, uses: &BTreeMap<String, BTreeSet<String>>) -> Result<ActionRunners> {
    let mut runners = ActionRunners::default();

    for (name, versions) in uses {
        let Some((repo, name)) = name.split_once('/') else {
            continue;
        };

        let project_dirs = cx
            .paths
            .project_dirs
            .context("Kick does not have project directories")?;

        let cache_dir = project_dirs.cache_dir();
        let repos_dir = cache_dir.join("repos");

        let repo_dir = repos_dir.join(repo).join(name);
        let git_dir = repo_dir.join("git");

        if !git_dir.is_dir() {
            fs::create_dir_all(&git_dir).with_context(|| {
                anyhow!("Failed to create repo directory: {}", git_dir.display())
            })?;
        }

        let r = match gix::open(&git_dir) {
            Ok(r) => r,
            Err(gix::open::Error::NotARepository { .. }) => gix::init_bare(&git_dir)?,
            Err(error) => {
                return Err(error).context("Failed to open or initialize cache repository")
            }
        };

        let url = format!("https://github.com/{repo}/{name}");

        let mut refspecs = Vec::new();
        let mut reverse = HashMap::new();

        for version in versions {
            for remote_name in [
                BString::from(format!("refs/tags/{version}")),
                BString::from(format!("refs/heads/{version}")),
            ] {
                refspecs.push(remote_name.clone());
                reverse.insert(remote_name, version);
            }
        }

        let mut out = Vec::new();

        match crate::gix::sync(&r, &url, &refspecs) {
            Ok(remotes) => {
                for (remote_name, id) in remotes {
                    let Some(version) = reverse.remove(&remote_name) else {
                        continue;
                    };

                    let work_dir = repo_dir.join(WORKDIR).join(version);

                    fs::create_dir_all(&work_dir).with_context(|| {
                        anyhow!("Failed to create work directory: {}", work_dir.display())
                    })?;

                    let id_path = work_dir.join(GIT_OBJECT_ID_FILE);

                    fs::write(&id_path, id.as_bytes()).with_context(|| {
                        anyhow!("Failed to write object ID: {}", id_path.display())
                    })?;

                    out.push((work_dir, id, repo, name, version));
                }
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to sync remote `{repo}/{name}` with remote `{url}`: {error}"
                );
            }
        }

        // Try to read out remaining versions from the workdir cache.
        for (_, version) in reverse {
            let work_dir = repo_dir.join(WORKDIR).join(version);
            let id_path = work_dir.join(GIT_OBJECT_ID_FILE);

            let id = match fs::read(&id_path) {
                Ok(id) => ObjectId::try_from(&id[..])
                    .with_context(|| anyhow!("{}: Failed to parse object ID", id_path.display()))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    continue;
                }
                Err(e) => {
                    return Err(e)
                        .context(anyhow!("{}: Failed to read object ID", id_path.display()))
                }
            };

            out.push((work_dir, id, repo, name, version));
        }

        for (work_dir, id, repo, name, version) in out {
            let key = format!("{repo}/{name}@{version}");

            // Load an action runner directly out of a repository without checking it out.
            let Some(runner) = crate::github_action::load(&r, id, &work_dir, version)? else {
                tracing::warn!("Could not load runner for {key}");
                continue;
            };

            let envs_dir = cache_dir.join(ENVS);

            fs::create_dir_all(&envs_dir).with_context(|| {
                anyhow!("Failed to create envs directory: {}", envs_dir.display())
            })?;

            runners.runners.insert(
                key,
                ActionRunner {
                    kind: runner.kind,
                    action_path: work_dir.into(),
                    defaults: runner.defaults,
                    envs_dir: envs_dir.into(),
                    id: id.to_string(),
                },
            );
        }
    }

    Ok(runners)
}
