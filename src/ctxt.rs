use std::cell::{Ref, RefCell};
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};

use crate::actions::Actions;
use crate::changes::{Change, Warning};
use crate::config::Config;
use crate::git::Git;
use crate::manifest::ManifestPackage;
use crate::model::{Repo, RepoParams, RepoRef};
use crate::process::Command;
use crate::repo_sets::RepoSets;
use crate::rust_version::RustVersion;

pub(crate) struct Ctxt<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a Config,
    pub(crate) actions: &'a Actions<'a>,
    pub(crate) repos: &'a [Repo],
    pub(crate) github_auth: Option<String>,
    pub(crate) rustc_version: Option<RustVersion>,
    pub(crate) git: Option<Git>,
    pub(crate) warnings: RefCell<Vec<Warning>>,
    pub(crate) changes: RefCell<Vec<Change>>,
    pub(crate) sets: &'a mut RepoSets,
}

impl<'a> Ctxt<'a> {
    /// Get repo parameters for the given package.
    pub(crate) fn repo_params<'m>(
        &'m self,
        package: &'m ManifestPackage,
        repo: &'m RepoRef,
    ) -> Result<RepoParams<'m>> {
        let variables = self.config.variables(repo);
        let package_params = package.package_params(repo)?;
        Ok(self
            .config
            .repo_params(self, repo, package_params, variables))
    }

    /// Iterate over non-disabled modules.
    pub(crate) fn repos(&self) -> impl Iterator<Item = &Repo> + '_ {
        self.repos.iter().filter(move |m| !m.is_disabled())
    }

    /// Require a working git command.
    pub(crate) fn require_git(&self) -> Result<&Git> {
        self.git.as_ref().context("no working git command")
    }

    /// Push a change.
    pub(crate) fn warning(&self, warning: Warning) {
        self.warnings.borrow_mut().push(warning);
    }

    /// Push a change.
    pub(crate) fn change(&self, change: Change) {
        self.changes.borrow_mut().push(change);
    }

    /// Get a list of warnings.
    pub(crate) fn warnings(&self) -> Ref<'_, [Warning]> {
        Ref::map(self.warnings.borrow(), Vec::as_slice)
    }

    /// Get a list of proposed changes.
    pub(crate) fn changes(&self) -> Ref<'_, [Change]> {
        Ref::map(self.changes.borrow(), Vec::as_slice)
    }

    /// Check if there's a change we can save.
    pub(crate) fn can_save(&self) -> bool {
        !self.changes.borrow().is_empty()
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
