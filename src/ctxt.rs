use std::cell::{Ref, RefCell};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, Context, Result};
use relative_path::RelativePath;

use crate::actions::Actions;
use crate::changes::{Change, Warning};
use crate::config::Config;
use crate::git::Git;
use crate::manifest::Package;
use crate::model::{Repo, RepoParams, RepoRef};
use crate::process::Command;
use crate::repo_sets::RepoSets;
use crate::rust_version::RustVersion;

pub(crate) struct Ctxt<'a> {
    pub(super) root: &'a Path,
    pub(crate) current_path: Option<&'a RelativePath>,
    pub(crate) config: &'a Config<'a>,
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
    pub(crate) fn root(&self) -> &Path {
        self.root
    }

    /// Get a repo path.
    pub(crate) fn to_path<P>(&self, path: P) -> PathBuf
    where
        P: AsRef<RelativePath>,
    {
        if let Some(current_path) = self.current_path {
            let output = current_path.relative(path);
            return PathBuf::from(output.as_str());
        }

        path.as_ref().to_path(self.root)
    }

    /// Construct a reporting context for the given repo.
    pub(crate) fn context<'ctx>(
        &'ctx self,
        repo: &'ctx RepoRef,
    ) -> impl Fn() -> anyhow::Error + 'ctx {
        move || {
            anyhow!(
                "In repo {}",
                empty_or_dot(self.to_path(repo.path())).display()
            )
        }
    }

    /// Get repo parameters for the given package.
    pub(crate) fn repo_params<'m>(
        &'m self,
        package: &'m Package,
        repo: &'m RepoRef,
    ) -> Result<RepoParams<'m>> {
        let variables = self.config.variables(repo);
        let package_params = package.package_params(repo)?;
        let random = repo.random();
        Ok(self
            .config
            .repo_params(self, package_params, random, variables))
    }

    /// Iterate over non-disabled modules.
    pub(crate) fn repos(&self) -> impl Iterator<Item = &Repo> + '_ {
        Repos {
            repos: self.repos,
            index: 0,
        }
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

/// Coerce an empty buffer into the `.` path if necessary.
pub(crate) fn empty_or_dot(path: PathBuf) -> PathBuf {
    if path.components().next().is_none() {
        PathBuf::from_iter([Component::CurDir])
    } else {
        path
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

struct Repos<'a> {
    repos: &'a [Repo],
    index: usize,
}

impl<'a> Iterator for Repos<'a> {
    type Item = &'a Repo;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let repo = self.repos.get(self.index)?;
            self.index += 1;

            if repo.is_disabled() {
                continue;
            }

            return Some(repo);
        }
    }
}
