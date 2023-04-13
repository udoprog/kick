use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fmt::Write;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Output, Stdio};

pub(crate) struct Command {
    command: OsString,
    args: Vec<OsString>,
    current_dir: Option<PathBuf>,
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
}

impl Command {
    pub(crate) fn new<S>(command: S) -> Self
    where
        S: AsRef<OsStr>,
    {
        Self {
            command: command.as_ref().into(),
            args: Vec::new(),
            current_dir: None,
            stdin: None,
            stdout: None,
            stderr: None,
        }
    }

    /// Add an argument to the command.
    pub(crate) fn arg<S>(&mut self, arg: S) -> &mut Self
    where
        S: AsRef<OsStr>,
    {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    /// Add arguments to the command.
    pub(crate) fn args<I>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        for arg in args {
            self.args.push(arg.as_ref().to_owned());
        }

        self
    }

    pub(crate) fn current_dir<P>(&mut self, dir: P) -> &mut Self
    where
        P: AsRef<Path>,
    {
        self.current_dir = Some(dir.as_ref().to_owned());
        self
    }

    #[tracing::instrument(skip_all, fields(command = self.display().to_string(), current_dir = ?self.current_dir_repr()))]
    pub(crate) fn status(&mut self) -> io::Result<ExitStatus> {
        tracing::trace!("Running for status");
        let mut command = self.command();
        let status = command.status()?;
        tracing::trace!(?status);
        Ok(status)
    }

    #[tracing::instrument(skip_all, fields(command = self.display().to_string(), current_dir = ?self.current_dir_repr()))]
    pub(crate) fn output(&mut self) -> io::Result<Output> {
        tracing::trace!("Running for output");
        let mut command = self.command();
        let output = command.output()?;
        tracing::trace!(?output.status);
        Ok(output)
    }

    pub(crate) fn stdin<T>(&mut self, stdin: T) -> &mut Self
    where
        T: Into<Stdio>,
    {
        self.stdin = Some(stdin.into());
        self
    }

    pub(crate) fn stdout<T>(&mut self, stdout: T) -> &mut Self
    where
        T: Into<Stdio>,
    {
        self.stdout = Some(stdout.into());
        self
    }

    pub(crate) fn stderr<T>(&mut self, stderr: T) -> &mut Self
    where
        T: Into<Stdio>,
    {
        self.stderr = Some(stderr.into());
        self
    }

    fn command(&mut self) -> std::process::Command {
        let mut command = std::process::Command::new(&self.command);

        command.args(&self.args[..]);

        if let Some(current_dir) = &self.current_dir {
            command.current_dir(current_dir);
        }

        if let Some(stdin) = self.stdin.take() {
            command.stdin(stdin);
        }

        if let Some(stdout) = self.stdout.take() {
            command.stdout(stdout);
        }

        if let Some(stderr) = self.stderr.take() {
            command.stderr(stderr);
        }

        command
    }

    /// Build a command representation.
    pub(crate) fn display(&self) -> Display<'_> {
        Display { inner: self }
    }

    /// Current dir representation.
    fn current_dir_repr(&self) -> Option<Cow<'_, str>> {
        Some(self.current_dir.as_ref()?.to_string_lossy())
    }
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Command")
            .field("command", &self.command)
            .field("args", &self.args)
            .field("current_dir", &self.current_dir)
            .finish()
    }
}

pub(crate) struct Display<'a> {
    inner: &'a Command,
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.inner.command.to_string_lossy().as_ref())?;

        for arg in &self.inner.args {
            f.write_char(' ')?;
            f.write_str(arg.to_string_lossy().as_ref())?;
        }

        Ok(())
    }
}
