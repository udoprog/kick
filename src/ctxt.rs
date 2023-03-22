use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use relative_path::RelativePath;

use crate::actions::Actions;
use crate::config::Config;
use crate::git::Git;
use crate::glob::Fragment;
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
    pub(crate) current_path: Option<&'a RelativePath>,
    pub(crate) filters: &'a [Fragment<'a>],
}

impl<'a> Ctxt<'a> {
    pub(crate) fn modules(&self) -> impl Iterator<Item = &Module> + '_ {
        /// Test if module should be skipped.
        fn should_keep(
            filters: &[Fragment<'_>],
            current_path: Option<&RelativePath>,
            module: &Module,
        ) -> bool {
            if filters.is_empty() {
                if let Some(path) = current_path {
                    return path == module.path.as_ref();
                }

                return true;
            }

            filters
                .iter()
                .any(|filter| filter.is_match(module.path.as_str()))
        }

        self.modules
            .iter()
            .filter(move |m| should_keep(&self.filters, self.current_path, m))
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
