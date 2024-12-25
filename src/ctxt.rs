use std::cell::{Ref, RefCell};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{ExitCode, Stdio};

use anyhow::{anyhow, Context, Result};
use directories::ProjectDirs;
use relative_path::RelativePath;

use super::system::{Git, System};
use crate::cargo::{self, Package, RustVersion};
use crate::changes::{Change, Warning};
use crate::config::{Config, Distribution, Os};
use crate::env::{Env, SecretString};
use crate::model::{RenderRustVersions, Repo, RepoParams, RepoRef, State};
use crate::process::Command;
use crate::repo_sets::RepoSets;
use crate::{octokit, system};

/// Paths being used.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Paths<'a> {
    pub(super) root: &'a Path,
    pub(crate) current_path: Option<&'a RelativePath>,
    pub(crate) project_dirs: Option<&'a ProjectDirs>,
}

impl Paths<'_> {
    /// Get a repo path that is used as the base to other paths.
    pub(crate) fn to_path<P>(self, path: P) -> PathBuf
    where
        P: AsRef<RelativePath>,
    {
        if self.root.components().eq([Component::CurDir]) {
            return PathBuf::from(path.as_ref().as_str());
        }

        if let Some(current_path) = self.current_path {
            let output = current_path.relative(path);

            if output.components().next().is_none() {
                return PathBuf::from_iter([Component::CurDir]);
            }

            return PathBuf::from(output.as_str());
        }

        path.as_ref().to_path(self.root)
    }

    /// Read the given path to a string.
    ///
    /// Returns `None` if the given path does not exist.
    pub(crate) fn read_to_string<P>(self, path: P) -> Result<Option<String>>
    where
        P: AsRef<RelativePath>,
    {
        let path = self.to_path(path);

        match fs::read_to_string(&path) {
            Ok(input) => Ok(Some(input)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(anyhow::Error::from(e).context(format!("Reading {}", path.display()))),
        }
    }
}

pub(crate) struct Ctxt<'a> {
    pub(crate) system: &'a System,
    pub(crate) git_credentials: &'a Option<system::git::Credentials>,
    pub(crate) current_os: Os,
    pub(crate) dist: Distribution,
    pub(crate) paths: Paths<'a>,
    pub(crate) config: &'a Config<'a>,
    pub(crate) repos: &'a [Repo],
    pub(crate) rustc_version: Option<RustVersion>,
    pub(crate) warnings: RefCell<Vec<Warning>>,
    pub(crate) changes: RefCell<Vec<Change>>,
    pub(crate) sets: &'a mut RepoSets,
    pub(crate) env: &'a Env,
}

impl Ctxt<'_> {
    /// Get known github authentication.
    pub(crate) fn github_auth(&self) -> Option<SecretString> {
        if let Some(credentials) = &self.git_credentials {
            return Some(credentials.get());
        }

        if let Some(token) = &self.env.github_token {
            return Some(token.clone());
        }

        None
    }

    /// Grab an octokit client optionally configured with a token.
    pub(crate) fn octokit(&self) -> Result<octokit::Client> {
        let auth = match (&self.git_credentials, &self.env.github_token) {
            (Some(auth_manager), _) => octokit::Auth::Basic(auth_manager.get()),
            (_, Some(token)) => octokit::Auth::Bearer(token.clone()),
            _ => octokit::Auth::None,
        };

        octokit::Client::new(auth)
    }

    pub(crate) fn root(&self) -> &Path {
        self.paths.root
    }

    /// Convert a context into an outcome.
    pub(crate) fn outcome(&self) -> ExitCode {
        for repo in self.repos() {
            if repo.is_disabled() {
                continue;
            }

            if matches!(repo.state(), State::Error) {
                return ExitCode::FAILURE;
            }
        }

        ExitCode::SUCCESS
    }

    /// Get a repo path that is used as the base to other paths.
    pub(crate) fn to_path<P>(&self, path: P) -> PathBuf
    where
        P: AsRef<RelativePath>,
    {
        self.paths.to_path(path)
    }

    /// Construct a reporting context for the given repo.
    pub(crate) fn context<'ctx>(
        &'ctx self,
        repo: &'ctx RepoRef,
    ) -> impl Fn() -> anyhow::Error + 'ctx {
        move || anyhow!("Error in repo {}", self.to_path(repo.path()).display())
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

        Ok(RepoParams {
            package_params,
            rust_versions: RenderRustVersions {
                rustc: self.rustc_version,
                edition_2018: cargo::rust_version::EDITION_2018,
                edition_2021: cargo::rust_version::EDITION_2021,
            },
            random,
            variables,
        })
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
        self.system.git.first().context("no working git command")
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
