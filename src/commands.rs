//! Helper system for setting up and running batches of commands.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::rc::Rc;
use std::str;

use anyhow::{anyhow, bail, ensure, Context, Result};
use bstr::BString;
use clap::{Parser, ValueEnum};
use gix::hash::Kind;
use gix::ObjectId;
use relative_path::RelativePath;
use termcolor::{Color, ColorSpec, WriteColor};

use crate::action::ActionKind;
use crate::config::{Distribution, Os};
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
const NODE_VERSION: u32 = 22;
const CURL: &str = "curl --proto '=https' --tlsv1.2 -sSf";

const DEBIAN_WANTED: &[&str] = &["gcc", "nodejs"];

const WINDOWS_BASH_MESSAGE: &str = r#"Bash is not installed by default on Windows!

To install it, consider:
* Run: winget install msys2.msys2
* Install manually from https://www.msys2.org/

If you install it in a non-standard location (other than C:\\msys64),
make sure that its usr/bin directory is in the system PATH."#;

enum Remediation {
    Command { title: String, command: Command },
}

/// Suggestions that might arise from a preparation.
#[derive(Default)]
pub(crate) struct Remediations {
    remediations: Vec<Remediation>,
}

impl Remediations {
    /// Test if suggestions are empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.remediations.is_empty()
    }

    fn command(&mut self, title: impl fmt::Display, command: Command) {
        self.remediations.push(Remediation::Command {
            title: title.to_string(),
            command,
        });
    }

    /// Apply remediations.
    pub(crate) fn apply<O>(self, o: &mut O, c: &BatchConfig<'_, '_>) -> Result<()>
    where
        O: ?Sized + WriteColor,
    {
        for remediation in self.remediations {
            match remediation {
                Remediation::Command { mut command, .. } => {
                    o.set_color(&c.colors.title)?;
                    writeln!(o, "Running: {}", command.display_with(c.shell))?;
                    o.reset()?;
                    let status = command.status()?;
                    ensure!(status.success(), status);
                }
            }
        }

        Ok(())
    }

    /// Print suggestions.
    pub(crate) fn print<O>(&self, o: &mut O, c: &BatchConfig<'_, '_>) -> Result<()>
    where
        O: ?Sized + WriteColor,
    {
        for remediation in &self.remediations {
            match remediation {
                Remediation::Command { title, command } => {
                    o.set_color(&c.colors.warn)?;
                    writeln!(o, "{title}")?;
                    o.reset()?;

                    writeln!(o, "  run: {}", command.display_with(c.shell))?;
                }
            }
        }

        Ok(())
    }
}

/// Preparations that need to be done before running a batch.
#[derive(Default)]
pub(crate) struct Prepare {
    /// WSL distributions that need to be available.
    wsl: BTreeSet<Distribution>,
    /// Actions that need to be synchronized.
    actions: Option<Actions>,
    /// Runners associated with actions.
    runners: Option<ActionRunners>,
}

impl Prepare {
    /// Access actions to prepare.
    pub(crate) fn actions_mut(&mut self) -> &mut Actions {
        self.actions.get_or_insert_with(Actions::default)
    }

    /// Run all preparations.
    pub(crate) fn prepare(&mut self, c: &BatchConfig<'_, '_>) -> Result<Remediations> {
        let mut suggestions = Remediations::default();

        if !self.wsl.is_empty() {
            let Some(wsl) = c.cx.system.wsl.first() else {
                bail!("WSL not available");
            };

            let available = wsl.list()?;

            let available = available
                .into_iter()
                .map(Distribution::from_string_ignore_case)
                .collect::<BTreeSet<_>>();

            for &dist in &self.wsl {
                let mut has_wsl = true;

                if dist != Distribution::Other && !available.contains(&dist) {
                    let mut command = Command::new(&wsl.path);
                    command
                        .arg("--install")
                        .arg(dist.to_string())
                        .arg("--no-launch");
                    suggestions.command(format_args!("WSL distro {dist} is missing"), command);

                    match dist {
                        Distribution::Ubuntu | Distribution::Debian => {
                            let mut command = Command::new("ubuntu");
                            command.arg("install");
                            suggestions.command(
                                format_args!("WSL distro {dist} needs to be configured"),
                                command,
                            );
                        }
                        _ => {}
                    }

                    has_wsl = false;
                }

                let has_rustup = if has_wsl {
                    let mut command = wsl.shell(c.repo_path, dist);
                    let status = command
                        .args(["rustup", "--version"])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()?;
                    status.success()
                } else {
                    false
                };

                if !has_rustup {
                    let mut command = wsl.shell(c.repo_path, dist);
                    command
                        .args(["bash", "-i", "-c"])
                        .arg(format!("{CURL} https://sh.rustup.rs | sh -s -- -y"));
                    suggestions
                        .command(format_args!("WSL distro {dist} is missing rustup"), command);
                }

                match dist {
                    Distribution::Ubuntu | Distribution::Debian => {
                        let mut wanted = BTreeSet::new();

                        for &package in DEBIAN_WANTED {
                            wanted.insert(package);
                        }

                        if has_wsl {
                            let output = wsl
                                .shell(c.repo_path, dist)
                                .args([
                                    "dpkg-query",
                                    "-W",
                                    "-f",
                                    "\\${db:Status-Status} \\${Package}\n",
                                ])
                                .stdout(Stdio::piped())
                                .output()?;

                            ensure!(
                                output.status.success(),
                                "Failed to query installed packages: {}",
                                output.status
                            );

                            for line in output.stdout.split(|&b| b == b'\n') {
                                let Ok(line) = str::from_utf8(line) else {
                                    continue;
                                };

                                if let Some(("installed", package)) = line.split_once(' ') {
                                    wanted.remove(package);
                                }
                            }
                        }

                        let wants_node_js = wanted.remove("nodejs");

                        if !wanted.is_empty() {
                            let packages = wanted.into_iter().collect::<Vec<_>>();
                            let packages = packages.join(" ");

                            let mut command = wsl.shell(c.repo_path, dist);
                            command.args(["bash", "-i", "-c"]).arg(format!(
                                "sudo apt update && sudo apt install --yes {packages}"
                            ));
                            suggestions.command(
                                format_args!("Some packages in {dist} are missing"),
                                command,
                            );
                        }

                        if wants_node_js {
                            let mut command = wsl.shell(c.repo_path, dist);
                            command.args(["bash", "-i", "-c"]).arg(format!(
                                "{CURL} https://deb.nodesource.com/setup_{NODE_VERSION}.x | sudo -E bash - && sudo apt-get install -y nodejs"
                            ));
                            suggestions.command(
                                format_args!("Missing a modern nodejs version in {dist}"),
                                command,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }

        if let Some(actions) = self.actions.take() {
            self.runners = Some(actions.synchronize(c.cx)?);
        }

        Ok(suggestions)
    }

    /// Access prepared runners, if they are available.
    pub(crate) fn runners(&self) -> Option<&ActionRunners> {
        self.runners.as_ref()
    }
}

/// A system of commands to be run.
pub(crate) struct WorkflowLoader<'a, 'cx> {
    cx: &'a Ctxt<'cx>,
    matrix_ignore: HashSet<String>,
}

impl<'a, 'cx> WorkflowLoader<'a, 'cx> {
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
    pub(crate) fn load_repo_workflows(
        &self,
        repo: &Repo,
        prepare: &mut Prepare,
    ) -> Result<Workflows<'a, 'cx>> {
        let mut workflows = Vec::new();
        let wfs = WorkflowManifests::new(self.cx, repo)?;

        for workflow in wfs.workflows() {
            let workflow = workflow?;

            let mut jobs = Vec::new();

            for job in workflow.jobs(&self.matrix_ignore)? {
                for (_, steps) in &job.matrices {
                    for step in &steps.steps {
                        if let Some(name) = &step.uses {
                            let actions = prepare.actions.get_or_insert_with(Actions::default);

                            actions.insert_action(name).with_context(|| {
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

        Ok(Workflows { workflows })
    }
}

/// Loaded uses.
#[derive(Default)]
pub(crate) struct Actions {
    actions: BTreeMap<(String, String), BTreeSet<String>>,
}

impl Actions {
    /// Add an action by id.
    pub(crate) fn insert_action<S>(&mut self, id: S) -> Result<()>
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
    pub(crate) fn synchronize(&self, cx: &Ctxt<'_>) -> Result<ActionRunners> {
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
}

impl<'a, 'cx> Workflows<'a, 'cx> {
    /// Iterate over workflows.
    pub(crate) fn iter<'this>(
        &'this self,
    ) -> impl Iterator<Item = &'this (WorkflowManifest<'a, 'cx>, Vec<Job>)> + 'this {
        self.workflows.iter()
    }

    /// Add jobs from a workflows, matrix, and associated steps.
    pub(crate) fn build_batch(
        &self,
        cx: &Ctxt<'_>,
        matrix: &Matrix,
        steps: &Steps,
        same_os: bool,
    ) -> Result<Batch> {
        let runs_on = steps.runs_on.to_exposed();

        let (os, dist) = match runs_on.split_once('-').map(|(os, _)| os) {
            Some("ubuntu") => (Os::Linux, Distribution::Ubuntu),
            Some("windows") => (Os::Windows, Distribution::Other),
            Some("macos") => (Os::Mac, Distribution::Other),
            _ => bail!("Unsupported runs-on directive: {}", steps.runs_on),
        };

        let run_on = if same_os {
            RunOn::Same
        } else {
            os_to_run_on(cx, &os, dist)?
        };

        let commands = build_steps(cx, &steps.steps, None, None)?;

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

fn new_env(
    cx: &Ctxt<'_>,
    runner: Option<&ActionRunner>,
    c: Option<&ActionConfig>,
) -> Result<(Env, Tree)> {
    let cache_dir = cx
        .paths
        .project_dirs
        .context("Missing project dirs for Kick")?
        .cache_dir();

    let state_dir = cache_dir.join("state");
    let env_file;
    let output_file;

    let mut file_env = BTreeMap::new();

    if let Some(runner) = runner {
        env_file = Rc::<Path>::from(runner.state_dir.join(format!("env-{}", runner.id)));
        output_file = Rc::<Path>::from(runner.state_dir.join(format!("output-{}", runner.id)));
        file_env.insert(
            String::from("GITHUB_ACTION_PATH"),
            runner.action_path.clone(),
        );
    } else {
        env_file = Rc::<Path>::from(state_dir.join("env"));
        output_file = Rc::<Path>::from(state_dir.join("output"));
    }

    file_env.insert(String::from("GITHUB_ENV"), env_file.clone());
    file_env.insert(String::from("GITHUB_OUTPUT"), output_file.clone());

    let mut env = BTreeMap::new();
    let mut tree = Tree::new();

    if let Some(c) = c {
        let mut inputs = BTreeMap::new();

        if let Some(runner) = runner {
            for (k, v) in &runner.defaults {
                inputs.insert(k.clone(), RString::from(v));
            }
        }

        inputs.extend(c.inputs.clone());

        if !inputs.is_empty() {
            for (key, value) in &inputs {
                let key = key.to_uppercase();
                env.insert(format!("INPUT_{key}"), value.clone());
            }

            tree.insert_prefix(["inputs"], inputs.clone());
        }
    }

    tree.insert(["runner", "os"], cx.os.as_tree_value());
    tree.insert_prefix(["env"], env.iter().map(|(k, v)| (k.clone(), v.clone())));
    tree.insert_prefix(
        ["env"],
        file_env
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string_lossy().into_owned())),
    );

    let env = Env {
        env: Rc::new(env),
        file_env: Rc::new(file_env),
        env_file,
        output_file,
    };

    Ok((env, tree))
}

/// Add jobs from a workflows, matrix, and associated steps.
fn build_steps(
    cx: &Ctxt<'_>,
    steps: &[Step],
    runner: Option<&ActionRunner>,
    c: Option<&ActionConfig>,
) -> Result<Vec<Schedule>> {
    let (env, tree) = new_env(cx, runner, c)?;

    let mut commands = Vec::new();

    for step in steps {
        let mut tree = tree.clone();
        tree.extend(&step.tree);
        let tree = Rc::new(tree);

        if let Some(run) = &step.run {
            commands.push(Schedule::Run(ScheduleRun {
                run: run.clone(),
                step: step.clone(),
                tree: tree.clone(),
                env: env.clone(),
            }));
        }

        if let Some(uses) = &step.uses {
            commands.push(Schedule::Use(ScheduleUse {
                uses: uses.clone(),
                step: step.clone(),
                tree: tree.clone(),
                env: env.clone(),
            }));
        }
    }

    Ok(commands)
}

#[derive(Default, Debug, Parser)]
pub(crate) struct BatchOptions {
    /// Run the command using the specified execution methods.
    #[arg(long, value_name = "run-on")]
    run_on: Vec<RunOnOption>,
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
    /// If there are any system remediations that have to be performed before
    /// running commands, apply them automatically.
    #[arg(long)]
    pub(crate) fix: bool,
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
            self.add_run_on(run_on.to_run_on())?;
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
        self.run_on
            .push(os_to_run_on(self.cx, os, Distribution::Ubuntu)?);
        Ok(())
    }

    /// Add a run on.
    pub(crate) fn add_run_on(&mut self, run_on: RunOn) -> Result<()> {
        if let RunOn::Wsl(..) = run_on {
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
    pub(crate) fn with_use(cx: &Ctxt<'_>, id: impl AsRef<RStr>, c: &ActionConfig) -> Result<Self> {
        let (env, tree) = new_env(cx, None, Some(c))?;

        Ok(Self {
            commands: vec![Schedule::Use(ScheduleUse {
                uses: id.as_ref().to_owned(),
                step: Step::default(),
                tree: Rc::new(tree),
                env,
            })],
            run_on: RunOn::Same,
            matrix: None,
        })
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

    /// Prepare running a batch.
    pub(crate) fn prepare(&self, c: &BatchConfig<'_, '_>, prepare: &mut Prepare) -> Result<()> {
        for run_on in self.runners(&c.run_on) {
            if let RunOn::Wsl(dist) = run_on {
                prepare.wsl.insert(dist);
            }
        }

        Ok(())
    }

    /// Commit a batch.
    pub(crate) fn commit<O>(
        self,
        o: &mut O,
        c: &BatchConfig<'_, '_>,
        runners: Option<&ActionRunners>,
    ) -> Result<()>
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

            while let Some(run) = scheduler.advance(c, runners)? {
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

    fn advance(
        &mut self,
        c: &BatchConfig<'_, '_>,
        runners: Option<&ActionRunners>,
    ) -> Result<Option<Run>> {
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
                    let run = node.build()?;
                    return Ok(Some(run));
                }
                Schedule::Run(run) => {
                    let run = run.build(self.tree())?;
                    return Ok(Some(run));
                }
                Schedule::Use(u) => {
                    let group = u.build(c.cx, self.tree(), runners)?;

                    for run in group.main.into_iter().rev() {
                        self.main.push_front(run);
                    }

                    for run in group.post.into_iter().rev() {
                        self.post.push_front(run);
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
    id: Option<Rc<RStr>>,
    uses: Rc<RStr>,
    path: Rc<Path>,
    node_version: u32,
    skipped: Option<String>,
    env: Env,
}

impl ScheduleNodeAction {
    fn build(self) -> Result<Run> {
        let (env, _) = self.env.build(None)?;

        let run = Run::node(self.node_version, self.path)
            .with_id(self.id.map(|id| id.as_ref().to_owned()))
            .with_name(Some(self.uses))
            .with_skipped(self.skipped)
            .with_env(env);

        Ok(self.env.decorate(run))
    }
}

#[derive(Clone)]
pub(crate) struct ScheduleUse {
    uses: RString,
    step: Step,
    tree: Rc<Tree>,
    env: Env,
}

impl ScheduleUse {
    fn build(
        self,
        cx: &Ctxt<'_>,
        parent: Option<&Tree>,
        runners: Option<&ActionRunners>,
    ) -> Result<RunGroup> {
        let mut tree = self.tree.as_ref().clone();

        if let Some(parent) = parent {
            tree.extend(parent);
        }

        let eval = Eval::new(&tree);
        let (_, tree_env) = self.env.build(Some((&eval, &self.step.env)))?;
        tree.insert_prefix(["env"], tree_env);
        let eval = Eval::new(&tree);

        let id = self.step.id.as_ref().map(|v| eval.eval(v)).transpose()?;

        let mut main = Vec::new();
        let mut post = Vec::new();

        let mut skipped = None;

        if let Some(condition) = &self.step.condition {
            if eval.test(condition)? {
                skipped = Some(condition.clone());
            }
        }

        let with = self
            .step
            .with
            .iter()
            .map(|(k, v)| Ok((k.clone(), eval.eval(v)?.into_owned())))
            .collect::<Result<BTreeMap<_, _>>>()?;

        if let Some(rust_toolchain) = rust_toolchain(&self.uses, &with)? {
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

        let uses_exposed = self.uses.to_exposed();

        if !should_skip_use(uses_exposed.as_ref()) {
            let c = ActionConfig::default()
                .with_id(id.map(Cow::into_owned))
                .with_skipped(skipped.as_ref())
                .with_inputs(with);

            let Some(runners) = runners else {
                bail!("No runners available for use");
            };

            let (runner_main, runner_post) = runners.build(cx, &c, &self.uses)?;

            main.push(Schedule::Push);
            main.extend(runner_main);
            main.push(Schedule::Pop);

            post.push(Schedule::Push);
            post.extend(runner_post);
            post.push(Schedule::Pop);
        }

        Ok(RunGroup { main, post })
    }
}

#[derive(Clone)]
struct Env {
    env: Rc<BTreeMap<String, RString>>,
    file_env: Rc<BTreeMap<String, Rc<Path>>>,
    env_file: Rc<Path>,
    output_file: Rc<Path>,
}

impl Env {
    fn build(
        &self,
        extra: Option<(&Eval<'_>, &BTreeMap<String, String>)>,
    ) -> Result<(BTreeMap<String, OsArg>, BTreeMap<String, RString>)> {
        let mut env = self
            .env
            .iter()
            .map(|(k, v)| (k.clone(), OsArg::from(v)))
            .collect::<BTreeMap<_, _>>();

        let mut tree_env = BTreeMap::new();

        for (key, value) in self.file_env.as_ref() {
            tree_env.insert(
                key.clone(),
                RString::from(value.to_string_lossy().into_owned()),
            );
            env.insert(key.clone(), OsArg::from(value));
        }

        if let Some((eval, input)) = extra {
            for (key, value) in input {
                let value = eval.eval(value)?.into_owned();
                tree_env.insert(key.clone(), value.clone());
                env.insert(key.clone(), OsArg::from(value));
            }
        }

        Ok((env, tree_env))
    }

    #[inline]
    fn decorate(&self, run: Run) -> Run {
        run.with_env_is_file(self.file_env.keys().cloned())
            .with_env_file(Some(self.env_file.clone()))
            .with_output_file(Some(self.output_file.clone()))
    }
}

#[derive(Clone)]
pub(crate) struct ScheduleRun {
    run: String,
    step: Step,
    tree: Rc<Tree>,
    env: Env,
}

impl ScheduleRun {
    fn build(self, parent: Option<&Tree>) -> Result<Run> {
        let mut tree = self.tree.as_ref().clone();

        if let Some(parent) = parent {
            tree.extend(parent);
        }

        let eval = Eval::new(&tree);
        let (env, tree_env) = self.env.build(Some((&eval, &self.step.env)))?;

        tree.insert_prefix(["env"], tree_env);
        let eval = Eval::new(&tree);

        let mut skipped = None;

        if let Some(condition) = &self.step.condition {
            if eval.test(condition)? {
                skipped = Some(condition.clone());
            }
        }

        let script = eval.eval(&self.run)?;

        let shell = self.step.shell.as_ref().map(|v| eval.eval(v)).transpose()?;
        let shell = to_shell(shell.as_deref())?;

        let id = self.step.id.as_ref().map(|v| eval.eval(v)).transpose()?;
        let name = self.step.name.as_ref().map(|v| eval.eval(v)).transpose()?;

        let working_directory = self
            .step
            .working_directory
            .as_ref()
            .map(|v| Ok::<_, anyhow::Error>(eval.eval(v)?.into_owned()))
            .transpose()?;

        let run = Run::script(script.as_ref(), shell)
            .with_id(id.map(Cow::into_owned))
            .with_name(name.as_deref())
            .with_env(env)
            .with_skipped(skipped.clone())
            .with_working_directory(working_directory);

        Ok(self.env.decorate(run))
    }
}

#[derive(Clone)]
pub(crate) enum Schedule {
    Push,
    Pop,
    BasicCommand(ScheduleBasicCommand),
    StaticSetup(ScheduleStaticSetup),
    NodeAction(ScheduleNodeAction),
    Run(ScheduleRun),
    Use(ScheduleUse),
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
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum RunOnOption {
    /// Run on the same system (default).
    Same,
    /// Run over WSL with the specified distribution.
    Wsl,
}

impl RunOnOption {
    /// Coerce into a [`RunOn`].
    fn to_run_on(self) -> RunOn {
        match self {
            Self::Same => RunOn::Same,
            Self::Wsl => RunOn::Wsl(Distribution::Ubuntu),
        }
    }
}

/// A run on configuration.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum RunOn {
    /// Run on the same system (default).
    #[default]
    Same,
    /// Run over WSL with the specified distribution.
    Wsl(Distribution),
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
            Self::Wsl(dist) => {
                let Some(wsl) = c.cx.system.wsl.first() else {
                    bail!("WSL not available");
                };

                Ok(setup_wsl(c, dist, path, wsl, run, current_env))
            }
        }
    }

    fn name(&self) -> Option<&str> {
        match *self {
            Self::Same => None,
            Self::Wsl(..) => Some("WSL"),
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
    dist: Distribution,
    path: &Path,
    wsl: &Wsl,
    run: &Run,
    current_env: &BTreeMap<String, String>,
) -> Runner {
    let mut cmd = wsl.shell(path, dist);

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
    id: Option<RString>,
    skipped: Option<String>,
    inputs: BTreeMap<String, RString>,
}

impl ActionConfig {
    /// Set the id of the action.
    pub(crate) fn with_id<S>(mut self, id: Option<S>) -> Self
    where
        S: AsRef<RStr>,
    {
        self.id = id.map(|s| s.as_ref().to_owned());
        self
    }

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
        c: &ActionConfig,
        uses: &RStr,
    ) -> Result<(Vec<Schedule>, Vec<Schedule>)> {
        let exposed = uses.to_exposed();

        let Some(runner) = self.runners.get(exposed.as_ref()) else {
            bail!("Could not find action runner for {uses}");
        };

        let mut main = Vec::new();
        let mut post = Vec::new();

        match &runner.kind {
            ActionKind::Node {
                main_path,
                post_path,
                node_version,
            } => {
                let id = c.id.as_deref().map(RStr::as_rc);
                let (env, _) = new_env(cx, Some(runner), Some(c))?;

                if let Some(path) = post_path {
                    post.push(Schedule::Push);
                    post.push(Schedule::NodeAction(ScheduleNodeAction {
                        id: id.clone(),
                        uses: uses.as_rc(),
                        path: path.clone(),
                        node_version: *node_version,
                        skipped: c.skipped.clone(),
                        env: env.clone(),
                    }));
                    post.push(Schedule::Pop);
                }

                main.push(Schedule::Push);
                main.push(Schedule::NodeAction(ScheduleNodeAction {
                    id: id.clone(),
                    uses: uses.as_rc(),
                    path: main_path.clone(),
                    node_version: *node_version,
                    skipped: c.skipped.clone(),
                    env,
                }));
                main.push(Schedule::Pop);
            }
            ActionKind::Composite { steps } => {
                let commands = build_steps(cx, steps, Some(runner), Some(c))?;
                main.extend(commands);
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

    let (r, open) = match gix::open(&git_dir) {
        Ok(r) => (r, true),
        Err(gix::open::Error::NotARepository { .. }) => (gix::init_bare(&git_dir)?, false),
        Err(error) => return Err(error).context("Failed to open or initialize cache repository"),
    };

    let mut refspecs = Vec::new();
    let mut reverse = HashMap::new();

    for version in versions {
        for remote_name in [
            BString::from(format!("refs/heads/{version}")),
            BString::from(format!("refs/tags/{version}")),
        ] {
            refspecs.push(remote_name.clone());
            reverse.insert(remote_name, version);
        }
    }

    let url = format!("{GITHUB_BASE}/{repo}/{name}");

    let mut out = Vec::new();

    tracing::debug!(?git_dir, ?url, "Syncing");

    let mut found = HashSet::new();

    match crate::gix::sync(&r, &url, &refspecs, open) {
        Ok(remotes) => {
            tracing::debug!(?url, ?repo, ?name, ?remotes, "Found remotes");

            for (remote_name, id) in remotes {
                let Some(version) = reverse.remove(&remote_name) else {
                    continue;
                };

                if found.contains(version) {
                    continue;
                }

                let (kind, action) = match crate::action::load(&r, &cx.eval, id) {
                    Ok(found) => found,
                    Err(error) => {
                        tracing::debug!(?remote_name, ?version, ?id, ?error, "Not an action");
                        continue;
                    }
                };

                found.insert(version);

                tracing::debug!(?remote_name, ?version, ?id, ?kind, "Found action");

                let work_dir = repo_dir.join(WORKDIR).join(version);

                fs::create_dir_all(&work_dir).with_context(|| {
                    anyhow!("Failed to create work directory: {}", work_dir.display())
                })?;

                let id_path = work_dir.join(GIT_OBJECT_ID_FILE);
                let existing = load_id(&id_path)?;

                let export = existing != Some(id);

                if export {
                    write_id(&id_path, id)?;
                }

                out.push((kind, action, work_dir, id, repo, name, version, export));
            }
        }
        Err(error) => {
            tracing::warn!("Failed to sync remote `{repo}/{name}` with remote `{url}`: {error}");
        }
    }

    // Try to read out remaining versions from the workdir cache.
    for version in versions {
        if !found.insert(version) {
            continue;
        }

        let work_dir = repo_dir.join(WORKDIR).join(version);
        let id_path = work_dir.join(GIT_OBJECT_ID_FILE);

        let Some(id) = load_id(&id_path)? else {
            continue;
        };

        // Load an action runner directly out of a repository without checking it out.
        let (kind, action) = crate::action::load(&r, &cx.eval, id)?;
        out.push((kind, action, work_dir, id, repo, name, version, false));
    }

    for (kind, action, work_dir, id, repo, name, version, export) in out {
        let key = format!("{repo}/{name}@{version}");
        tracing::debug!(?work_dir, key, export, "Loading runner");

        let runner = action.load(kind, &work_dir, version, export)?;

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

fn load_id(path: &Path) -> Result<Option<ObjectId>> {
    use bstr::ByteSlice;

    match fs::read(path) {
        Ok(id) => match ObjectId::from_hex(id.trim()) {
            Ok(id) => Ok(Some(id)),
            Err(e) => {
                _ = fs::remove_file(path);
                Err(e).context(anyhow!("{}: Failed to parse Object ID", path.display()))
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context(anyhow!("{}: Failed to read Object ID", path.display())),
    }
}

fn write_id(path: &Path, id: ObjectId) -> Result<()> {
    let mut buf = [0u8; const { Kind::longest().len_in_hex() + 8 }];
    let n = id.hex_to_buf(&mut buf[..]);
    buf[n] = b'\n';

    fs::write(path, &buf[..n + 1])
        .with_context(|| anyhow!("Failed to write Object ID: {}", path.display()))?;

    Ok(())
}

fn os_to_run_on(cx: &Ctxt<'_>, os: &Os, dist: Distribution) -> Result<RunOn> {
    if cx.os == *os && cx.dist.matches(dist) {
        return Ok(RunOn::Same);
    }

    if cx.os == Os::Windows && *os == Os::Linux && cx.system.wsl.first().is_some() {
        return Ok(RunOn::Wsl(dist));
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
