use std::borrow::Cow;
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
use termcolor::{ColorSpec, WriteColor};

use crate::config::Os;
use crate::once::Once;
use crate::process::{Command, OsArg};
use crate::rstr::{RStr, RString};
use crate::shell::Shell;
use crate::workflows::{Matrix, Step};
use crate::Distribution;

use super::{
    ActionConfig, BatchConfig, Env, Run, RunKind, RunOn, Schedule, ScheduleBasicCommand,
    ScheduleUse, Scheduler, Session,
};

const WINDOWS_BASH_MESSAGE: &str = r#"Bash is not installed by default on Windows!

To install it, consider:
* Run: winget install msys2.msys2
* Install manually from https://www.msys2.org/

If you install it in a non-standard location (other than C:\\msys64),
make sure that its usr/bin directory is in the system PATH."#;

/// A constructed workflow batch.
pub(crate) struct Batch {
    run_on: RunOn,
    os: Os,
    commands: Vec<Schedule>,
    matrix: Option<Matrix>,
}

impl Batch {
    pub(super) fn new(
        run_on: RunOn,
        os: Os,
        commands: Vec<Schedule>,
        matrix: Option<Matrix>,
    ) -> Self {
        Self {
            run_on,
            os,
            commands,
            matrix,
        }
    }

    /// Construct a batch from a single use.
    pub(super) fn with_use(
        batch: &BatchConfig<'_, '_>,
        c: &ActionConfig<'_>,
        id: impl AsRef<RStr>,
    ) -> Result<Self> {
        let env = Env::new(batch, None, Some(c))?;

        let u = Schedule::Use(ScheduleUse::new(
            id.as_ref().as_rc(),
            Rc::new(Step::default()),
            env,
        ));

        Ok(Self {
            run_on: RunOn::Same,
            os: batch.cx.os.clone(),
            commands: vec![u],
            matrix: None,
        })
    }

    /// Construct a batch with a single command.
    pub(crate) fn command<C, A>(os: Os, command: C, args: A) -> Self
    where
        C: Into<OsArg>,
        A: IntoIterator<Item: Into<OsArg>>,
    {
        Batch {
            run_on: RunOn::Same,
            os,
            commands: vec![Schedule::BasicCommand(ScheduleBasicCommand::new(
                command, args,
            ))],
            matrix: None,
        }
    }

    /// Commit a batch.
    pub(crate) fn commit<O>(
        self,
        o: &mut O,
        c: &BatchConfig<'_, '_>,
        session: &mut Session,
    ) -> Result<()>
    where
        O: ?Sized + WriteColor,
    {
        let mut scheduler = Scheduler::new();

        let scripts_dir = Once::new(|| {
            let dir = c.cx.paths.cache.context("Missing cache directory")?;
            Ok(dir.join("scripts"))
        });

        for (run_on, os) in self.runners(&c.run_on) {
            match run_on {
                RunOn::Same => {
                    session.is_same = true;
                }
                RunOn::Wsl(dist) => {
                    session.dists.insert(dist);
                }
            }

            write!(o, "# In ")?;

            o.set_color(&c.colors.title)?;
            write!(o, "{}", c.path.display())?;
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

            while let Some(run) = scheduler.advance(o, c, session, &os)? {
                let modified;

                let path = match &run.working_directory {
                    Some(working_directory) => {
                        let working_directory = working_directory.to_exposed();
                        let working_directory = RelativePath::new(working_directory.as_ref());
                        modified = working_directory.to_logical_path(&c.path);
                        &modified
                    }
                    None => &c.path,
                };

                let mut run_command;
                let mut paths = &[][..];
                let mut display_command = None;
                let script_file;
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

                let mut skipped = run.skipped.as_deref();

                match run_on {
                    RunOn::Same => {
                        let skip;

                        (skip, run_command, paths, script_file) = setup_same(c, path, &run)?;

                        if skip && skipped.is_none() {
                            skipped = Some("incompatible with distro");
                        }

                        for (key, value) in env {
                            run_command.env(key, value);
                        }

                        if let Some(script) = &script_file {
                            if let ScriptFileKind::Inline { contents, .. } = &script.kind {
                                script_source = Some((Cow::Borrowed(contents.as_ref()), c.shell));
                            }
                        }
                    }
                    RunOn::Wsl(dist) => {
                        let Some(wsl) = c.cx.system.wsl.first() else {
                            bail!("WSL not available");
                        };

                        let mut command;
                        let wslenv;

                        (command, wslenv, script_file, script_source) =
                            setup_wsl(&run, env_keys.map(String::as_str));

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

                        display_command = Some(command);
                    }
                };

                let mut make_script = None;

                if let Some(script_file) = &script_file {
                    let script_path = match &script_file.kind {
                        ScriptFileKind::Inline { contents, ext } => {
                            let scripts_dir = scripts_dir.try_get()?;

                            let sequence = session.sequence();
                            let process_id = c.process_id;

                            let id = match scheduler.id("-", run.id.as_deref()) {
                                Some(mut id) => {
                                    id.push('-');
                                    id
                                }
                                None => RString::new(),
                            };

                            let script_path = Rc::<Path>::from(
                                scripts_dir.join(format!("kick-{id}{process_id}-{sequence}.{ext}")),
                            );

                            make_script = Some((script_path.clone(), contents));
                            script_path
                        }
                        ScriptFileKind::Existing { path } => path.clone(),
                    };

                    if let Some(variable) = script_file.variable {
                        run_command.env(variable, script_path.clone());
                    }

                    if script_file.argument {
                        run_command.arg(script_path.clone());
                    }
                }

                // Note that we don't want to pass PATH to WSL, it will only
                // confuse any processes running in there since those paths
                // points to OS-specified binaries.
                if !paths.is_empty() || !scheduler.paths().is_empty() {
                    let current_path;

                    let current_path = match run_on {
                        RunOn::Wsl(..) => None,
                        RunOn::Same => {
                            current_path = env::var_os("PATH");
                            current_path.as_ref().map(env::split_paths)
                        }
                    };

                    let paths = env::join_paths(
                        paths
                            .iter()
                            .cloned()
                            .chain(current_path.into_iter().flatten())
                            .chain(scheduler.paths().iter().cloned().map(PathBuf::from)),
                    )?;

                    run_command.env("PATH", paths);
                } else if matches!(run_on, RunOn::Wsl(..)) {
                    run_command.env_remove("PATH");
                }

                let display_impl;

                let display: &dyn fmt::Display;
                let shell;
                let display_env;
                let display_env_remove;

                'display: {
                    if c.verbose >= 2 {
                        display_impl = run_command.display_with(c.shell).with_exposed(c.exposed);
                        display = &display_impl;
                        shell = c.shell;
                        display_env = &run_command.env;
                        display_env_remove = &run_command.env_remove;
                        break 'display;
                    }

                    let current_command = display_command.as_ref().unwrap_or(&run_command);

                    display_env = &current_command.env;
                    display_env_remove = &current_command.env_remove;

                    if let Some((ref script_source, this_shell)) = script_source {
                        display = script_source;
                        shell = this_shell;
                        break 'display;
                    }

                    display_impl = display_command
                        .as_ref()
                        .unwrap_or(&run_command)
                        .display_with(c.shell)
                        .with_exposed(c.exposed);

                    display = &display_impl;
                    shell = c.shell;
                };

                let mut line = Line::new(o);

                if let Some(name) = scheduler.name(" / ", run.name.as_deref()) {
                    line.write(&c.colors.title, format_args!("{name}"))?;
                }

                if let Some(skipped) = skipped {
                    line.write(&c.colors.skip_cond, format_args!("(skipped: {skipped})"))?;
                }

                if c.verbose == 0 && !display_env.is_empty() && !display_env_remove.is_empty() {
                    let plural = pluralize(display_env.len(), "variable", "variables");

                    line.write(
                        &c.colors.warn,
                        format_args!("(see {} env {plural} with `-VV`)", display_env.len()),
                    )?;
                }

                line.finish()?;

                if c.verbose >= 1 || run.name.is_none() {
                    match shell {
                        Shell::Bash => {
                            if c.verbose >= 2 {
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

                                for key in display_env_remove {
                                    let key = key.to_string_lossy();
                                    write!(o, "{key}=\"\" ")?;
                                }
                            }

                            writeln!(o, "{display}")?;
                        }
                        Shell::Powershell => {
                            if c.verbose >= 2 && !display_env.is_empty() {
                                writeln!(o, "powershell -Command {{")?;

                                for (key, value) in display_env {
                                    let key = key.to_string_lossy();

                                    let value = if c.exposed {
                                        value.to_exposed_lossy()
                                    } else {
                                        value.to_string_lossy()
                                    };

                                    let value = shell.escape_string(value.as_ref());

                                    if shell.is_env_literal(key.as_ref()) {
                                        writeln!(o, r#"  $Env:{key}={value};"#)?;
                                    } else {
                                        writeln!(o, r#"  ${{Env:{key}={value}}};"#)?;
                                    }
                                }

                                for key in display_env_remove {
                                    let key = key.to_string_lossy();
                                    writeln!(o, r#"  Remove-Item Env:{key};"#)?;
                                }

                                writeln!(o, "  {display}")?;
                                writeln!(o, "}}")?;
                            } else {
                                writeln!(o, "{display}")?;
                            }
                        }
                    }

                    if c.verbose >= 2 {
                        if let Some((source, shell)) = &script_source {
                            o.set_color(&c.colors.title)?;
                            writeln!(o, "# {shell} script:")?;
                            o.reset()?;
                            writeln!(o, "{source}")?;
                        }
                    }
                }

                if skipped.is_none() && !c.dry_run {
                    truncate(run.files())?;

                    make_dirs(
                        run.dirs()
                            .chain(make_script.as_ref().and_then(|(s, _)| s.parent())),
                    )?;

                    if let Some((p, contents)) = make_script.take() {
                        tracing::trace!(?p, "Writing temporary script");

                        fs::write(&p, contents.to_exposed().as_bytes()).with_context(|| {
                            anyhow!("Failed to write script file: {}", p.display())
                        })?;

                        session.remove_path(&p);
                    }

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

                            if c.cx.os == Os::Windows && matches!(run_on, RunOn::Wsl(..)) {
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

                    tracing::debug!(?new_env, ?new_paths, ?new_outputs);

                    for (key, value) in new_env {
                        scheduler.env_mut().insert(key, value);
                    }

                    for line in new_paths {
                        scheduler.paths_mut().push(OsString::from(line));
                    }

                    if !new_outputs.is_empty() {
                        if let Some(id) = &run.id {
                            let id = id.to_exposed();
                            scheduler
                                .insert_new_outputs(id.as_ref(), &new_outputs)
                                .with_context(|| anyhow!("New outputs {new_outputs:?}"))?;
                        } else {
                            tracing::warn!("Outputs produced, but no id to store them");
                        }
                    }

                    purge_dirs(run.purge_dirs())?;
                }
            }
        }

        Ok(())
    }

    fn runners(&self, opts: &[(RunOn, Os)]) -> BTreeSet<(RunOn, Os)> {
        let mut set = BTreeSet::new();
        set.extend(opts.iter().cloned());
        set.insert((self.run_on, self.os.clone()));
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
            .with_context(|| anyhow!("Failed to create directory: {}", path.display()))?;
    }

    Ok(())
}

/// Purge directories.
fn purge_dirs<I>(paths: I) -> Result<()>
where
    I: IntoIterator<Item: AsRef<Path>>,
{
    for path in paths {
        let path = path.as_ref();

        match fs::remove_dir_all(path) {
            Ok(()) => {
                tracing::debug!(?path, "Removed directory");
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e)
                    .with_context(|| anyhow!("Failed to remove directory: {}", path.display()));
            }
        }
    }

    Ok(())
}

fn parse_key_values(contents: &[u8]) -> Result<Vec<(String, String)>> {
    process_lines(contents, |line| {
        let (key, value) = line.split_once('=')?;

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

#[derive(Debug)]
enum ScriptFileKind {
    Inline {
        contents: Box<RStr>,
        ext: &'static str,
    },
    Existing {
        path: Rc<Path>,
    },
}

#[derive(Debug)]
struct ScriptFile {
    variable: Option<&'static str>,
    argument: bool,
    kind: ScriptFileKind,
}

impl ScriptFile {
    fn inline(
        variable: Option<&'static str>,
        argument: bool,
        contents: Box<RStr>,
        ext: &'static str,
    ) -> Self {
        Self {
            variable,
            argument,
            kind: ScriptFileKind::Inline { contents, ext },
        }
    }

    fn path(variable: Option<&'static str>, argument: bool, path: Rc<Path>) -> Self {
        Self {
            variable,
            argument,
            kind: ScriptFileKind::Existing { path },
        }
    }
}

fn as_same_dist_specific(c: &BatchConfig<'_, '_>, command: &str) -> Option<Vec<Distribution>> {
    if c.cx.os != Os::Linux {
        return None;
    }

    let mut it = command
        .split_whitespace()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .skip_while(|s| s.starts_with('-'));

    let head = it.next()?;

    let command = if head == "sudo" { it.next()? } else { head };

    match command {
        "apt" => Some(vec![Distribution::Debian, Distribution::Ubuntu]),
        "dnf" => Some(vec![Distribution::Fedora]),
        "yum" => Some(vec![Distribution::Fedora]),
        _ => None,
    }
}

fn setup_same<'a>(
    c: &BatchConfig<'_, 'a>,
    path: &Path,
    run: &Run,
) -> Result<(bool, Command, &'a [PathBuf], Option<ScriptFile>)> {
    let mut skip = false;

    match &run.run {
        RunKind::Shell { script, shell } => {
            if let Some(dist) = as_same_dist_specific(c, script.to_exposed().as_ref()) {
                skip = !dist.contains(&c.cx.dist);
            }

            match shell {
                Shell::Powershell => {
                    let Some(powershell) = c.cx.system.powershell.first() else {
                        bail!("PowerShell not available");
                    };

                    let mut c = powershell.command_in(path);
                    c.arg("-Command");
                    c.arg(script);
                    Ok((skip, c, &[], None))
                }
                Shell::Bash => {
                    let Some(bash) = c.cx.system.bash.first() else {
                        if let Os::Windows = &c.cx.os {
                            tracing::warn!("{WINDOWS_BASH_MESSAGE}");
                        };

                        bail!("Bash is not available");
                    };

                    let mut c = bash.command_in(path);
                    c.args(["-i"]);
                    let script_file = ScriptFile::inline(None, true, script.clone(), "bash");
                    Ok((skip, c, &bash.paths, Some(script_file)))
                }
            }
        }
        RunKind::Command { command, args } => {
            let mut c = Command::new(command);
            c.args(args.as_ref());
            c.current_dir(path);
            Ok((skip, c, &[], None))
        }
        RunKind::Node {
            node_version,
            script_file,
        } => {
            let node = c.cx.system.find_node(*node_version)?;
            let mut c = Command::new(&node.path);
            c.arg(script_file);
            c.current_dir(path);
            Ok((skip, c, &[], None))
        }
    }
}

fn setup_wsl<'run, 'a>(
    run: &'run Run,
    env: impl IntoIterator<Item = &'a str>,
) -> (
    Command,
    String,
    Option<ScriptFile>,
    Option<(Cow<'run, RStr>, Shell)>,
) {
    let mut seen = HashSet::new();
    let mut wslenv = String::new();
    let mut script_file = None;
    let mut script_source = None;

    let mut c;

    match &run.run {
        RunKind::Shell { script, shell } => match shell {
            Shell::Powershell => {
                c = Command::new("powershell");
                c.args(["-Command"]);
                c.arg(script);

                script_source = Some((Cow::Borrowed(script.as_ref()), Shell::Powershell));
            }
            Shell::Bash => {
                c = Command::new("bash");
                c.args(["-i", "$KICK_SCRIPT_FILE"]);
                script_file = Some(ScriptFile::inline(
                    Some("KICK_SCRIPT_FILE"),
                    false,
                    script.clone(),
                    "bash",
                ));
                script_source = Some((Cow::Borrowed(script.as_ref()), Shell::Bash));
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
            c.args(["-i", "-c", "exec node $KICK_SCRIPT_FILE"]);
            script_file = Some(ScriptFile::path(
                Some("KICK_SCRIPT_FILE"),
                false,
                node_script_file.clone(),
            ));

            let source = RString::from("exec node $KICK_SCRIPT_FILE".to_owned());
            script_source = Some((Cow::Owned(source), Shell::Bash));
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

    if let Some(ScriptFile {
        variable: Some(variable),
        ..
    }) = &script_file
    {
        if !wslenv.is_empty() {
            wslenv.push(':');
        }

        wslenv.push_str(variable);
        wslenv.push_str("/p");
    }

    (c, wslenv, script_file, script_source)
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

fn pluralize<T>(len: usize, singular: T, plural: T) -> T {
    if len == 1 {
        singular
    } else {
        plural
    }
}

struct Line<'a, O>
where
    O: ?Sized,
{
    out: &'a mut O,
    written: bool,
}

impl<'a, O> Line<'a, O>
where
    O: ?Sized + WriteColor,
{
    fn new(out: &'a mut O) -> Self {
        Self {
            out,
            written: false,
        }
    }

    fn write(&mut self, color: &ColorSpec, args: impl fmt::Display) -> Result<()> {
        if !self.written {
            write!(self.out, "# ")?;
        } else {
            write!(self.out, " ")?;
        }

        self.out.set_color(color)?;
        write!(self.out, "{args}")?;
        self.out.reset()?;
        self.written = true;
        Ok(())
    }

    fn finish(self) -> Result<()> {
        if self.written {
            writeln!(self.out)?;
        }

        Ok(())
    }
}
