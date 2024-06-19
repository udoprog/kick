use std::path::{Path, PathBuf};

use crate::process::Command;

#[derive(Debug)]
pub(crate) struct Generic {
    pub(crate) path: PathBuf,
}

impl Generic {
    #[inline]
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Set up a command.
    pub(crate) fn command<D>(&self, dir: D) -> Command
    where
        D: AsRef<Path>,
    {
        let mut c = Command::new(&self.path);
        c.current_dir(dir);
        c
    }
}
