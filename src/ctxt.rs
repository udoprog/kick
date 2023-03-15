use std::path::Path;
use std::process::{Command, Stdio};

use crate::actions::Actions;
use crate::config::Config;
use crate::model::Module;
use crate::rust_version::RustVersion;

pub(crate) struct Ctxt<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a Config,
    pub(crate) actions: &'a Actions<'a>,
    pub(crate) modules: Vec<Module<'a>>,
    pub(crate) github_auth: Option<String>,
    pub(crate) rustc_version: Option<RustVersion>,
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
