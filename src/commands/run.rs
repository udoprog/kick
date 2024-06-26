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
    pub(super) run: RunKind,
    pub(super) id: Option<Rc<RStr>>,
    pub(super) name: Option<RString>,
    pub(super) env: BTreeMap<String, OsArg>,
    pub(super) skipped: Option<String>,
    pub(super) working_directory: Option<RString>,
    // If an environment file is supported, this is the path to the file to set up.
    pub(super) env_file: Option<Rc<Path>>,
    // If an environment file is supported, this is the path to the file to set up.
    pub(super) path_file: Option<Rc<Path>>,
    // If an output file is supported, this is the path to the file to set up.
    pub(super) output_file: Option<Rc<Path>>,
    // The directory where to store tools, if possible.
    pub(super) tools_path: Option<Rc<Path>>,
    // The directory where temporary data is stored.
    pub(super) temp_path: Option<Rc<Path>>,
    // Environment variables which are files.
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
            run,
            id: None,
            name: None,
            env: BTreeMap::new(),
            skipped: None,
            working_directory: None,
            env_file: None,
            path_file: None,
            output_file: None,
            tools_path: None,
            temp_path: None,
            env_is_file: HashSet::new(),
        }
    }

    /// Modify the id of the run command.
    #[inline]
    pub(super) fn with_id(mut self, id: Option<Rc<RStr>>) -> Self {
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

    /// Modify the path file of the run command.
    #[inline]
    pub(super) fn with_path_file(mut self, path_file: Option<Rc<Path>>) -> Self {
        self.path_file = path_file;
        self
    }

    /// Modify the output file of the run command.
    #[inline]
    pub(super) fn with_output_file(mut self, output_file: Option<Rc<Path>>) -> Self {
        self.output_file = output_file;
        self
    }

    /// Modify the tools path.
    #[inline]
    pub(super) fn with_tools_path(mut self, tools_path: Rc<Path>) -> Self {
        self.tools_path = Some(tools_path);
        self
    }

    /// Modify the temp path.
    #[inline]
    pub(super) fn with_temp_path(mut self, temp_path: Rc<Path>) -> Self {
        self.temp_path = Some(temp_path);
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

    /// Iterate over all files associated with this run that should be cleaned
    /// up between each run.
    pub(super) fn files(&self) -> impl Iterator<Item = &Path> {
        self.env_file
            .as_slice()
            .iter()
            .chain(self.path_file.as_slice())
            .chain(self.output_file.as_slice())
            .map(Rc::as_ref)
    }

    /// Iterate over all directories that should exist.
    pub(super) fn dirs(&self) -> impl Iterator<Item = &Path> {
        self.tools_path
            .as_deref()
            .into_iter()
            .chain(self.temp_path.as_deref())
    }

    /// Iterate over directories that should be purged after the run.
    pub(super) fn purge_dirs(&self) -> impl Iterator<Item = &Path> {
        self.temp_path.as_deref().into_iter()
    }
}
