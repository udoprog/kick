use std::path::{Path, PathBuf};

use crate::process::Command;

#[derive(Debug)]
pub(crate) struct Wsl {
    pub(crate) path: PathBuf,
}

impl Wsl {
    #[inline]
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Set up a WSL shell command.
    pub(crate) fn shell<D>(&self, dir: D) -> Command
    where
        D: AsRef<Path>,
    {
        let mut command = Command::new(&self.path);
        command.args(["--shell-type", "login"]);
        command.current_dir(dir);
        command
    }
}
