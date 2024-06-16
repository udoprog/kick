use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, ChildStdout, ExitStatus, Output, Stdio};

use anyhow::{anyhow, Context, Result};

use crate::model::ShellFlavor;
use crate::rstr::{RStr, RString};

pub(crate) enum Arg {
    OsString(OsString),
    Redact(RString),
}

impl Arg {
    pub(crate) fn to_string_lossy(&self) -> Cow<'_, str> {
        match self {
            Self::OsString(s) => s.to_string_lossy(),
            Self::Redact(s) => Cow::Owned(s.to_string()),
        }
    }
}

impl fmt::Debug for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OsString(s) => s.fmt(f),
            Self::Redact(s) => s.fmt(f),
        }
    }
}

pub(crate) struct Command {
    command: Arg,
    args: Vec<Arg>,
    current_dir: Option<PathBuf>,
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
    pub(crate) env: Vec<(OsString, Arg)>,
}

impl Command {
    pub(crate) fn new<S>(command: S) -> Self
    where
        S: AsRef<OsStr>,
    {
        Self::new_inner(Arg::OsString(command.as_ref().into()))
    }

    pub(crate) fn new_redact<S>(command: S) -> Self
    where
        S: AsRef<RStr>,
    {
        Self::new_inner(Arg::Redact(command.as_ref().into()))
    }

    fn new_inner(command: Arg) -> Self {
        Self {
            command,
            args: Vec::new(),
            current_dir: None,
            stdin: None,
            stdout: None,
            stderr: None,
            env: Vec::new(),
        }
    }

    /// Add an argument to the command.
    pub(crate) fn arg<S>(&mut self, arg: S) -> &mut Self
    where
        S: AsRef<OsStr>,
    {
        self.args.push(Arg::OsString(arg.as_ref().to_owned()));
        self
    }

    /// Add an argument to the command.
    pub(crate) fn arg_redact<S>(&mut self, arg: S) -> &mut Self
    where
        S: AsRef<RStr>,
    {
        self.args.push(Arg::Redact(arg.as_ref().to_owned()));
        self
    }

    /// Add arguments to the command.
    pub(crate) fn args<I>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        for arg in args {
            self.args.push(Arg::OsString(arg.as_ref().to_owned()));
        }

        self
    }

    /// Add an environment variable to the command.
    pub(crate) fn env<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.env.push((
            key.as_ref().to_owned(),
            Arg::OsString(value.as_ref().to_owned()),
        ));
        self
    }

    /// Add an environment variable to the command that might be redacted.
    pub(crate) fn env_redact<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: AsRef<RStr>,
    {
        self.env.push((
            key.as_ref().to_owned(),
            Arg::Redact(value.as_ref().to_owned()),
        ));
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
    pub(crate) fn spawn(&mut self) -> Result<Child> {
        let mut command = self.command();
        let result = command.spawn();
        let child = result.with_context(|| anyhow!("Spawning `{}`", self.display()))?;
        Ok(Child { child })
    }

    #[tracing::instrument(skip_all, fields(command = self.display().to_string(), current_dir = ?self.current_dir_repr()))]
    pub(crate) fn status(&mut self) -> Result<ExitStatus> {
        let mut command = self.command();
        let result = command.status();
        let status = result.with_context(|| anyhow!("Executing `{}`", self.display()))?;
        tracing::trace!(status = status.to_string());
        Ok(status)
    }

    #[tracing::instrument(skip_all, fields(command = self.display().to_string(), current_dir = ?self.current_dir_repr()))]
    pub(crate) fn output(&mut self) -> Result<Output> {
        let mut command = self.command();
        let output = command.output();
        let output = output.with_context(|| anyhow!("Executing `{}`", self.display()))?;
        tracing::trace!(status = output.status.to_string());
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
        let mut command = match &self.command {
            Arg::OsString(s) => std::process::Command::new(s),
            Arg::Redact(s) => {
                let s = s.to_redacted();
                std::process::Command::new(s.as_ref())
            }
        };

        for arg in &self.args {
            match arg {
                Arg::OsString(arg) => {
                    command.arg(arg);
                }
                Arg::Redact(arg) => {
                    command.arg(arg.to_redacted().as_ref());
                }
            }
        }

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

        for (key, value) in &self.env {
            match value {
                Arg::OsString(value) => {
                    command.env(key, value);
                }
                Arg::Redact(value) => {
                    command.env(key, value.to_redacted().as_ref());
                }
            }
        }

        command
    }

    /// Build a command representation.
    pub(crate) fn display(&self) -> Display<'_> {
        self.display_with(ShellFlavor::default())
    }

    /// Build a command representation.
    pub(crate) fn display_with(&self, flavor: ShellFlavor) -> Display<'_> {
        Display {
            inner: self,
            flavor,
        }
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

pub(crate) struct Child {
    child: std::process::Child,
}

impl Child {
    pub(crate) fn stdin(&mut self) -> Result<ChildStdin> {
        self.child.stdin.take().context("Missing stdin")
    }

    pub(crate) fn stdout(&mut self) -> Result<ChildStdout> {
        self.child.stdout.take().context("Missing stdout")
    }

    pub(crate) fn wait_with_output(self) -> Result<Output> {
        let output = self.child.wait_with_output()?;
        tracing::trace!(?output.status);
        Ok(output)
    }
}

pub(crate) struct Display<'a> {
    inner: &'a Command,
    flavor: ShellFlavor,
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lossy = self.inner.command.to_string_lossy();
        let escaped = crate::shell::escape(lossy.as_ref(), self.flavor);
        f.write_str(&escaped)?;

        for arg in &self.inner.args {
            f.write_char(' ')?;
            let lossy = arg.to_string_lossy();
            let escaped = crate::shell::escape(lossy.as_ref(), self.flavor);
            f.write_str(&escaped)?;
        }

        Ok(())
    }
}
