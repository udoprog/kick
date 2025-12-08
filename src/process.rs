use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Output, Stdio};
use std::rc::Rc;

use anyhow::{Context, Result, anyhow};
use tokio::process::{self, ChildStdin, ChildStdout};

use crate::rstr::{RStr, RString};
use crate::shell::Shell;

#[derive(Clone)]
enum OsArgKind {
    Path(Box<Path>),
    Str(Box<str>),
    OsStr(Box<OsStr>),
    RStr(Box<RStr>),
}

/// A wrapper type that can losslessly represent many types which can be
/// converted into an `OsStr`.
#[derive(Clone)]
pub(crate) struct OsArg {
    kind: OsArgKind,
}

impl OsArg {
    pub(crate) fn to_string_lossy(&self) -> Cow<'_, str> {
        match &self.kind {
            OsArgKind::Path(p) => p.to_string_lossy(),
            OsArgKind::Str(p) => Cow::Borrowed(p.as_ref()),
            OsArgKind::OsStr(s) => s.to_string_lossy(),
            OsArgKind::RStr(s) => Cow::Owned(s.to_string()),
        }
    }

    pub(crate) fn to_exposed_lossy(&self) -> Cow<'_, str> {
        match &self.kind {
            OsArgKind::Path(p) => p.to_string_lossy(),
            OsArgKind::Str(p) => Cow::Borrowed(p.as_ref()),
            OsArgKind::OsStr(s) => s.to_string_lossy(),
            OsArgKind::RStr(s) => s.to_exposed(),
        }
    }

    /// Convert the argument into an `OsStr`.
    pub(crate) fn to_os_str(&self) -> Cow<'_, OsStr> {
        match &self.kind {
            OsArgKind::Path(value) => Cow::Borrowed(value.as_ref().as_os_str()),
            OsArgKind::Str(value) => Cow::Borrowed(OsStr::new(value.as_ref())),
            OsArgKind::OsStr(value) => Cow::Borrowed(value),
            OsArgKind::RStr(value) => match value.to_exposed() {
                Cow::Owned(value) => Cow::Owned(OsString::from(value)),
                Cow::Borrowed(value) => Cow::Borrowed(OsStr::new(value)),
            },
        }
    }
}

impl fmt::Debug for OsArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            OsArgKind::Path(s) => s.fmt(f),
            OsArgKind::Str(s) => s.fmt(f),
            OsArgKind::OsStr(s) => s.fmt(f),
            OsArgKind::RStr(s) => s.fmt(f),
        }
    }
}

impl AsRef<OsArg> for OsArg {
    #[inline]
    fn as_ref(&self) -> &OsArg {
        self
    }
}

impl From<&OsArg> for OsArg {
    #[inline]
    fn from(s: &OsArg) -> Self {
        s.clone()
    }
}

macro_rules! from {
    ($variant:ident, $borrowed:ident, $owned:ty) => {
        impl From<Box<$borrowed>> for OsArg {
            #[inline]
            fn from(value: Box<$borrowed>) -> Self {
                Self {
                    kind: OsArgKind::$variant(value),
                }
            }
        }

        impl From<&Box<$borrowed>> for OsArg {
            #[inline]
            fn from(value: &Box<$borrowed>) -> Self {
                Self::from(value.clone())
            }
        }

        impl From<Rc<$borrowed>> for OsArg {
            #[inline]
            fn from(value: Rc<$borrowed>) -> Self {
                Self::from(&*value)
            }
        }

        impl From<&Rc<$borrowed>> for OsArg {
            #[inline]
            fn from(value: &Rc<$borrowed>) -> Self {
                Self::from(&**value)
            }
        }

        impl From<&$borrowed> for OsArg {
            #[inline]
            fn from(value: &$borrowed) -> Self {
                Self::from(Box::<$borrowed>::from(value))
            }
        }

        impl From<$owned> for OsArg {
            #[inline]
            fn from(value: $owned) -> Self {
                Self::from(&*value)
            }
        }

        impl From<&$owned> for OsArg {
            #[inline]
            fn from(value: &$owned) -> Self {
                Self::from(&**value)
            }
        }
    };
}

from!(Path, Path, PathBuf);
from!(Str, str, String);
from!(RStr, RStr, RString);
from!(OsStr, OsStr, OsString);

pub(crate) struct Command {
    pub(crate) command: OsArg,
    pub(crate) args: Vec<OsArg>,
    current_dir: Option<PathBuf>,
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
    pub(crate) env: Vec<(OsString, OsArg)>,
    pub(crate) env_remove: Vec<OsString>,
}

impl Command {
    pub(crate) fn new(command: impl Into<OsArg>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            current_dir: None,
            stdin: None,
            stdout: None,
            stderr: None,
            env: Vec::new(),
            env_remove: Vec::new(),
        }
    }

    /// Add an argument to the command.
    pub(crate) fn arg(&mut self, arg: impl Into<OsArg>) -> &mut Self {
        self.args.push(arg.into());
        self
    }

    /// Add arguments to the command.
    pub(crate) fn args(&mut self, args: impl IntoIterator<Item: Into<OsArg>>) -> &mut Self {
        for arg in args {
            self.args.push(arg.into());
        }

        self
    }

    /// Add an environment variable to the command.
    pub(crate) fn env(&mut self, key: impl AsRef<OsStr>, value: impl Into<OsArg>) -> &mut Self {
        self.env.push((key.as_ref().to_owned(), value.into()));
        self
    }

    /// Mark an environment variable to be removed.
    pub(crate) fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.env_remove.push(key.as_ref().to_owned());
        self
    }

    pub(crate) fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
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
    pub(crate) async fn status(&mut self) -> Result<ExitStatus> {
        let mut command = self.command();
        let result = command.status().await;
        let status = result.with_context(|| anyhow!("Executing `{}`", self.display()))?;
        tracing::trace!(status = status.to_string());
        Ok(status)
    }

    #[tracing::instrument(skip_all, fields(command = self.display().to_string(), current_dir = ?self.current_dir_repr()))]
    pub(crate) async fn output(&mut self) -> Result<Output> {
        let mut command = self.command();
        let output = command.output().await;
        let output = output.with_context(|| anyhow!("Executing `{}`", self.display()))?;
        tracing::trace!(status = output.status.to_string());
        Ok(output)
    }

    pub(crate) fn stdin(&mut self, stdin: impl Into<Stdio>) -> &mut Self {
        self.stdin = Some(stdin.into());
        self
    }

    pub(crate) fn stdout(&mut self, stdout: impl Into<Stdio>) -> &mut Self {
        self.stdout = Some(stdout.into());
        self
    }

    pub(crate) fn stderr(&mut self, stderr: impl Into<Stdio>) -> &mut Self {
        self.stderr = Some(stderr.into());
        self
    }

    fn command(&mut self) -> process::Command {
        let mut command = process::Command::new(self.command.to_os_str());

        for arg in &self.args {
            command.arg(arg.to_os_str());
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
            command.env(key, value.to_os_str());
        }

        for key in &self.env_remove {
            command.env_remove(key);
        }

        command
    }

    /// Display the current command as a bash command.
    #[inline]
    pub(crate) fn display(&self) -> Display<'_> {
        self.display_with(Shell::Bash)
    }

    /// Build a command representation.
    pub(crate) fn display_with(&self, shell: Shell) -> Display<'_> {
        Display {
            inner: self,
            shell,
            exposed: false,
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
    child: process::Child,
}

impl Child {
    pub(crate) fn stdin(&mut self) -> Result<ChildStdin> {
        self.child.stdin.take().context("Missing stdin")
    }

    pub(crate) fn stdout(&mut self) -> Result<ChildStdout> {
        self.child.stdout.take().context("Missing stdout")
    }

    pub(crate) async fn wait_with_output(self) -> Result<Output> {
        let output = self.child.wait_with_output().await?;
        tracing::trace!(?output.status);
        Ok(output)
    }
}

pub(crate) struct Display<'a> {
    inner: &'a Command,
    shell: Shell,
    exposed: bool,
}

impl Display<'_> {
    /// Configure display to be exposed.
    pub(crate) fn with_exposed(self, exposed: bool) -> Self {
        Self { exposed, ..self }
    }
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lossy = if self.exposed {
            self.inner.command.to_exposed_lossy()
        } else {
            self.inner.command.to_string_lossy()
        };

        let escaped = self.shell.escape(lossy.as_ref());

        if let (Shell::Powershell, Cow::Owned(..)) = (self.shell, &escaped) {
            "& ".fmt(f)?;
        }

        f.write_str(&escaped)?;

        for arg in &self.inner.args {
            f.write_char(' ')?;

            let lossy = if self.exposed {
                arg.to_exposed_lossy()
            } else {
                arg.to_string_lossy()
            };

            let escaped = self.shell.escape(lossy.as_ref());
            f.write_str(&escaped)?;
        }

        Ok(())
    }
}
