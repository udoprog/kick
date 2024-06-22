use std::collections::{BTreeSet, HashSet};
use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str;

use anyhow::{anyhow, bail, ensure, Context, Result};
use relative_path::RelativePath;
use termcolor::WriteColor;

use crate::config::Os;
use crate::ctxt::Ctxt;
use crate::process::{Command, OsArg};
use crate::rstr::{RStr, RString};
use crate::shell::Shell;
use crate::workflows::{Matrix, Step};

use super::new_env;
use super::{
    ActionConfig, ActionRunners, BatchConfig, Prepare, Run, RunKind, RunOn, Schedule,
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

    /// Construct a batch with multiple commands.
    pub(crate) fn with_use(cx: &Ctxt<'_>, id: impl AsRef<RStr>, c: &ActionConfig) -> Result<Self> {
        let (env, tree) = new_env(cx, None, Some(c))?;

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

            if let RunOn::Wsl(dist) = run_on {
                write!(o, " on ")?;

                o.set_color(&c.colors.title)?;
                write!(o, "{dist} (WSL)")?;
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
                scheduler.push_back(run.clone());
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

                let mut command;
                let mut paths = &[][..];
                let mut display_command = None;
                let mut script_source = None;

                let env_keys = c.env_passthrough.iter();
                let env_keys = env_keys.chain(c.env.keys());
                let env_keys = env_keys.chain(run.env.keys());
                let env_keys = env_keys.chain(scheduler.env().keys());

                let env = c.env.iter().map(|(k, v)| (k.clone(), OsArg::from(v)));
                let env = env.chain(run.env.iter().map(|(k, v)| (k.clone(), v.clone())));
                let env = env.chain(
                    scheduler
                        .env()
                        .iter()
                        .map(|(k, v)| (k.clone(), OsArg::from(v))),
                );

                match run_on {
                    RunOn::Same => {
                        (command, paths) = setup_same(c, path, &run)?;

                        for (key, value) in env {
                            command.env(key, value);
                        }
                    }
                    RunOn::Wsl(dist) => {
                        let Some(wsl) = c.cx.system.wsl.first() else {
                            bail!("WSL not available");
                        };

                        let (mut inner, wslenv, kick_script_file, inner_script_source) =
                            setup_wsl(&run, env_keys.map(String::as_str));

                        command = wsl.shell(path, dist);
                        command.arg(&inner.command);
                        command.args(&inner.args);

                        for (key, value) in env {
                            command.env(key.clone(), value.clone());
                            inner.env(key, value);
                        }

                        if !wslenv.is_empty() {
                            command.env("WSLENV", wslenv);
                        }

                        if let Some(kick_script_file) = kick_script_file {
                            command.env("KICK_SCRIPT_FILE", kick_script_file);
                        }

                        display_command = Some(inner);
                        script_source = inner_script_source;
                    }
                };

                if !paths.is_empty() {
                    let current_path = env::var_os("PATH").unwrap_or_default();
                    let current_path = env::split_paths(&current_path);
                    let paths = env::join_paths(paths.iter().cloned().chain(current_path))?;
                    command.env("PATH", paths);
                }

                step += 1;

                let display_impl;

                let (display, shell, display_env): (&dyn fmt::Display, _, _) = 'display: {
                    if c.verbose == 2 {
                        display_impl = command.display_with(c.shell).with_exposed(c.exposed);
                        break 'display (&display_impl, c.shell, &command.env);
                    }

                    let display_env = &display_command.as_ref().unwrap_or(&command).env;

                    if let Some(script_source) = &script_source {
                        break 'display (script_source, Shell::Bash, display_env);
                    }

                    display_impl = display_command
                        .as_ref()
                        .unwrap_or(&command)
                        .display_with(Shell::Bash)
                        .with_exposed(c.exposed);

                    (&display_impl, Shell::Bash, display_env)
                };

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

                if c.verbose == 0 && !display_env.is_empty() {
                    let plural = if display_env.len() == 1 {
                        "variable"
                    } else {
                        "variables"
                    };

                    write!(o, " ")?;

                    o.set_color(&c.colors.warn)?;
                    write!(o, "(see {} env {plural} with `-V`)", display_env.len())?;
                    o.reset()?;
                }

                writeln!(o)?;

                match shell {
                    Shell::Bash => {
                        if c.verbose > 0 {
                            for (key, value) in display_env {
                                let key = key.to_string_lossy();

                                let value = if c.exposed {
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
                        if c.verbose > 0 && !display_env.is_empty() {
                            writeln!(o, "powershell -Command {{")?;

                            for (key, value) in display_env {
                                let key = key.to_string_lossy();

                                let value = if c.exposed {
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

                if run.skipped.is_none() && !c.dry_run {
                    truncate(
                        run.env_file
                            .as_slice()
                            .iter()
                            .chain(run.output_file.as_slice()),
                    )?;

                    let status = command.status()?;
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

fn setup_same<'a>(
    c: &BatchConfig<'_, 'a>,
    path: &Path,
    run: &Run,
) -> Result<(Command, &'a [PathBuf])> {
    match &run.run {
        RunKind::Shell { script, shell } => match shell {
            Shell::Powershell => {
                let Some(powershell) = c.cx.system.powershell.first() else {
                    bail!("PowerShell not available");
                };

                let mut c = powershell.command(path);
                c.arg("-Command");
                c.arg(script);
                Ok((c, &[]))
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
                Ok((c, &bash.paths))
            }
        },
        RunKind::Command { command, args } => {
            let mut c = Command::new(command);
            c.args(args.as_ref());
            c.current_dir(path);
            Ok((c, &[]))
        }
        RunKind::Node {
            node_version,
            script_file,
        } => {
            let node = c.cx.system.find_node(*node_version)?;
            let mut c = Command::new(&node.path);
            c.arg(script_file);
            c.current_dir(path);
            Ok((c, &[]))
        }
    }
}

fn setup_wsl<'a>(
    run: &Run,
    env: impl IntoIterator<Item = &'a str>,
) -> (Command, String, Option<Rc<Path>>, Option<RString>) {
    let mut seen = HashSet::new();
    let mut wslenv = String::new();
    let mut kick_script_file = None;
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
                c.arg(script);

                script_source = Some(script.as_ref().to_owned());
            }
        },
        RunKind::Command { command, args } => {
            c = Command::new(command);
            c.args(args.as_ref());
        }
        RunKind::Node { script_file, .. } => {
            c = Command::new("bash");
            c.args(["-i", "-c", "node $KICK_SCRIPT_FILE"]);
            wslenv.push_str("KICK_SCRIPT_FILE/p");
            kick_script_file = Some(script_file.clone());
            script_source = Some(format!("node {}", script_file.display()).into());
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

    if kick_script_file.is_some() {
        if !wslenv.is_empty() {
            wslenv.push(':');
        }

        wslenv.push_str("KICK_SCRIPT_FILE/p");
    }

    (c, wslenv, kick_script_file, script_source)
}
