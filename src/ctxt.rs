use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::actions::Actions;
use crate::config::Config;
use crate::git::Git;
use crate::model::Module;
use crate::rust_version::RustVersion;

pub(crate) struct Ctxt<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a Config,
    pub(crate) actions: &'a Actions<'a>,
    pub(crate) modules: &'a [Module],
    pub(crate) github_auth: Option<String>,
    pub(crate) rustc_version: Option<RustVersion>,
    pub(crate) git: Option<Git>,
}

impl<'a> Ctxt<'a> {
    /// Iterate over non-disabled modules.
    pub(crate) fn modules(&self) -> impl Iterator<Item = &Module> + '_ {
        self.modules.iter().filter(move |m| !m.disabled)
    }

    /// Require a working git command.
    pub(crate) fn require_git(&self) -> Result<&Git> {
        self.git.as_ref().context("no working git command")
    }
}

/// Minor version from rustc.
pub(crate) fn rustc_version() -> Option<RustVersion> {
    let output = Command::new("rustc")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    let output = String::from_utf8(output.stdout).ok()?;
    let output = output.trim();
    tracing::trace!("rustc --version: {output}");
    let version = output.split(' ').nth(1)?;
    RustVersion::parse(version)
}
