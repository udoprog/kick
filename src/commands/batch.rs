use std::collections::{BTreeSet, HashSet};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str;

use anyhow::{anyhow, bail, ensure, Context, Result};
use relative_path::{Component, RelativePath};
use termcolor::WriteColor;

use crate::config::Os;
use crate::process::{Command, OsArg};
use crate::rstr::{RStr, RString};
use crate::shell::Shell;
use crate::workflows::{Matrix, Step};

use super::{
    new_env, ActionConfig, BatchConfig, Prepare, Run, RunKind, RunOn, Schedule,
    ScheduleBasicCommand, ScheduleUse, Scheduler,
};

const WINDOWS_BASH_MESSAGE: &str = r#"Bash is not installed by default on Windows!

To install it, consider:
* Run: winget install msys2.msys2
* Install manually from https://www.msys2.org/

If you install it in a non-standard location (other than C:\\msys64),
make sure that its usr/bin directory is in the system PATH."#;

/// A constructed workflow batch.
pub(crate) struct Batch {
    commands: Vec<Schedule>,
    run_on: RunOn,
    matrix: Option<Matrix>,
}

impl Batch {
    pub(super) fn new(commands: Vec<Schedule>, run_on: RunOn, matrix: Option<Matrix>) -> Self {
        Self {
            commands,
            run_on,
            matrix,
        }
    }

    /// Construct a batch from a single use.
    pub(super) fn with_use(
        batch: &BatchConfig<'_, '_>,
        c: &ActionConfig,
        id: impl AsRef<RStr>,
    ) -> Result<Self> {
        let (env, tree) = new_env(batch, None, Some(c))?;

        Ok(Self {
            commands: vec![Schedule::Use(ScheduleUse::new(
                id.as_ref().to_owned(),
                Step::default(),
                Rc::new(tree),
                env,
            ))],
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
            commands: vec![Schedule::BasicCommand(ScheduleBasicCommand::new(
                command, args,
            ))],
            run_on: RunOn::Same,
            matrix: None,
        }
    }

    /// Commit a batch.
    pub(crate) fn commit<O>(
        self,
        o: &mut O,
        batch: &BatchConfig<'_, '_>,
        prepare: &mut Prepare,
    ) -> Result<()>
    where
        O: ?Sized + WriteColor,
    {
        let mut scheduler = Scheduler::new();

        for run_on in self.runners(&batch.run_on) {
            if let RunOn::Wsl(dist) = run_on {
                prepare.dists.insert(dist);
            }

            write!(o, "# In ")?;

            o.set_color(&batch.colors.title)?;
            write!(o, "{}", batch.path.display())?;
            o.reset()?;

            if let RunOn::Wsl(dist) = run_on {
                write!(o, " on ")?;

                o.set_color(&batch.colors.title)?;
                write!(o, "{dist} (WSL)")?;
                o.reset()?;
            }

            if let Some(matrix) = &self.matrix {
                write!(o, " ")?;

                o.set_color(&batch.colors.matrix)?;
                write!(o, "{}", matrix.display())?;
                o.reset()?;
            }

            writeln!(o)?;

            for run in self.commands.iter() {
                scheduler.push_back(run.clone());
            }

            let mut step = 0usize;

            while let Some(run) = scheduler.advance(o, batch, prepare)? {
                let modified;

                let path = match &run.working_directory {
                    Some(working_directory) => {
                        let working_directory = working_directory.to_exposed();
                        let working_directory = RelativePath::new(working_directory.as_ref());
                        modified = working_directory.to_logical_path(&batch.path);
                        &modified
                    }
                    None => &batch.path,
                };

                let mut run_command;
                let mut paths = &[][..];
                let mut display_command = None;
                let mut script_source = None;
                let script_file;

                let env_keys = batch.env_passthrough.iter();
                let env_keys = env_keys.chain(batch.env.keys());
                let env_keys = env_keys.chain(run.env.keys());
                let env_keys = env_keys.chain(scheduler.env().keys());

                let env = batch.env.iter().map(|(k, v)| (k.clone(), OsArg::from(v)));
                let env = env.chain(run.env.iter().map(|(k, v)| (k.clone(), v.clone())));
                let env = env.chain(
                    scheduler
                        .env()
                        .iter()
                        .map(|(k, v)| (k.clone(), OsArg::from(v))),
                );

                match run_on {
                    RunOn::Same => {
                        (run_command, paths, script_file) = setup_same(batch, path, &run)?;

                        for (key, value) in env {
                            run_command.env(key, value);
                        }
                    }
                    RunOn::Wsl(dist) => {
                        let Some(wsl) = batch.cx.system.wsl.first() else {
                            bail!("WSL not available");
                        };

                        let mut command;
                        let wslenv;
                        let kick_script_file;

                        (
                            command,
                            wslenv,
                            kick_script_file,
                            script_file,
                            script_source,
                        ) = setup_wsl(&run, env_keys.map(String::as_str));

                        run_command = wsl.shell(path, dist);
                        run_command.arg(&command.command);
                        run_command.args(&command.args);

                        for (key, value) in env {
                            run_command.env(key.clone(), value.clone());
                            command.env(key, value);
                        }

                        if !wslenv.is_empty() {
                            run_command.env("WSLENV", wslenv);
                        }

                        if let Some(kick_script_file) = kick_script_file {
                            run_command.env("KICK_SCRIPT_FILE", kick_script_file);
                        }

                        display_command = Some(command);
                    }
                };

                if !paths.is_empty() || !scheduler.paths().is_empty() {
                    let current_path = env::var_os("PATH").unwrap_or_default();
                    let current_path = env::split_paths(&current_path);

                    let paths = env::join_paths(
                        paths
                            .iter()
                            .cloned()
                            .chain(scheduler.paths().iter().cloned().map(PathBuf::from))
                            .chain(current_path),
                    )?;

                    run_command.env("PATH", paths);
                }

                step += 1;

                let display_impl;

                let (display, shell, display_env): (&dyn fmt::Display, _, _) = 'display: {
                    if batch.verbose == 2 {
                        display_impl = run_command
                            .display_with(batch.shell)
                            .with_exposed(batch.exposed);
                        break 'display (&display_impl, batch.shell, &run_command.env);
                    }

                    let display_env = &display_command.as_ref().unwrap_or(&run_command).env;

                    if let Some(script_source) = &script_source {
                        break 'display (script_source, Shell::Bash, display_env);
                    }

                    display_impl = display_command
                        .as_ref()
                        .unwrap_or(&run_command)
                        .display_with(Shell::Bash)
                        .with_exposed(batch.exposed);

                    (&display_impl, Shell::Bash, display_env)
                };

                write!(o, "# ")?;

                o.set_color(&batch.colors.title)?;

                if let Some(name) = &run.name {
                    write!(o, "{name}")?;
                } else {
                    write!(o, "step {step}")?;
                }

                o.reset()?;

                if let Some(skipped) = &run.skipped {
                    write!(o, " ")?;
                    o.set_color(&batch.colors.skip_cond)?;
                    write!(o, "(skipped: {skipped})")?;
                    o.reset()?;
                }

                if batch.verbose == 0 && !display_env.is_empty() {
                    let plural = if display_env.len() == 1 {
                        "variable"
                    } else {
                        "variables"
                    };

                    write!(o, " ")?;

                    o.set_color(&batch.colors.warn)?;
                    write!(o, "(see {} env {plural} with `-V`)", display_env.len())?;
                    o.reset()?;
                }

                writeln!(o)?;

                match shell {
                    Shell::Bash => {
                        if batch.verbose > 0 {
                            for (key, value) in display_env {
                                let key = key.to_string_lossy();

                                let value = if batch.exposed {
                                    value.to_exposed_lossy()
                                } else {
                                    value.to_string_lossy()
                                };

                                let value = shell.escape(value.as_ref());
                                write!(o, "{key}={value} ")?;
                            }
                        }

                        write!(o, "{display}")?;
                    }
                    Shell::Powershell => {
                        if batch.verbose > 0 && !display_env.is_empty() {
                            writeln!(o, "powershell -Command {{")?;

                            for (key, value) in display_env {
                                let key = key.to_string_lossy();

                                let value = if batch.exposed {
                                    value.to_exposed_lossy()
                                } else {
                                    value.to_string_lossy()
                                };

                                let value = shell.escape_string(value.as_ref());
                                writeln!(o, r#"  $Env:{key}={value};"#)?;
                            }

                            writeln!(o, "  {display}")?;
                            write!(o, "}}")?;
                        } else {
                            write!(o, "{display}")?;
                        }
                    }
                }

                writeln!(o)?;

                if run.skipped.is_none() && !batch.dry_run {
                    truncate(run.files())?;
                    make_dirs(run.dirs())?;

                    let status = run_command.status()?;

                    ensure!(status.success(), status);

                    let mut new_env = Vec::new();
                    let mut new_paths = Vec::new();
                    let mut new_outputs = Vec::new();

                    if let Some(env_file) = &run.env_file {
                        if let Ok(contents) = fs::read(env_file) {
                            new_env = parse_key_values(&contents)?;
                        }
                    }

                    if let Some(path_file) = &run.path_file {
                        if let Ok(contents) = fs::read(path_file) {
                            new_paths = parse_lines(&contents)?;

                            if batch.cx.os == Os::Windows && matches!(run_on, RunOn::Wsl(..)) {
                                for path in &mut new_paths {
                                    *path = translate_path_to_windows(path)?;
                                }
                            }
                        }
                    }

                    if let Some(output_file) = &run.output_file {
                        if let Ok(contents) = fs::read(output_file) {
                            new_outputs = parse_key_values(&contents)?;
                        }
                    }

                    tracing::debug!(id = ?run.id, ?new_env, ?new_paths, ?new_outputs);

                    for (key, value) in new_env {
                        scheduler.env_mut().insert(key, value);
                    }

                    for line in new_paths {
                        scheduler.paths_mut().push(OsString::from(line));
                    }

                    if !new_outputs.is_empty() {
                        scheduler
                            .insert_new_outputs(run.id.as_deref(), &new_outputs)
                            .with_context(|| anyhow!("New outputs {new_outputs:?}"))?;
                    }
                }
            }
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

        File::create(path)
            .with_context(|| anyhow!("Failed to truncate temporary file: {}", path.display()))?;
    }

    Ok(())
}

/// Create the given collection of directories.
fn make_dirs<I>(paths: I) -> Result<()>
where
    I: IntoIterator<Item: AsRef<Path>>,
{
    for path in paths {
        let path = path.as_ref();

        fs::create_dir_all(path)
            .with_context(|| anyhow!("Failed to create temporary directory: {}", path.display()))?;
    }

    Ok(())
}

fn parse_key_values(contents: &[u8]) -> Result<Vec<(String, String)>> {
    process_lines(contents, |line| {
        let (key, value) = line.split_once("=")?;

        let key = key.trim();
        let value = value.trim();

        if key.is_empty() || value.is_empty() {
            return None;
        }

        Some((key.to_owned(), value.to_owned()))
    })
}

fn parse_lines(contents: &[u8]) -> Result<Vec<String>> {
    process_lines(contents, |line| {
        let line = line.trim();

        if line.is_empty() {
            return None;
        }

        Some(line.to_owned())
    })
}

fn process_lines<F, O>(contents: &[u8], mut f: F) -> Result<Vec<O>>
where
    F: FnMut(&str) -> Option<O>,
{
    let mut out = Vec::new();
    let mut reader = BufReader::new(contents);
    let mut line = Vec::new();

    loop {
        line.clear();

        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }

        let Ok(line) = str::from_utf8(&line) else {
            continue;
        };

        if let Some(o) = f(line) {
            out.push(o);
        }
    }

    Ok(out)
}

struct ScriptFile {
    contents: Box<RStr>,
    ext: &'static str,
}

impl ScriptFile {
    fn new(contents: Box<RStr>, ext: &'static str) -> Self {
        Self { contents, ext }
    }
}

fn setup_same<'a>(
    c: &BatchConfig<'_, 'a>,
    path: &Path,
    run: &Run,
) -> Result<(Command, &'a [PathBuf], Option<ScriptFile>)> {
    match &run.run {
        RunKind::Shell { script, shell } => match shell {
            Shell::Powershell => {
                let Some(powershell) = c.cx.system.powershell.first() else {
                    bail!("PowerShell not available");
                };

                let mut c = powershell.command(path);
                c.arg("-Command");
                c.arg(script);
                Ok((c, &[], None))
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
                c.arg(OsArg::bash_escape(script.as_ref()));

                let script_file = ScriptFile::new(script.clone(), "bash");
                Ok((c, &bash.paths, Some(script_file)))
            }
        },
        RunKind::Command { command, args } => {
            let mut c = Command::new(command);
            c.args(args.as_ref());
            c.current_dir(path);
            Ok((c, &[], None))
        }
        RunKind::Node {
            node_version,
            script_file,
        } => {
            let node = c.cx.system.find_node(*node_version)?;
            let mut c = Command::new(&node.path);
            c.arg(script_file);
            c.current_dir(path);
            Ok((c, &[], None))
        }
    }
}

fn setup_wsl<'a>(
    run: &Run,
    env: impl IntoIterator<Item = &'a str>,
) -> (
    Command,
    String,
    Option<Rc<Path>>,
    Option<ScriptFile>,
    Option<RString>,
) {
    let mut seen = HashSet::new();
    let mut wslenv = String::new();
    let mut kick_script_file = None;
    let mut script_file = None;
    let mut script_source = None;

    let mut c;

    match &run.run {
        RunKind::Shell { script, shell } => match shell {
            Shell::Powershell => {
                c = Command::new("powershell");
                c.args(["-Command"]);
                c.arg(script);

                script_source = Some(script.as_ref().to_owned());
            }
            Shell::Bash => {
                c = Command::new("bash");
                c.args(["-i", "-c"]);
                c.arg(OsArg::bash_escape(script));
                script_source = Some(script.as_ref().to_owned());
                wslenv.push_str("KICK_SCRIPT_FILE/p");
                script_file = Some(ScriptFile::new(script.clone(), "bash"));
            }
        },
        RunKind::Command { command, args } => {
            c = Command::new(command);
            c.args(args.as_ref());
        }
        RunKind::Node {
            script_file: node_script_file,
            ..
        } => {
            c = Command::new("bash");
            c.args(["-i", "-c", "node \\$KICK_SCRIPT_FILE"]);
            wslenv.push_str("KICK_SCRIPT_FILE/p");
            kick_script_file = Some(node_script_file.clone());
            script_source = Some(format!("node {}", node_script_file.display()).into());
            script_file = Some(ScriptFile::new(
                Box::from(RStr::new("node $KICK_SCRIPT_FILE")),
                "js",
            ));
        }
    }

    for e in env {
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

    if kick_script_file.is_some() || script_file.is_some() {
        if !wslenv.is_empty() {
            wslenv.push(':');
        }

        wslenv.push_str("KICK_SCRIPT_FILE/p");
    }

    (c, wslenv, kick_script_file, script_file, script_source)
}

fn translate_path_to_windows(path: &str) -> Result<String> {
    let mut it = RelativePath::new(path)
        .components()
        .filter(|c| !c.as_str().is_empty());

    if !matches!(it.next(), Some(Component::Normal("mnt"))) {
        bail!("Path does not start with `mnt`");
    }

    let drive = it.next().context("Missing drive letter")?;

    let mut out = String::with_capacity(path.len());

    for c in drive.as_str().chars() {
        out.extend(c.to_uppercase());
    }

    out.push(':');

    for c in it {
        out.push('\\');
        out.push_str(c.as_str());
    }

    Ok(out)
}
