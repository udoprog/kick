use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, ChildStdout, ExitStatus, Output, Stdio};
use std::rc::Rc;

use anyhow::{anyhow, Context, Result};

use crate::rstr::{RStr, RString};
use crate::shell::Shell;

#[derive(Clone)]
enum OsArgKind {
    Path(Box<Path>),
    Str(Box<str>),
    OsStr(Box<OsStr>),
    RStr(Box<RStr>),
    /// An argument which is bash-argument escaped. For some reason, bash
    /// processes raw `$` in input arguments to `-c`. To avoid these from being
    /// evaluated, we have to escape them.
    ///
    /// If you don't believe me, try this:
    ///
    /// ```sh
    /// echo "echo $HOME"
    /// echo 'echo $HOME'
    /// bash -c 'echo $HOME'
    /// ```
    BashEscape(Box<RStr>),
}

/// A wrapper type that can losslessly represent many types which can be
/// converted into an `OsStr`.
#[derive(Clone)]
pub(crate) struct OsArg {
    kind: OsArgKind,
}

impl OsArg {
    /// Construct a bash escape os argument.
    pub(crate) fn bash_escape(value: impl AsRef<RStr>) -> Self {
        Self {
            kind: OsArgKind::BashEscape(Box::from(value.as_ref())),
        }
    }

    pub(crate) fn to_string_lossy(&self) -> Cow<'_, str> {
        match &self.kind {
            OsArgKind::Path(p) => p.to_string_lossy(),
            OsArgKind::Str(p) => Cow::Borrowed(p.as_ref()),
            OsArgKind::OsStr(s) => s.to_string_lossy(),
            OsArgKind::RStr(s) => Cow::Owned(s.to_string()),
            OsArgKind::BashEscape(s) => {
                let value = s.to_string_lossy();

                match bash_escape(value.as_ref()) {
                    Cow::Owned(value) => Cow::Owned(value),
                    Cow::Borrowed(..) => return value,
                }
            }
        }
    }

    pub(crate) fn to_exposed_lossy(&self) -> Cow<'_, str> {
        match &self.kind {
            OsArgKind::Path(p) => p.to_string_lossy(),
            OsArgKind::Str(p) => Cow::Borrowed(p.as_ref()),
            OsArgKind::OsStr(s) => s.to_string_lossy(),
            OsArgKind::RStr(s) => s.to_exposed(),
            OsArgKind::BashEscape(s) => {
                let value = s.to_string_lossy();

                match bash_escape(value.as_ref()) {
                    Cow::Owned(value) => Cow::Owned(value),
                    Cow::Borrowed(..) => value,
                }
            }
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
            OsArgKind::BashEscape(value) => {
                let value = value.to_exposed();

                match bash_escape(value.as_ref()) {
                    Cow::Owned(value) => Cow::Owned(OsString::from(value)),
                    Cow::Borrowed(..) => match value {
                        Cow::Owned(value) => Cow::Owned(OsString::from(value)),
                        Cow::Borrowed(value) => Cow::Borrowed(OsStr::new(value)),
                    },
                }
            }
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
            OsArgKind::BashEscape(s) => s.fmt(f),
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
}

impl Command {
    pub(crate) fn new<S>(command: S) -> Self
    where
        S: Into<OsArg>,
    {
        Self {
            command: command.into(),
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
        S: Into<OsArg>,
    {
        self.args.push(arg.into());
        self
    }

    /// Add arguments to the command.
    pub(crate) fn args<I>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item: Into<OsArg>>,
    {
        for arg in args {
            self.args.push(arg.into());
        }

        self
    }

    /// Add an environment variable to the command.
    pub(crate) fn env<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: Into<OsArg>,
    {
        self.env.push((key.as_ref().to_owned(), value.into()));
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
        let mut command = std::process::Command::new(self.command.to_os_str());

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

fn bash_escape(s: &str) -> Cow<'_, str> {
    let Some(at) = s.find('$') else {
        return Cow::Borrowed(s);
    };

    let mut escaped = String::with_capacity(s.len() + 2);

    let (head, tail) = s.split_at(at);

    escaped.push_str(head);

    for c in tail.chars() {
        match c {
            '$' => escaped.push_str(r"\$"),
            _ => escaped.push(c),
        }
    }

    Cow::Owned(escaped)
}
