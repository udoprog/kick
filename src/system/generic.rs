use std::path::{Path, PathBuf};

use crate::process::Command;

#[derive(Debug)]
pub(crate) struct Generic {
    pub(crate) path: PathBuf,
    pub(crate) paths: Vec<PathBuf>,
}

impl Generic {
    #[inline]
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            paths: Vec::new(),
        }
    }

    /// Add a path to use.
    #[cfg_attr(not(windows), allow(unused))]
    pub(crate) fn add_path(&mut self, path: PathBuf) {
        self.paths.push(path);
    }

    /// Set up a command.
    pub(crate) fn command(&self) -> Command {
        Command::new(&self.path)
    }

    /// Set up a command.
    pub(crate) fn command_in<D>(&self, dir: D) -> Command
    where
        D: AsRef<Path>,
    {
        let mut c = Command::new(&self.path);
        c.current_dir(dir);
        c
    }
}
