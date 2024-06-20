use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str;

use anyhow::{anyhow, bail, ensure, Context, Result};
use bstr::BString;
use clap::{Parser, ValueEnum};
use gix::ObjectId;
use relative_path::RelativePath;
use termcolor::{Color, ColorSpec, WriteColor};

use crate::action::{ActionKind, ActionStep};
use crate::config::Os;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::process::{Command, OsArg};
use crate::rstr::{RStr, RString};
use crate::shell::Shell;
use crate::system::Wsl;
use crate::workflows::{Eval, Job, Matrix, Step, Steps, Tree, WorkflowManifest, WorkflowManifests};

const GITHUB_BASE: &str = "https://github.com";
const GIT_OBJECT_ID_FILE: &str = ".git-object-id";
const WORKDIR: &str = "workdir";
const STATE: &str = "state";

const WINDOWS_BASH_MESSAGE: &str = r#"Bash is not installed by default on Windows!

To install it, consider:
* Run: winget install msys2.msys2
* Install manually from https://www.msys2.org/

If you install it in a non-standard location (other than C:\\msys64),
make sure that its usr/bin directory is in the system PATH."#;

/// A system of commands to be run.
pub struct CommandSystem<'a, 'cx> {
    cx: &'a Ctxt<'cx>,
    matrix_ignore: HashSet<String>,
}

impl<'a, 'cx> CommandSystem<'a, 'cx> {
    /// Create a new command system.
    pub(crate) fn new(cx: &'a Ctxt<'cx>) -> Self {
        Self {
            cx,
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

    /// Load workflows from a repository.
    pub(crate) fn load_repo_workflows(&self, repo: &Repo) -> Result<Workflows<'a, 'cx>> {
        let mut actions = Actions::default();

        let mut workflows = Vec::new();
        let wfs = WorkflowManifests::new(self.cx, repo)?;

        for workflow in wfs.workflows() {
            let workflow = workflow?;

            let mut jobs = Vec::new();

            for job in workflow.jobs(&self.matrix_ignore)? {
                for (_, steps) in &job.matrices {
                    for step in &steps.steps {
                        if let Some(name) = &step.uses {
                            actions.add_action(name).with_context(|| {
                                anyhow!(
                                    "Uses statement in job `{}` and step `{}`",
                                    job.id,
                                    step.name()
                                )
                            })?;
                        }
                    }
                }

                jobs.push(job);
            }

            workflows.push((workflow, jobs));
        }

        Ok(Workflows {
            workflows,
            actions,
            runners: Rc::new(ActionRunners::default()),
        })
    }
}

/// Loaded uses.
#[derive(Default)]
pub(crate) struct Actions {
    actions: BTreeMap<(String, String), BTreeSet<String>>,
}

impl Actions {
    /// Add an action by id.
    pub(crate) fn add_action<S>(&mut self, id: S) -> Result<()>
    where
        S: AsRef<RStr>,
    {
        let id = id.as_ref();
        let u =
            parse_action(id.to_exposed().as_ref()).with_context(|| anyhow!("Bad action: {id}"))?;

        match u {
            Use::Github(repo, name, version) => {
                self.actions
                    .entry((repo.to_owned(), name.to_owned()))
                    .or_default()
                    .insert(version.to_owned());
            }
        }

        Ok(())
    }

    /// Synchronize github uses.
    pub(crate) fn synchronize(&mut self, cx: &Ctxt<'_>) -> Result<ActionRunners> {
        let mut runners = ActionRunners::default();

        for ((repo, name), versions) in &self.actions {
            sync_github_use(&mut runners.runners, cx, repo, name, versions)
                .with_context(|| anyhow!("Failed to sync GitHub use {repo}/{name}@{versions:?}"))?;
        }

        Ok(runners)
    }
}

/// Loaded workflows.
pub(crate) struct Workflows<'a, 'cx> {
    workflows: Vec<(WorkflowManifest<'a, 'cx>, Vec<Job>)>,
    actions: Actions,
    runners: Rc<ActionRunners>,
}

impl<'a, 'cx> Workflows<'a, 'cx> {
    /// Iterate over workflows.
    pub(crate) fn iter<'this>(
        &'this self,
    ) -> impl Iterator<Item = &'this (WorkflowManifest<'a, 'cx>, Vec<Job>)> + 'this {
        self.workflows.iter()
    }

    /// Synchronize github uses.
    pub(crate) fn synchronize(&mut self, cx: &Ctxt<'cx>) -> Result<()> {
        self.runners = Rc::new(self.actions.synchronize(cx)?);
        Ok(())
    }

    /// Add jobs from a workflows, matrix, and associated steps.
    pub(crate) fn build_batch_from_step(
        &self,
        cx: &Ctxt<'_>,
        matrix: &Matrix,
        steps: &Steps,
        same_os: bool,
    ) -> Result<Batch> {
        let runs_on = steps.runs_on.to_exposed();

        let os = match runs_on.split_once('-').map(|(os, _)| os) {
            Some("ubuntu") => Os::Linux,
            Some("windows") => Os::Windows,
            Some("macos") => Os::Mac,
            _ => bail!("Unsupported runs-on directive: {}", steps.runs_on),
        };

        let run_on = if same_os {
            RunOn::Same
        } else {
            os_to_run_on(cx, &os)?
        };

        let mut commands = Vec::new();

        for step in &steps.steps {
            commands.push(Schedule::Step(ScheduleStep {
                step: step.clone(),
                runners: self.runners.clone(),
                tree: step.tree.clone(),
            }));
        }

        Ok(Batch {
            commands,
            run_on,
            matrix: if !matrix.is_empty() {
                Some(matrix.clone())
            } else {
                None
            },
        })
    }
}

#[derive(Default, Debug, Parser)]
pub(crate) struct BatchOptions {
    /// Run the command using the specified execution methods.
    #[arg(long, value_name = "run-on")]
    run_on: Vec<RunOn>,
    /// Environment variables to pass to the command to run. Only specifying
    /// `<key>` means that the specified environment variable should be passed
    /// through.
    ///
    /// For WSL, this constructs the WSLENV environment variable, which dictates
    /// what environments are passed in.
    #[arg(long, short = 'E', value_name = "key[=value]")]
    env: Vec<String>,
    /// Print verbose information about the command being run.
    #[arg(long)]
    verbose: bool,
    /// When printing diagnostics output, exposed secrets.
    ///
    /// If this is not specified, secrets will be printed as `***`.
    #[arg(long)]
    exposed: bool,
    /// Don't actually run any commands, just print what would be done.
    #[arg(long)]
    dry_run: bool,
}

/// A batch runner configuration.
pub(crate) struct BatchConfig<'a, 'cx> {
    cx: &'a Ctxt<'cx>,
    repo_path: &'a Path,
    shell: Shell,
    colors: Colors,
    env: BTreeMap<String, String>,
    env_passthrough: BTreeSet<String>,
    run_on: Vec<RunOn>,
    verbose: bool,
    dry_run: bool,
    exposed: bool,
}

impl<'a, 'cx> BatchConfig<'a, 'cx> {
    /// Construct a new batch configuration.
    pub(crate) fn new(cx: &'a Ctxt<'cx>, repo_path: &'a Path, shell: Shell) -> Self {
        Self {
            cx,
            repo_path,
            shell,
            colors: Colors::new(),
            env: BTreeMap::new(),
            env_passthrough: BTreeSet::new(),
            run_on: Vec::new(),
            verbose: false,
            dry_run: false,
            exposed: false,
        }
    }

    /// Add options from [`BatchOptions`].
    pub(crate) fn add_opts(&mut self, opts: &BatchOptions) -> Result<()> {
        for &run_on in &opts.run_on {
            self.add_run_on(run_on)?;
        }

        if opts.exposed {
            self.exposed = true;
        }

        if opts.verbose {
            self.verbose = true;
        }

        if opts.dry_run {
            self.dry_run = true;
        }

        for env in &opts.env {
            self.parse_env(env)?;
        }

        Ok(())
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

    /// Add an operating system.
    pub(crate) fn add_os(&mut self, os: &Os) -> Result<()> {
        self.run_on.push(os_to_run_on(self.cx, os)?);
        Ok(())
    }

    /// Add a run on.
    pub(crate) fn add_run_on(&mut self, run_on: RunOn) -> Result<()> {
        if let RunOn::Wsl = run_on {
            if self.cx.system.wsl.is_empty() {
                bail!("WSL is not available");
            }
        }

        self.run_on.push(run_on);
        Ok(())
    }
}

/// A constructed workflow batch.
pub(crate) struct Batch {
    commands: Vec<Schedule>,
    run_on: RunOn,
    matrix: Option<Matrix>,
}

impl Batch {
    /// Construct a batch with multiple commands.
    pub(crate) fn with_schedule(commands: Vec<Schedule>) -> Self {
        Self {
            commands,
            run_on: RunOn::Same,
            matrix: None,
        }
    }

    /// Construct a batch with a single command.
    pub(crate) fn command<C, A>(command: C, args: A) -> Self
    where
        C: Into<OsArg>,
        A: IntoIterator<Item: Into<OsArg>>,
    {
        Batch {
            commands: vec![Schedule::BasicCommand(ScheduleBasicCommand {
                command: command.into(),
                args: args.into_iter().map(Into::into).collect(),
            })],
            run_on: RunOn::Same,
            matrix: None,
        }
    }

    /// Commit a batch.
    pub(crate) fn commit<O>(self, o: &mut O, c: &BatchConfig<'_, '_>) -> Result<()>
    where
        O: ?Sized + WriteColor,
    {
        let mut scheduler = Scheduler::new();

        for run_on in self.runners(&c.run_on) {
            scheduler.push();

            write!(o, "# In ")?;

            o.set_color(&c.colors.title)?;
            write!(o, "{}", c.repo_path.display())?;
            o.reset()?;

            if let Some(name) = run_on.name() {
                write!(o, " using ")?;

                o.set_color(&c.colors.title)?;
                write!(o, "{name}")?;
                o.reset()?;
            }

            if let Some(matrix) = &self.matrix {
                write!(o, " ")?;

                o.set_color(&c.colors.matrix)?;
                write!(o, "{}", matrix.display())?;
                o.reset()?;
            }

            writeln!(o)?;

            for run in self.commands.iter() {
                scheduler.main.push_back(run.clone());
            }

            let mut step = 0usize;

            while let Some(run) = scheduler.advance(c)? {
                let modified;

                let path = match &run.working_directory {
                    Some(working_directory) => {
                        let working_directory = working_directory.to_exposed();
                        let working_directory = RelativePath::new(working_directory.as_ref());
                        modified = working_directory.to_logical_path(c.repo_path);
                        &modified
                    }
                    None => c.repo_path,
                };

                let mut runner = run_on.build_runner(c, path, &run, scheduler.env())?;

                for (key, value) in &c.env {
                    runner.command.env(key, value);
                }

                for (key, value) in scheduler.env() {
                    runner.command.env(key, value);
                }

                for (key, value) in &run.env {
                    runner.command.env(key, value);
                }

                for (key, value) in &runner.extra_env {
                    runner.command.env(key, value);
                }

                if !runner.paths.is_empty() {
                    let current_path = env::var_os("PATH").unwrap_or_default();
                    let paths = env::split_paths(&current_path);
                    let paths = env::join_paths(runner.paths.iter().cloned().chain(paths))?;
                    runner.command.env("PATH", paths);
                }

                step += 1;

                write!(o, "# ")?;

                o.set_color(&c.colors.title)?;

                if let Some(name) = &run.name {
                    write!(o, "{name}")?;
                } else {
                    write!(o, "step {step}")?;
                }

                o.reset()?;

                if let Some(skipped) = &run.skipped {
                    write!(o, " ")?;
                    o.set_color(&c.colors.skip_cond)?;
                    write!(o, "(skipped: {skipped})")?;
                    o.reset()?;
                }

                if !c.verbose && !runner.command.env.is_empty() {
                    let plural = if runner.command.env.len() == 1 {
                        "variable"
                    } else {
                        "variables"
                    };

                    write!(o, " ")?;

                    o.set_color(&c.colors.warn)?;
                    write!(
                        o,
                        "(see {} env {plural} with `--verbose`)",
                        runner.command.env.len()
                    )?;
                    o.reset()?;
                }

                writeln!(o)?;

                match &c.shell {
                    Shell::Bash => {
                        if c.verbose {
                            for (key, value) in &runner.command.env {
                                let key = key.to_string_lossy();

                                let value = if c.exposed {
                                    value.to_exposed_lossy()
                                } else {
                                    value.to_string_lossy()
                                };

                                let value = c.shell.escape(value.as_ref());
                                write!(o, "{key}={value} ")?;
                            }
                        }

                        write!(
                            o,
                            "{}",
                            runner.command.display_with(c.shell).with_exposed(c.exposed)
                        )?;
                    }
                    Shell::Powershell => {
                        if c.verbose && !runner.command.env.is_empty() {
                            writeln!(o, "powershell -Command {{")?;

                            for (key, value) in &runner.command.env {
                                let key = key.to_string_lossy();

                                let value = if c.exposed {
                                    value.to_exposed_lossy()
                                } else {
                                    value.to_string_lossy()
                                };

                                let value = c.shell.escape_string(value.as_ref());
                                writeln!(o, r#"  $Env:{key}={value};"#)?;
                            }

                            writeln!(
                                o,
                                "  {}",
                                runner.command.display_with(c.shell).with_exposed(c.exposed)
                            )?;
                            write!(o, "}}")?;
                        } else {
                            write!(
                                o,
                                "{}",
                                runner.command.display_with(c.shell).with_exposed(c.exposed)
                            )?;
                        }
                    }
                }

                writeln!(o)?;

                if run.skipped.is_none() && !c.dry_run {
                    truncate(
                        run.env_file
                            .as_slice()
                            .iter()
                            .chain(run.output_file.as_slice()),
                    )?;

                    let status = runner.command.status()?;
                    ensure!(status.success(), status);

                    if let Some(env_file) = &run.env_file {
                        if let Ok(contents) = fs::read(env_file) {
                            let env = scheduler.env_mut();

                            for (key, value) in parse_output(&contents)? {
                                env.insert(key, value);
                            }
                        }
                    }

                    if let (Some(output_file), Some(id), Some(tree)) =
                        (&run.output_file, &run.id, scheduler.tree_mut())
                    {
                        let id = id.to_exposed();

                        if let Ok(contents) = fs::read(output_file) {
                            let values = parse_output(&contents)?;
                            tree.insert_prefix(["steps", id.as_ref(), "outputs"], values);
                        }
                    }
                }
            }

            scheduler.pop();
        }

        Ok(())
    }

    fn runners(&self, opts: &[RunOn]) -> BTreeSet<RunOn> {
        let mut set = BTreeSet::new();
        set.extend(opts.iter().copied());
        set.insert(self.run_on);
        set
    }
}

/// Truncate the given collection of files and ensure they exist.
fn truncate<I>(paths: I) -> Result<()>
where
    I: IntoIterator<Item: AsRef<Path>>,
{
    for path in paths {
        let path = path.as_ref();

        let file = File::create(path).with_context(|| {
            anyhow!(
                "Failed to create temporary environment file: {}",
                path.display()
            )
        })?;

        file.set_len(0).with_context(|| {
            anyhow!(
                "Failed to truncate temporary environment file: {}",
                path.display()
            )
        })?;
    }

    Ok(())
}

enum RunKind {
    Shell {
        script: Box<RStr>,
        shell: Shell,
    },
    Command {
        command: OsArg,
        args: Box<[OsArg]>,
    },
    Node {
        node_version: u32,
        script_file: Rc<Path>,
    },
}

pub(crate) struct RunGroup {
    run: Option<Run>,
    main: Vec<Schedule>,
    post: Vec<Schedule>,
}

struct Scheduler {
    stack: Vec<Tree>,
    env: BTreeMap<String, String>,
    main: VecDeque<Schedule>,
    post: VecDeque<Schedule>,
}

impl Scheduler {
    fn new() -> Self {
        Self {
            stack: vec![],
            env: BTreeMap::new(),
            main: VecDeque::new(),
            post: VecDeque::new(),
        }
    }

    fn push(&mut self) {
        self.stack.push(Tree::new());
    }

    fn pop(&mut self) {
        self.stack.pop();
    }

    fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    fn tree(&self) -> Option<&Tree> {
        self.stack.last()
    }

    fn env_mut(&mut self) -> &mut BTreeMap<String, String> {
        &mut self.env
    }

    fn tree_mut(&mut self) -> Option<&mut Tree> {
        self.stack.last_mut()
    }

    fn advance(&mut self, c: &BatchConfig<'_, '_>) -> Result<Option<Run>> {
        loop {
            let command = 'next: {
                if let Some(item) = self.main.pop_front() {
                    break 'next item;
                }

                if let Some(item) = self.post.pop_front() {
                    break 'next item;
                };

                return Ok(None);
            };

            match command {
                Schedule::Push => {
                    self.push();
                }
                Schedule::Pop => {
                    self.pop();
                }
                Schedule::BasicCommand(command) => {
                    let run = command.build();
                    return Ok(Some(run));
                }
                Schedule::StaticSetup(setup) => {
                    let run = setup.build();
                    return Ok(Some(run));
                }
                Schedule::NodeAction(node) => {
                    let run = node.build();
                    return Ok(Some(run));
                }
                Schedule::ActionStep(step) => {
                    let run = step.build(self.tree())?;
                    return Ok(Some(run));
                }
                Schedule::Step(step) => {
                    let group = step.build(c.cx, self.tree())?;

                    for run in group.main.into_iter().rev() {
                        self.main.push_front(run);
                    }

                    for run in group.post.into_iter().rev() {
                        self.post.push_front(run);
                    }

                    if let Some(run) = group.run {
                        return Ok(Some(run));
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct ScheduleBasicCommand {
    command: OsArg,
    args: Vec<OsArg>,
}

impl ScheduleBasicCommand {
    fn build(self) -> Run {
        Run::command(self.command, self.args)
    }
}

#[derive(Clone)]
pub(crate) struct ScheduleStaticSetup {
    command: &'static str,
    args: Vec<RString>,
    name: &'static str,
    skipped: Option<String>,
}

impl ScheduleStaticSetup {
    fn build(self) -> Run {
        Run::command(self.command, self.args)
            .with_name(Some(self.name))
            .with_skipped(self.skipped)
    }
}

#[derive(Clone)]
pub(crate) struct ScheduleNodeAction {
    uses: Rc<RStr>,
    path: Rc<Path>,
    node_version: u32,
    skipped: Option<String>,
    env: Rc<BTreeMap<String, RString>>,
    file_env: Rc<BTreeMap<String, Rc<Path>>>,
    env_file: Rc<Path>,
    output_file: Rc<Path>,
}

impl ScheduleNodeAction {
    fn build(self) -> Run {
        let mut env = self
            .env
            .as_ref()
            .iter()
            .map(|(k, v)| (k.clone(), OsArg::from(v)))
            .collect::<BTreeMap<_, _>>();

        for (key, value) in self.file_env.as_ref() {
            env.insert(key.clone(), OsArg::from(value));
        }

        Run::node(self.node_version, self.path)
            .with_name(Some(self.uses))
            .with_skipped(self.skipped)
            .with_env(env)
            .with_env_is_file(self.file_env.keys().cloned())
            .with_env_file(Some(self.env_file))
            .with_output_file(Some(self.output_file))
    }
}

#[derive(Clone)]
pub(crate) struct ScheduleActionStep {
    uses: Rc<RStr>,
    index: usize,
    len: usize,
    step: Rc<ActionStep>,
    env: Rc<BTreeMap<String, RString>>,
    file_env: Rc<BTreeMap<String, Rc<Path>>>,
    env_file: Rc<Path>,
    output_file: Rc<Path>,
    tree: Rc<Tree>,
    skipped: Option<String>,
}

impl ScheduleActionStep {
    fn build(self, parent: Option<&Tree>) -> Result<Run> {
        let mut tree = self.tree.as_ref().clone();

        if let Some(parent) = parent {
            tree.extend(parent);
        }

        let eval = Eval::new().with_tree(&tree);

        let mut new_env = BTreeMap::new();

        for (k, v) in &self.step.env {
            new_env.insert(k.clone(), eval.eval(v)?.into_owned());
        }

        tree.insert_prefix(["env"], new_env.clone());

        let eval = Eval::new().with_tree(&tree);
        let script = eval.eval(&self.step.run)?.into_owned();

        let id = self.step.id.as_ref().map(|v| eval.eval(v)).transpose()?;

        let shell = self.step.shell.as_ref().map(|v| eval.eval(v)).transpose()?;
        let shell = to_shell(shell.as_deref())?;

        let name = if self.len == 1 {
            self.uses.as_ref().to_owned()
        } else {
            RString::from(format!(
                "{} (step {} / {})",
                self.uses,
                self.index + 1,
                self.len
            ))
        };

        let mut env = self
            .env
            .as_ref()
            .iter()
            .map(|(k, v)| (k.clone(), OsArg::from(v)))
            .collect::<BTreeMap<_, _>>();

        for (key, value) in self.file_env.as_ref() {
            env.insert(key.clone(), OsArg::from(value));
        }

        env.extend(new_env.iter().map(|(k, v)| (k.clone(), OsArg::from(v))));

        let run = Run::script(script, shell)
            .with_id(id.map(Cow::into_owned))
            .with_name(Some(name))
            .with_skipped(self.skipped.clone())
            .with_env(env)
            .with_env_is_file(self.file_env.keys().cloned())
            .with_env_file(Some(self.env_file.clone()))
            .with_output_file(Some(self.output_file.clone()));

        Ok(run)
    }
}

#[derive(Clone)]
pub(crate) struct ScheduleStep {
    step: Step,
    runners: Rc<ActionRunners>,
    tree: Rc<Tree>,
}

impl ScheduleStep {
    fn build(self, cx: &Ctxt<'_>, parent: Option<&Tree>) -> Result<RunGroup> {
        let mut tree = self.tree.as_ref().clone();

        if let Some(parent) = parent {
            tree.extend(parent);
        }

        let eval = Eval::new().with_tree(&tree);

        let new_env = self
            .step
            .env
            .into_iter()
            .map(|(k, v)| Ok((k, eval.eval(&v)?.into_owned())))
            .collect::<Result<BTreeMap<_, _>>>()?;

        tree.insert_prefix(["env"], new_env);
        let eval = Eval::new().with_tree(&tree);

        let mut main = Vec::new();
        let mut post = Vec::new();

        let run = self.step.run.as_ref().map(|v| eval.eval(v)).transpose()?;

        let mut skipped = None;

        if let Some(condition) = &self.step.condition {
            if eval.test(condition)? {
                skipped = Some(condition.clone());
            }
        }

        if let Some(uses) = self.step.uses.as_deref() {
            let uses_exposed = uses.to_exposed();

            let with = self
                .step
                .with
                .iter()
                .map(|(k, v)| Ok((k.clone(), eval.eval(v)?.into_owned())))
                .collect::<Result<BTreeMap<_, _>>>()?;

            if let Some(rust_toolchain) = rust_toolchain(uses, &with)? {
                main.push(Schedule::Push);

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

                    main.push(Schedule::StaticSetup(ScheduleStaticSetup {
                        command: "rustup",
                        args: args.clone(),
                        name: "install toolchain",
                        skipped: skipped.clone(),
                    }));
                }

                main.push(Schedule::StaticSetup(ScheduleStaticSetup {
                    command: "rustup",
                    args: vec![RString::from("default"), rust_toolchain.version.to_owned()],
                    name: "set default rust version",
                    skipped: skipped.clone(),
                }));

                main.push(Schedule::Pop);
            }

            if !should_skip_use(uses_exposed.as_ref()) {
                let c = ActionConfig::default()
                    .with_skipped(skipped.as_ref())
                    .with_inputs(with);

                let (runner_main, runner_post) = self.runners.build(cx, uses, &c)?;

                main.push(Schedule::Push);
                main.extend(runner_main);
                main.push(Schedule::Pop);

                post.push(Schedule::Push);
                post.extend(runner_post);
                post.push(Schedule::Pop);
            }
        }

        let mut entry = None;

        if let Some(script) = run.as_deref() {
            let shell = self.step.shell.as_ref().map(|v| eval.eval(v)).transpose()?;
            let shell = to_shell(shell.as_deref())?;

            let name = self.step.name.as_ref().map(|v| eval.eval(v)).transpose()?;

            let working_directory = self
                .step
                .working_directory
                .as_ref()
                .map(|v| Ok::<_, anyhow::Error>(eval.eval(v)?.into_owned()))
                .transpose()?;

            let env = tree
                .get_prefix("env")
                .into_iter()
                .map(|(key, value)| (key.clone(), OsArg::from(value)))
                .collect::<BTreeMap<_, _>>();

            entry = Some(
                Run::script(script, shell)
                    .with_name(name.as_deref())
                    .with_env(env)
                    .with_skipped(skipped.clone())
                    .with_working_directory(working_directory),
            );
        }

        Ok(RunGroup {
            run: entry,
            main,
            post,
        })
    }
}

#[derive(Clone)]
pub(crate) enum Schedule {
    Push,
    Pop,
    BasicCommand(ScheduleBasicCommand),
    StaticSetup(ScheduleStaticSetup),
    NodeAction(ScheduleNodeAction),
    ActionStep(ScheduleActionStep),
    Step(ScheduleStep),
}

/// A run configuration.
pub(crate) struct Run {
    id: Option<RString>,
    run: RunKind,
    name: Option<RString>,
    env: BTreeMap<String, OsArg>,
    skipped: Option<String>,
    working_directory: Option<RString>,
    // If an environment file is supported, this is the path to the file to set up.
    env_file: Option<Rc<Path>>,
    // If an output file is supported, this is the path to the file to set up.
    output_file: Option<Rc<Path>>,
    env_is_file: HashSet<String>,
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

    /// Setup a command to run.
    fn node(node_version: u32, script_file: Rc<Path>) -> Self {
        Self::with_run(RunKind::Node {
            node_version,
            script_file: script_file.clone(),
        })
    }

    fn with_run(run: RunKind) -> Self {
        Self {
            id: None,
            run,
            name: None,
            env: BTreeMap::new(),
            skipped: None,
            working_directory: None,
            env_file: None,
            output_file: None,
            env_is_file: HashSet::new(),
        }
    }

    /// Modify the id of the run command.
    fn with_id(mut self, id: Option<RString>) -> Self {
        self.id = id;
        self
    }

    /// Modify the name of the run command.
    #[inline]
    fn with_name<S>(mut self, name: Option<S>) -> Self
    where
        S: AsRef<RStr>,
    {
        self.name = name.map(|name| name.as_ref().to_owned());
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

    /// Modify the output file of the run command.
    #[inline]
    fn with_output_file(mut self, output_file: Option<Rc<Path>>) -> Self {
        self.output_file = output_file;
        self
    }

    /// Mark environment variables which are files.
    #[inline]
    fn with_env_is_file<I>(mut self, env_is_file: I) -> Self
    where
        I: IntoIterator<Item: AsRef<str>>,
    {
        self.env_is_file = env_is_file
            .into_iter()
            .map(|s| s.as_ref().to_owned())
            .collect();
        self
    }
}

/// A run on configuration.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, ValueEnum)]
pub(crate) enum RunOn {
    /// Run on the same system (default).
    #[default]
    Same,
    /// Run over WSL.
    Wsl,
}

impl RunOn {
    fn build_runner(
        &self,
        c: &BatchConfig<'_, '_>,
        path: &Path,
        run: &Run,
        current_env: &BTreeMap<String, String>,
    ) -> Result<Runner> {
        match *self {
            Self::Same => setup_same(c, path, run),
            Self::Wsl => {
                let Some(wsl) = c.cx.system.wsl.first() else {
                    bail!("WSL not available");
                };

                Ok(setup_wsl(c, path, wsl, run, current_env))
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
    extra_env: BTreeMap<&'static str, OsArg>,
    paths: Vec<PathBuf>,
}

impl Runner {
    fn new(command: Command) -> Self {
        Self {
            command,
            extra_env: BTreeMap::new(),
            paths: Vec::new(),
        }
    }
}

fn parse_output(contents: &[u8]) -> Result<Vec<(String, String)>> {
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
fn rust_toolchain<'a>(
    uses: &'a RStr,
    with: &'a BTreeMap<String, RString>,
) -> Result<Option<RustToolchain<'a>>> {
    let Some((head, version)) = uses.split_once('@') else {
        return Ok(None);
    };

    let Some((_, what)) = head.split_once('/') else {
        return Ok(None);
    };

    if what != "rust-toolchain" {
        return Ok(None);
    }

    let version = with
        .get("toolchain")
        .map(RString::as_rstr)
        .unwrap_or(version);

    let components = with.get("components").map(RString::as_rstr);
    let targets = with.get("targets").map(RString::as_rstr);

    Ok(Some(RustToolchain {
        version,
        components,
        targets,
    }))
}

fn setup_same(c: &BatchConfig<'_, '_>, path: &Path, run: &Run) -> Result<Runner> {
    match &run.run {
        RunKind::Shell { script, shell } => match shell {
            Shell::Powershell => {
                let Some(powershell) = c.cx.system.powershell.first() else {
                    bail!("PowerShell not available");
                };

                let mut c = powershell.command(path);
                c.arg("-Command");
                c.arg(script);
                Ok(Runner::new(c))
            }
            Shell::Bash => {
                let Some(bash) = c.cx.system.bash.first() else {
                    if let Os::Windows = &c.cx.os {
                        tracing::warn!("{WINDOWS_BASH_MESSAGE}");
                    };

                    bail!("Bash is not available");
                };

                let mut c = bash.command(path);
                c.args(["-i", "-c"]);
                c.arg(script);

                let mut r = Runner::new(c);

                if !bash.paths.is_empty() {
                    r.paths.clone_from(&bash.paths);
                }

                Ok(r)
            }
        },
        RunKind::Command { command, args } => {
            let mut c = Command::new(command);
            c.args(args.as_ref());
            c.current_dir(path);
            Ok(Runner::new(c))
        }
        RunKind::Node {
            node_version,
            script_file,
        } => {
            let node = c.cx.system.find_node(*node_version)?;
            let mut c = Command::new(&node.path);
            c.arg(script_file);
            c.current_dir(path);
            Ok(Runner::new(c))
        }
    }
}

fn setup_wsl(
    c: &BatchConfig<'_, '_>,
    path: &Path,
    wsl: &Wsl,
    run: &Run,
    current_env: &BTreeMap<String, String>,
) -> Runner {
    let mut cmd = wsl.shell(path);

    let mut seen = HashSet::new();
    let mut wslenv = String::new();
    let mut extra_env = BTreeMap::new();

    match &run.run {
        RunKind::Shell { script, shell } => match shell {
            Shell::Powershell => {
                cmd.args(["powershell", "-Command"]);
                cmd.arg(script);
            }
            Shell::Bash => {
                cmd.args(["bash", "-i", "-c"]);
                cmd.arg(script);
            }
        },
        RunKind::Command { command, args } => {
            cmd.arg(command);
            cmd.args(args.as_ref());
        }
        RunKind::Node { script_file, .. } => {
            cmd.args(["bash", "-i", "-c", "node $KICK_SCRIPT_FILE"]);
            wslenv.push_str("KICK_SCRIPT_FILE/p");
            extra_env.insert("KICK_SCRIPT_FILE", OsArg::from(script_file));
        }
    }

    for e in c
        .env_passthrough
        .iter()
        .chain(c.env.keys())
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

        if run.env_is_file.contains(e) {
            wslenv.push_str("/p");
        }
    }

    extra_env.insert("WSLENV", OsArg::from(wslenv));

    let mut runner = Runner::new(cmd);
    runner.extra_env = extra_env;
    runner
}

/// System colors.
struct Colors {
    skip_cond: ColorSpec,
    title: ColorSpec,
    matrix: ColorSpec,
    warn: ColorSpec,
}

impl Colors {
    /// Construct colors system.
    fn new() -> Self {
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

/// An actions configuration.
#[derive(Default)]
pub(crate) struct ActionConfig {
    skipped: Option<String>,
    inputs: BTreeMap<String, RString>,
}

impl ActionConfig {
    /// Set the skipped status of the action.
    pub(crate) fn with_skipped<S>(mut self, skipped: Option<S>) -> Self
    where
        S: AsRef<RStr>,
    {
        self.skipped = skipped.map(|s| s.as_ref().to_string_lossy().into_owned());
        self
    }

    /// Set inputs variables for runner.
    pub(crate) fn with_inputs(mut self, inputs: BTreeMap<String, RString>) -> Self {
        self.inputs = inputs;
        self
    }
}

#[derive(Debug)]
struct ActionRunner {
    kind: ActionKind,
    action_path: Rc<Path>,
    defaults: BTreeMap<String, String>,
    state_dir: Rc<Path>,
    id: String,
}

#[derive(Default, Debug)]
pub(crate) struct ActionRunners {
    runners: HashMap<String, ActionRunner>,
}

impl ActionRunners {
    /// Build the run configurations of an action.
    pub(crate) fn build(
        &self,
        cx: &Ctxt<'_>,
        uses: &RStr,
        c: &ActionConfig,
    ) -> Result<(Vec<Schedule>, Vec<Schedule>)> {
        let exposed = uses.to_exposed();

        let Some(runner) = self.runners.get(exposed.as_ref()) else {
            bail!("Could not find action runner for {uses}");
        };

        let mut main = Vec::new();
        let mut post = Vec::new();

        let env_file = Rc::<Path>::from(runner.state_dir.join(format!("env-{}", runner.id)));
        let output_file = Rc::<Path>::from(runner.state_dir.join(format!("output-{}", runner.id)));

        let it = runner
            .defaults
            .iter()
            .map(|(k, v)| (k.clone(), RString::from(v.clone())));

        let inputs = it
            .chain(c.inputs.iter().map(|(k, v)| (k.clone(), v.clone())))
            .collect::<BTreeMap<_, _>>();

        let mut file_env = BTreeMap::new();

        file_env.insert(String::from("GITHUB_ENV"), env_file.clone());
        file_env.insert(String::from("GITHUB_OUTPUT"), output_file.clone());
        file_env.insert(
            String::from("GITHUB_ACTION_PATH"),
            runner.action_path.clone(),
        );

        let mut env = BTreeMap::new();

        for (key, value) in &inputs {
            let key = key.to_uppercase();
            env.insert(format!("INPUT_{key}"), value.clone());
        }

        let env = Rc::new(env);
        let file_env = Rc::new(file_env);

        let mut tree = Tree::new();

        tree.insert(["runner", "os"], cx.os.as_tree_value());

        tree.insert_prefix(
            ["env"],
            env.as_ref().iter().map(|(k, v)| (k.clone(), v.clone())),
        );

        tree.insert_prefix(
            ["env"],
            file_env
                .iter()
                .map(|(k, v)| (k.clone(), v.to_string_lossy().into_owned())),
        );

        tree.insert_prefix(["inputs"], inputs);

        let tree = Rc::new(tree);

        match &runner.kind {
            ActionKind::Node {
                main_path,
                post_path,
                node_version,
            } => {
                if let Some(path) = post_path {
                    post.push(Schedule::Push);
                    post.push(Schedule::NodeAction(ScheduleNodeAction {
                        uses: uses.as_rc(),
                        path: path.clone(),
                        node_version: *node_version,
                        skipped: c.skipped.clone(),
                        env: env.clone(),
                        file_env: file_env.clone(),
                        env_file: env_file.clone(),
                        output_file: output_file.clone(),
                    }));
                    post.push(Schedule::Pop);
                }

                main.push(Schedule::Push);
                main.push(Schedule::NodeAction(ScheduleNodeAction {
                    uses: uses.as_rc(),
                    path: main_path.clone(),
                    node_version: *node_version,
                    skipped: c.skipped.clone(),
                    env,
                    file_env,
                    env_file,
                    output_file,
                }));
                main.push(Schedule::Pop);
            }
            ActionKind::Composite { steps } => {
                let uses = uses.as_rc();

                for (index, s) in steps.iter().enumerate() {
                    main.push(Schedule::ActionStep(ScheduleActionStep {
                        uses: uses.clone(),
                        index,
                        len: steps.len(),
                        step: Rc::new(s.clone()),
                        env: env.clone(),
                        file_env: file_env.clone(),
                        env_file: env_file.clone(),
                        output_file: output_file.clone(),
                        tree: tree.clone(),
                        skipped: c.skipped.clone(),
                    }));
                }
            }
        }

        Ok((main, post))
    }
}

fn sync_github_use(
    runners: &mut HashMap<String, ActionRunner>,
    cx: &Ctxt<'_>,
    repo: &str,
    name: &str,
    versions: &BTreeSet<String>,
) -> Result<()> {
    let project_dirs = cx
        .paths
        .project_dirs
        .context("Kick does not have project directories")?;

    let cache_dir = project_dirs.cache_dir();
    let actions_dir = cache_dir.join("actions");
    let state_dir = Rc::from(cache_dir.join(STATE));
    let repo_dir = actions_dir.join(repo).join(name);
    let git_dir = repo_dir.join("git");

    if !git_dir.is_dir() {
        fs::create_dir_all(&git_dir)
            .with_context(|| anyhow!("Failed to create repo directory: {}", git_dir.display()))?;
    }

    let r = match gix::open(&git_dir) {
        Ok(r) => r,
        Err(gix::open::Error::NotARepository { .. }) => gix::init_bare(&git_dir)?,
        Err(error) => return Err(error).context("Failed to open or initialize cache repository"),
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

    tracing::debug!(?git_dir, ?url, "Syncing");

    match crate::gix::sync(&r, &url, &refspecs) {
        Ok(remotes) => {
            tracing::debug!(?url, ?repo, ?name, ?remotes, "Found remotes");

            for (remote_name, id) in remotes {
                let Some(version) = reverse.remove(&remote_name) else {
                    continue;
                };

                let work_dir = repo_dir.join(WORKDIR).join(version);

                fs::create_dir_all(&work_dir).with_context(|| {
                    anyhow!("Failed to create work directory: {}", work_dir.display())
                })?;

                let id_path = work_dir.join(GIT_OBJECT_ID_FILE);

                fs::write(&id_path, id.as_bytes())
                    .with_context(|| anyhow!("Failed to write object ID: {}", id_path.display()))?;

                out.push((work_dir, id, repo, name, version));
            }
        }
        Err(error) => {
            tracing::warn!("Failed to sync remote `{repo}/{name}` with remote `{url}`: {error}");
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
                return Err(e).context(anyhow!("{}: Failed to read object ID", id_path.display()))
            }
        };

        out.push((work_dir, id, repo, name, version));
    }

    for (work_dir, id, repo, name, version) in out {
        let key = format!("{repo}/{name}@{version}");
        tracing::debug!(?work_dir, "Exporting {key}");

        // Load an action runner directly out of a repository without checking it out.
        let Some(runner) = crate::action::load(&r, id, &work_dir, version)? else {
            tracing::warn!("Could not load runner for {key}");
            continue;
        };

        fs::create_dir_all(&state_dir)
            .with_context(|| anyhow!("Failed to create envs directory: {}", state_dir.display()))?;

        runners.insert(
            key,
            ActionRunner {
                kind: runner.kind,
                action_path: work_dir.into(),
                defaults: runner.defaults,
                state_dir: state_dir.clone(),
                id: id.to_string(),
            },
        );
    }

    Ok(())
}

fn os_to_run_on(cx: &Ctxt<'_>, os: &Os) -> Result<RunOn> {
    if cx.os == *os {
        return Ok(RunOn::Same);
    }

    if cx.os == Os::Windows && *os == Os::Linux && cx.system.wsl.first().is_some() {
        return Ok(RunOn::Wsl);
    }

    bail!("No support for {os:?} on current system {:?}", cx.os);
}

fn to_shell(shell: Option<&RStr>) -> Result<Shell> {
    let Some(shell) = shell else {
        return Ok(Shell::Bash);
    };

    match shell.to_exposed().as_ref() {
        "bash" => Ok(Shell::Bash),
        "powershell" => Ok(Shell::Powershell),
        _ => bail!("Unsupported shell: {shell}"),
    }
}

enum Use {
    Github(String, String, String),
}

fn parse_action(uses: &str) -> Result<Use> {
    let ((repo, name), version) = uses
        .split_once('@')
        .and_then(|(k, v)| Some((k.split_once('/')?, v)))
        .context("Expected <repo>/<name>@<version>")?;

    Ok(Use::Github(
        repo.to_owned(),
        name.to_owned(),
        version.to_owned(),
    ))
}
