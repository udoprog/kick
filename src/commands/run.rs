use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::str;

use crate::process::OsArg;
use crate::rstr::{RStr, RString};
use crate::shell::Shell;

pub(super) enum RunKind {
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

/// A run configuration.
pub(crate) struct Run {
    pub(super) id: Option<RString>,
    pub(super) run: RunKind,
    pub(super) name: Option<RString>,
    pub(super) env: BTreeMap<String, OsArg>,
    pub(super) skipped: Option<String>,
    pub(super) working_directory: Option<RString>,
    // If an environment file is supported, this is the path to the file to set up.
    pub(super) env_file: Option<Rc<Path>>,
    // If an output file is supported, this is the path to the file to set up.
    pub(super) output_file: Option<Rc<Path>>,
    pub(super) env_is_file: HashSet<String>,
}

impl Run {
    /// Setup a command to run.
    pub(super) fn command<C, A>(command: C, args: A) -> Self
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
    pub(super) fn script(script: impl Into<Box<RStr>>, shell: Shell) -> Self {
        Self::with_run(RunKind::Shell {
            script: script.into(),
            shell,
        })
    }

    /// Setup a command to run.
    pub(super) fn node(node_version: u32, script_file: Rc<Path>) -> Self {
        Self::with_run(RunKind::Node {
            node_version,
            script_file: script_file.clone(),
        })
    }

    pub(super) fn with_run(run: RunKind) -> Self {
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
    pub(super) fn with_id(mut self, id: Option<RString>) -> Self {
        self.id = id;
        self
    }

    /// Modify the name of the run command.
    #[inline]
    pub(super) fn with_name<S>(mut self, name: Option<S>) -> Self
    where
        S: AsRef<RStr>,
    {
        self.name = name.map(|name| name.as_ref().to_owned());
        self
    }

    /// Modify the environment of the run command.
    #[inline]
    pub(super) fn with_env(mut self, env: BTreeMap<String, OsArg>) -> Self {
        self.env = env;
        self
    }

    /// Modify the skipped status of the run command.
    #[inline]
    pub(super) fn with_skipped(mut self, skipped: Option<String>) -> Self {
        self.skipped = skipped;
        self
    }

    /// Modify the working directory of the run command.
    #[inline]
    pub(super) fn with_working_directory(mut self, working_directory: Option<RString>) -> Self {
        self.working_directory = working_directory;
        self
    }

    /// Modify the environment file of the run command.
    #[inline]
    pub(super) fn with_env_file(mut self, env_file: Option<Rc<Path>>) -> Self {
        self.env_file = env_file;
        self
    }

    /// Modify the output file of the run command.
    #[inline]
    pub(super) fn with_output_file(mut self, output_file: Option<Rc<Path>>) -> Self {
        self.output_file = output_file;
        self
    }

    /// Mark environment variables which are files.
    #[inline]
    pub(super) fn with_env_is_file<I>(mut self, env_is_file: I) -> Self
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
