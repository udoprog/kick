use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

use anyhow::{Context, Result};

use crate::process::Command;

#[derive(Debug)]
pub(crate) struct Wsl {
    command: PathBuf,
}

impl Wsl {
    #[inline]
    pub(crate) fn new(command: PathBuf) -> Self {
        Self { command }
    }

    /// Set up a WSL shell command.
    pub(crate) fn shell<D>(&self, dir: D) -> Command
    where
        D: AsRef<Path>,
    {
        let mut command = Command::new(&self.command);
        command.args(["--shell-type", "login"]);
        command.current_dir(dir);
        command
    }
}

pub(crate) fn version(path: &OsStr) -> Result<Option<ExitStatus>> {
    match std::process::Command::new(path)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) => Ok(Some(status)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context("docker --version"),
    }
}
