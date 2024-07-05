use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{Context, Result};
use bstr::io::BufReadExt;
use bstr::ByteSlice;

use crate::process::Command;

#[derive(Debug)]
pub(crate) struct Dnf {
    pub(crate) path: PathBuf,
}

impl Dnf {
    #[inline]
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Set up a command.
    pub(crate) fn list_installed(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();

        let mut dnf = Command::new(&self.path);
        dnf.args(["list", "--installed"]);
        dnf.stdout(Stdio::piped());

        let output = dnf.output()?.stdout;

        for line in output.byte_lines().skip(1) {
            let mut line = line?;

            if line.is_empty() {
                continue;
            }

            let mut it = line.split_str(" ");
            let name = it.next().context("expected package name")?;

            let (name, _) = name.split_once_str(".").context("illegal name")?;

            line.resize(name.len(), b' ');
            out.push(String::from_utf8(line)?);
        }

        Ok(out)
    }
}
