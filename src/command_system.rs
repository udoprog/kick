use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::rc::Rc;
use std::str;

use anyhow::{anyhow, bail, ensure, Context, Result};
use bstr::BString;
use gix::ObjectId;
use relative_path::RelativePath;
use termcolor::{Color, ColorSpec, WriteColor};

use crate::config::Os;
use crate::ctxt::Ctxt;
use crate::github_action::GithubActionKind;
use crate::model::Repo;
use crate::process::{Command, OsArg};
use crate::rstr::{RStr, RString};
use crate::shell::Shell;
use crate::system::Wsl;
use crate::workflows::{Eval, Job, Matrix, Step, Tree, Workflow, Workflows};

const GITHUB_BASE: &str = "https://github.com";
const GIT_OBJECT_ID_FILE: &str = ".git-object-id";
const WORKDIR: &str = "workdir";
const ENVS: &str = "envs";

/// A system of commands to be run.
pub struct CommandSystem<'a, 'cx> {
    cx: &'a Ctxt<'cx>,
    colors: &'a Colors,
    verbose: bool,
    dry_run: bool,
    env: BTreeMap<String, String>,
    env_passthrough: BTreeSet<String>,
    workflows: HashSet<String>,
    jobs: HashSet<String>,
    argument_runners: Vec<RunnerKind>,
    batches: Vec<CommandBatch>,
    matrix_ignore: HashSet<String>,
}

impl<'a, 'cx> CommandSystem<'a, 'cx> {
    /// Create a new command system.
    pub(crate) fn new(cx: &'a Ctxt<'cx>, colors: &'a Colors) -> Self {
        Self {
            cx,
            colors,
            verbose: false,
            dry_run: false,
            env: BTreeMap::new(),
            env_passthrough: BTreeSet::new(),
            workflows: HashSet::new(),
            jobs: HashSet::new(),
            argument_runners: Vec::new(),
            batches: Vec::new(),
            matrix_ignore: HashSet::new(),
        }
    }

    /// Insert a matrix variable to ignore.
    pub(crate) fn ignore_matrix_variable<S>(&mut self, variable: S)
    where
        S: AsRef<str>,
    {
        self.matrix_ignore.insert(variable.as_ref().to_owned());
    }

    /// Set the command system to be verbose.
    pub(crate) fn set_verbose(&mut self) {
        self.verbose = true;
    }

    /// Set the command system to be a dry run.
    pub(crate) fn set_dry_run(&mut self) {
        self.dry_run = true;
    }

    /// Parse an environment.
    pub(crate) fn parse_env(&mut self, env: &str) -> Result<()> {
        if let Some((key, value)) = env.split_once('=') {
            self.env.insert(key.to_owned(), value.to_owned());
        } else {
            self.env_passthrough.insert(env.to_owned());
        }

        Ok(())
    }

    /// Add a workflow to be loaded.
    pub(crate) fn add_workflow<S>(&mut self, workflow: S)
    where
        S: AsRef<str>,
    {
        self.workflows.insert(workflow.as_ref().to_owned());
    }

    /// Add a job to be loaded.
    pub(crate) fn add_job<S>(&mut self, job: S)
    where
        S: AsRef<str>,
    {
        self.jobs.insert(job.as_ref().to_owned());
    }

    /// Add an operating system.
    pub(crate) fn add_os(&mut self, os: &Os) -> Result<()> {
        self.argument_runners
            .push(RunnerKind::from_os(self.cx, os)?);
        Ok(())
    }

    /// Add a command to run.
    pub(crate) fn add_command<C, A>(&mut self, command: C, args: A)
    where
        C: Into<OsArg>,
        A: IntoIterator<Item: Into<OsArg>>,
    {
        self.batches.push(CommandBatch {
            commands: vec![Run::command(command, args)],
            runner: None,
            matrix: None,
        });
    }

    /// Load workflows.
    pub(crate) fn load_workflows(
        &mut self,
        repo: &Repo,
        ignore_runs_on: bool,
    ) -> Result<Vec<(Workflow<'a, 'cx>, Vec<Job>)>> {
        let mut uses = BTreeMap::<_, BTreeSet<_>>::new();
        let mut workflows = Vec::new();

        let wfs = Workflows::new(self.cx, repo)?;

        for workflow in wfs.workflows() {
            let workflow = workflow?;

            if !self.workflows.is_empty() && !self.workflows.contains(workflow.id()) {
                continue;
            }

            let mut jobs = Vec::new();

            for job in workflow.jobs(&self.matrix_ignore)? {
                let id = job.id.to_exposed();

                if !self.jobs.is_empty() && !self.jobs.contains(id.as_ref()) {
                    continue;
                }

                for (_, steps) in &job.matrices {
                    for step in &steps.steps {
                        if let Some((_, name)) = &step.uses {
                            let name = name.to_exposed();

                            let u = parse_uses(name.as_ref()).with_context(|| {
                                anyhow!(
                                    "Uses statement in job `{}` and step `{}`",
                                    job.name,
                                    step.name()
                                )
                            })?;

                            match u {
                                Use::Github(repo, name, version) => {
                                    uses.entry((repo.to_owned(), name.to_owned()))
                                        .or_default()
                                        .insert(version.to_owned());
                                }
                            }
                        }
                    }
                }

                jobs.push(job);
            }

            workflows.push((workflow, jobs));
        }

        let runners = sync_github_uses(self.cx, &uses)?;

        for (workflow, jobs) in &workflows {
            for job in jobs {
                self.job_to_batches(job, ignore_runs_on, &runners)
                    .with_context(|| anyhow!("Workflow `{}` job `{}`", workflow.id(), job.name))?;
            }
        }

        Ok(workflows)
    }

    pub(crate) fn commit<O>(
        self,
        o: &mut O,
        repo_path: &Path,
        same_os: bool,
        default_shell: Shell,
    ) -> Result<()>
    where
        O: ?Sized + WriteColor,
    {
        for batch in &self.batches {
            for runner in batch.runners(&self.argument_runners, same_os) {
                write!(o, "# In ")?;

                o.set_color(&self.colors.title)?;
                write!(o, "{}", repo_path.display())?;
                o.reset()?;

                if let Some(name) = runner.name() {
                    write!(o, " using ")?;

                    o.set_color(&self.colors.title)?;
                    write!(o, "{name}")?;
                    o.reset()?;
                }

                if let Some(matrix) = &batch.matrix {
                    write!(o, " ")?;

                    o.set_color(&self.colors.matrix)?;
                    write!(o, "{}", matrix.display())?;
                    o.reset()?;
                }

                writeln!(o)?;

                let mut current_env = BTreeMap::new();

                for (index, run) in batch.commands.iter().enumerate() {
                    let modified;

                    let path = match &run.working_directory {
                        Some(working_directory) => {
                            let working_directory = working_directory.to_exposed();
                            let working_directory = RelativePath::new(working_directory.as_ref());
                            modified = working_directory.to_logical_path(repo_path);
                            &modified
                        }
                        None => repo_path,
                    };

                    let mut runner = runner.build(self.cx, &self, path, run, &current_env)?;

                    for (key, value) in &self.env {
                        runner.command.env(key, value);
                    }

                    for (key, value) in &run.env {
                        runner.command.env(key, value);
                    }

                    if let Some((key, value)) = runner.extra_env {
                        if !value.is_empty() {
                            runner.command.env(key, value);
                        }
                    }

                    write!(o, "# ")?;

                    if let Some(name) = &run.name {
                        o.set_color(&self.colors.title)?;
                        write!(o, "{name}")?;
                        o.reset()?;
                    } else {
                        o.set_color(&self.colors.title)?;
                        write!(o, "Step {} / {}", index + 1, batch.commands.len())?;
                        o.reset()?;
                        write!(o, "")?;
                    }

                    if let Some(skipped) = &run.skipped {
                        write!(o, " ")?;
                        o.set_color(&self.colors.skip_cond)?;
                        write!(o, "(skipped: {skipped})")?;
                        o.reset()?;
                    }

                    if !self.verbose && !runner.command.env.is_empty() {
                        let plural = if runner.command.env.len() == 1 {
                            "variable"
                        } else {
                            "variables"
                        };

                        write!(o, " ")?;

                        o.set_color(&self.colors.warn)?;
                        write!(
                            o,
                            "(see {} env {plural} with `--verbose`)",
                            runner.command.env.len()
                        )?;
                        o.reset()?;
                    }

                    writeln!(o)?;

                    match &default_shell {
                        Shell::Bash => {
                            if self.verbose {
                                for (key, value) in &runner.command.env {
                                    let key = key.to_string_lossy();
                                    let value = value.to_string_lossy();
                                    let value = default_shell.escape(value.as_ref());
                                    write!(o, "{key}={value} ")?;
                                }
                            }

                            write!(o, "{}", runner.command.display_with(default_shell))?;
                        }
                        Shell::Powershell => {
                            if self.verbose && !runner.command.env.is_empty() {
                                writeln!(o, "powershell -Command {{")?;

                                for (key, value) in &runner.command.env {
                                    let key = key.to_string_lossy();
                                    let value = value.to_string_lossy();
                                    let value = default_shell.escape(value.as_ref());
                                    writeln!(o, "  $Env:{key}={value};")?;
                                }

                                writeln!(o, "  {}", runner.command.display_with(default_shell))?;
                                write!(o, "}}")?;
                            } else {
                                write!(o, "{}", runner.command.display_with(default_shell))?;
                            }
                        }
                    }

                    writeln!(o)?;

                    if run.skipped.is_none() && !self.dry_run {
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

    fn job_to_batches(
        &mut self,
        job: &Job,
        ignore_runs_on: bool,
        runners: &ActionRunners,
    ) -> Result<()> {
        'outer: for (matrix, steps) in &job.matrices {
            let runner = {
                let runs_on = steps.runs_on.to_exposed();

                let os = match runs_on.split_once('-').map(|(os, _)| os) {
                    Some("ubuntu") => Os::Linux,
                    Some("windows") => Os::Windows,
                    Some("macos") => Os::Mac,
                    _ => bail!("Unsupported runs-on directive: {}", steps.runs_on),
                };

                let runner = match RunnerKind::from_os(self.cx, &os) {
                    Ok(runner) => runner,
                    Err(error) => {
                        tracing::warn!("Failed to set up runner: {error}");
                        continue 'outer;
                    }
                };

                Some(runner)
            };

            let runner = if ignore_runs_on { None } else { runner };

            let mut commands = Vec::new();

            for step in &steps.steps {
                if let Some((_, uses)) = &step.uses {
                    let uses_redacted = uses.to_exposed();

                    if !should_skip_use(uses_redacted.as_ref()) {
                        if let Some(runner) = runners.runners.get(uses_redacted.as_ref()) {
                            let env_file = Rc::<Path>::from(
                                runner.envs_dir.join(format!("env-{}", runner.id)),
                            );

                            match &runner.kind {
                                GithubActionKind::Node {
                                    main,
                                    post,
                                    node_version,
                                } => {
                                    let Some(node) = self
                                        .cx
                                        .system
                                        .node
                                        .iter()
                                        .find(|n| n.version.major >= *node_version)
                                    else {
                                        let alternatives = self
                                            .cx
                                            .system
                                            .node
                                            .iter()
                                            .map(|n| n.version.to_string())
                                            .collect::<Vec<_>>()
                                            .join(", ");
                                        bail!("Could not find node {node_version} on the system, alternatives: {alternatives}");
                                    };

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
                                        Run::command(&node.path, [main])
                                            .with_name(Some(uses.clone()))
                                            .with_env(env.clone())
                                            .with_skipped(step.skipped.clone())
                                            .with_env_file(Some(env_file.clone())),
                                    );

                                    if let Some(post) = post {
                                        let args = vec![RString::from(
                                            post.to_string_lossy().into_owned(),
                                        )];

                                        commands.push(
                                            Run::command(&node.path, args)
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

                                        let shell = to_shell(step.shell.as_deref())?;

                                        commands.push(
                                            Run::script(script, shell)
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

                        commands
                            .push(Run::command("rustup", args).with_skipped(step.skipped.clone()));
                    }

                    commands.push(
                        Run::command("rustup", [RStr::new("default"), rust_toolchain.version])
                            .with_skipped(step.skipped.clone()),
                    );
                }

                let shell = to_shell(step.shell.as_deref().map(RStr::to_exposed).as_deref())?;

                if let Some(script) = &step.run {
                    let env = step
                        .env()
                        .iter()
                        .map(|(key, value)| (key.clone(), OsArg::from(value)))
                        .collect();

                    commands.push(
                        Run::script(script, shell)
                            .with_name(step.name.clone())
                            .with_env(env)
                            .with_skipped(step.skipped.clone())
                            .with_working_directory(step.working_directory.clone()),
                    );
                }
            }

            self.batches.push(CommandBatch {
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
}

struct CommandBatch {
    commands: Vec<Run>,
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

enum RunKind {
    Shell { script: Box<RStr>, shell: Shell },
    Command { command: OsArg, args: Box<[OsArg]> },
}

struct Run {
    run: RunKind,
    name: Option<RString>,
    env: BTreeMap<String, OsArg>,
    skipped: Option<String>,
    working_directory: Option<RString>,
    // If an environment file is supported, this is the path to the file to set up.
    env_file: Option<Rc<Path>>,
}

impl Run {
    /// Setup a command to run.
    fn command<C, A>(command: C, args: A) -> Self
    where
        C: Into<OsArg>,
        A: IntoIterator<Item: Into<OsArg>>,
    {
        Self::with_run(RunKind::Command {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
        })
    }

    /// Setup a script to run.
    fn script(script: impl Into<Box<RStr>>, shell: Shell) -> Self {
        Self::with_run(RunKind::Shell {
            script: script.into(),
            shell,
        })
    }

    fn with_run(run: RunKind) -> Self {
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
        cx: &Ctxt<'_>,
        system: &CommandSystem,
        path: &Path,
        command: &Run,
        current_env: &BTreeMap<String, String>,
    ) -> Result<Runner> {
        match *self {
            Self::Same => setup_same(cx, path, command),
            Self::Wsl => {
                let Some(wsl) = cx.system.wsl.first() else {
                    bail!("WSL not available");
                };

                Ok(setup_wsl(system, path, wsl, command, current_env))
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

fn parse_env(contents: &[u8]) -> Result<Vec<(String, String)>> {
    use bstr::ByteSlice;

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
        .map(RString::as_rstr)
        .unwrap_or(version);

    let components = step.with.get("components").map(RString::as_rstr);
    let targets = step.with.get("targets").map(RString::as_rstr);

    Ok(Some(RustToolchain {
        version,
        components,
        targets,
    }))
}

fn setup_same(cx: &Ctxt<'_>, path: &Path, run: &Run) -> Result<Runner> {
    match &run.run {
        RunKind::Shell { script, shell } => match shell {
            Shell::Powershell => {
                let Some(powershell) = cx.system.powershell.first() else {
                    bail!("PowerShell not available");
                };

                let mut c = powershell.command(path);
                c.arg("-Command");
                c.arg(script);
                Ok(Runner::new(c))
            }
            Shell::Bash => {
                let Some(bash) = cx.system.bash.first() else {
                    bail!("Bash is not available");
                };

                let mut c = bash.command(path);
                c.args(["-i", "-c"]);
                c.arg(script);
                Ok(Runner::new(c))
            }
        },
        RunKind::Command { command, args } => {
            let mut c = Command::new(command);
            c.args(args.as_ref());
            c.current_dir(path);
            Ok(Runner::new(c))
        }
    }
}

fn setup_wsl(
    system: &CommandSystem,
    path: &Path,
    wsl: &Wsl,
    run: &Run,
    current_env: &BTreeMap<String, String>,
) -> Runner {
    let mut c = wsl.shell(path);

    match &run.run {
        RunKind::Shell { script, shell } => match shell {
            Shell::Powershell => {
                c.args(["powershell", "-Command"]);
                c.arg(script);
            }
            Shell::Bash => {
                c.args(["bash", "-i", "-c"]);
                c.arg(script);
            }
        },
        RunKind::Command { command, args } => {
            c.arg(command);
            c.args(args.as_ref());
        }
    }

    let mut seen = HashSet::new();

    let mut wslenv = String::new();

    for e in system
        .env_passthrough
        .iter()
        .chain(system.env.keys())
        .chain(run.env.keys())
        .chain(current_env.keys())
    {
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

/// System colors.
pub(crate) struct Colors {
    skip_cond: ColorSpec,
    title: ColorSpec,
    matrix: ColorSpec,
    warn: ColorSpec,
}

impl Colors {
    /// Construct colors system.
    pub(crate) fn new() -> Self {
        let mut skip_cond = ColorSpec::new();
        skip_cond.set_fg(Some(Color::Red));
        skip_cond.set_bold(true);

        let mut title = ColorSpec::new();
        title.set_fg(Some(Color::White));
        title.set_bold(true);

        let mut matrix = ColorSpec::new();
        matrix.set_fg(Some(Color::Yellow));

        let mut warn = ColorSpec::new();
        warn.set_fg(Some(Color::Yellow));

        Self {
            skip_cond,
            title,
            matrix,
            warn,
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

fn sync_github_uses(
    cx: &Ctxt<'_>,
    github_uses: &BTreeMap<(String, String), BTreeSet<String>>,
) -> Result<ActionRunners> {
    let mut runners = ActionRunners::default();

    for ((repo, name), versions) in github_uses {
        let project_dirs = cx
            .paths
            .project_dirs
            .context("Kick does not have project directories")?;

        let cache_dir = project_dirs.cache_dir();
        let actions_dir = cache_dir.join("actions");

        let repo_dir = actions_dir.join(repo).join(name);
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

        let url = format!("{GITHUB_BASE}/{repo}/{name}");

        let mut out = Vec::new();

        tracing::debug!("Syncing {} from {url}", git_dir.display());

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

fn to_shell(shell: Option<&str>) -> Result<Shell> {
    let Some(shell) = shell else {
        return Ok(Shell::Bash);
    };

    match shell {
        "bash" => Ok(Shell::Bash),
        "powershell" => Ok(Shell::Powershell),
        other => bail!("Unsupported shell: {}", other),
    }
}

enum Use {
    Github(String, String, String),
}

fn parse_uses(uses: &str) -> Result<Use> {
    let (head, version) = uses.split_once('@').context("No version in uses")?;
    let (repo, name) = head.split_once('/').context("Expected <repo>/<name>")?;

    Ok(Use::Github(
        repo.to_owned(),
        name.to_owned(),
        version.to_owned(),
    ))
}
