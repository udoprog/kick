use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

use anyhow::{Context, Result};

use crate::process::Command;
use crate::rstr::RStr;

#[derive(Debug)]
pub(crate) struct PowerShell {
    command: PathBuf,
}

impl PowerShell {
    #[inline]
    pub(crate) fn new(command: PathBuf) -> Self {
        Self { command }
    }

    /// Run a powershell command.
    pub(crate) fn command<D>(&self, dir: D, command: &RStr) -> Command
    where
        D: AsRef<Path>,
    {
        let mut c = Command::new(&self.command);
        c.arg("-Command");
        c.arg(command);
        c.current_dir(dir);
        c
    }
}

pub(crate) fn test(path: &OsStr) -> Result<Option<ExitStatus>> {
    match std::process::Command::new(path)
        .arg("-Help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) => Ok(Some(status)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context("powershell -Help"),
    }
}
