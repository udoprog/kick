use std::cell::{Ref, RefCell, RefMut};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use relative_path::{RelativePath, RelativePathBuf};

use super::system::{Git, System};
use crate::cargo::{self, Package, RustVersion};
use crate::changes::{Change, ChangeWrapper, Warning};
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
    pub(crate) current: Option<&'a RelativePath>,
    pub(crate) config: Option<&'a Path>,
    pub(crate) cache: Option<&'a Path>,
    /// Redirect paths under a repo to a git worktree of it.
    pub(crate) redirect: Option<Redirect<'a>>,
}

/// A registered repo whose working directory is a git worktree of it located
/// elsewhere. Both paths are relative to the root.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Redirect<'a> {
    pub(crate) repo: &'a RelativePath,
    pub(crate) worktree: &'a RelativePath,
}

impl Redirect<'_> {
    /// Rewrite a path under the repo into the same path under the worktree.
    ///
    /// Returns `None` if the path is not under the repo, or already under the
    /// worktree.
    pub(crate) fn rewrite(self, path: &RelativePath) -> Option<RelativePathBuf> {
        if path.starts_with(self.worktree) {
            return None;
        }

        let rest = path.strip_prefix(self.repo).ok()?;

        if rest.as_str().is_empty() {
            return Some(self.worktree.to_owned());
        }

        Some(self.worktree.join(rest))
    }
}

impl Paths<'_> {
    /// Get a repo path that is used as the base to other paths.
    pub(crate) fn to_path(self, path: impl AsRef<RelativePath>) -> PathBuf {
        let path = path.as_ref();
        let rewritten = self.redirect.and_then(|r| r.rewrite(path));
        let path = rewritten.as_deref().unwrap_or(path);

        if self.root.components().eq([Component::CurDir]) {
            return PathBuf::from(path.as_str());
        }

        if let Some(current_path) = self.current {
            let output = current_path.relative(path);

            if output.components().next().is_none() {
                return PathBuf::from_iter([Component::CurDir]);
            }

            return PathBuf::from(output.as_str());
        }

        path.to_path(self.root)
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
    pub(crate) term: Arc<AtomicBool>,
    pub(crate) system: &'a System,
    pub(crate) git_credentials: &'a Option<system::git::Credentials>,
    pub(crate) os: Os,
    pub(crate) dist: Distribution,
    pub(crate) paths: Paths<'a>,
    pub(crate) config: &'a Config<'a>,
    pub(crate) repos: &'a [Repo],
    pub(crate) rustc_version: Option<RustVersion>,
    pub(crate) warnings: RefCell<Vec<Warning>>,
    pub(crate) changes: RefCell<Vec<ChangeWrapper>>,
    pub(crate) sets: &'a mut RepoSets,
    pub(crate) env: &'a Env,
}

impl<'a> Ctxt<'a> {
    /// Check if context is terminated or not.
    pub(crate) fn is_terminated(&self) -> bool {
        self.term.load(Ordering::Relaxed)
    }

    /// Get known github authentication.
    pub(crate) fn github_auth(&self) -> Option<SecretString> {
        if let Some(credentials) = &self.git_credentials {
            return Some(credentials.get());
        }

        if let Some(token) = self.env.github_tokens.first() {
            return Some(token.secret.clone());
        }

        None
    }

    /// Grab an octokit client optionally configured with a token.
    pub(crate) fn octokit(&self) -> Result<octokit::Client> {
        let auth = match (&self.git_credentials, self.env.github_tokens.first()) {
            (Some(auth_manager), _) => octokit::Auth::Basic(auth_manager.get()),
            (_, Some(token)) => octokit::Auth::Bearer(token.secret.clone()),
            _ => octokit::Auth::None,
        };

        octokit::Client::new(auth)
    }

    /// Convert a context into an outcome.
    pub(crate) fn outcome(&self) -> ExitCode {
        for repo in self.repos() {
            if matches!(repo.state(), State::Error) {
                return ExitCode::FAILURE;
            }
        }

        ExitCode::SUCCESS
    }

    /// Get a repo path that is used as the base to other paths.
    pub(crate) fn to_path(&self, path: impl AsRef<RelativePath>) -> PathBuf {
        self.paths.to_path(path)
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
    pub(crate) fn repos(&self) -> Repos<'a> {
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
        self.changes.borrow_mut().push(ChangeWrapper {
            change,
            written: false,
        });
    }

    /// Get a list of warnings.
    pub(crate) fn warnings(&self) -> Ref<'_, [Warning]> {
        Ref::map(self.warnings.borrow(), Vec::as_slice)
    }

    /// Get a list of proposed changes.
    pub(crate) fn changes(&self) -> Ref<'_, [ChangeWrapper]> {
        Ref::map(self.changes.borrow(), Vec::as_slice)
    }

    /// Get a list of proposed changes.
    pub(crate) fn changes_mut(&self) -> RefMut<'_, [ChangeWrapper]> {
        RefMut::map(self.changes.borrow_mut(), Vec::as_mut_slice)
    }

    /// Check if there's changes to save.
    pub(crate) fn can_save(&self) -> bool {
        self.changes.borrow().iter().any(|c| !c.written)
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

/// Iterator over repositories.
pub(crate) struct Repos<'a> {
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use relative_path::RelativePath;

    use super::{Paths, Redirect};

    fn redirect() -> Redirect<'static> {
        Redirect {
            repo: RelativePath::new("repos/track"),
            worktree: RelativePath::new("repos/track/.claude/worktrees/wt"),
        }
    }

    fn paths(current: Option<&'static str>, redirect: Option<Redirect<'static>>) -> Paths<'static> {
        Paths {
            root: Path::new("/src"),
            current: current.map(RelativePath::new),
            config: None,
            cache: None,
            redirect,
        }
    }

    #[test]
    fn redirect_rewrite() {
        let rewrite = |p: &str| {
            redirect()
                .rewrite(RelativePath::new(p))
                .map(|p| p.to_string())
        };

        assert_eq!(
            rewrite("repos/track").as_deref(),
            Some("repos/track/.claude/worktrees/wt")
        );
        assert_eq!(
            rewrite("repos/track/src/main.rs").as_deref(),
            Some("repos/track/.claude/worktrees/wt/src/main.rs")
        );
        // Already under the worktree.
        assert_eq!(rewrite("repos/track/.claude/worktrees/wt/src"), None);
        // Not under the repo, including a repo sharing a name prefix.
        assert_eq!(rewrite("repos/kick"), None);
        assert_eq!(rewrite("repos/tracker"), None);
        assert_eq!(rewrite(""), None);
    }

    #[test]
    fn to_path_with_redirect() {
        let p = paths(None, Some(redirect()));
        assert_eq!(
            p.to_path("repos/track/Cargo.toml"),
            PathBuf::from("/src/repos/track/.claude/worktrees/wt/Cargo.toml")
        );
        assert_eq!(p.to_path("repos/kick"), PathBuf::from("/src/repos/kick"));

        let p = paths(
            Some("repos/track/.claude/worktrees/wt/src"),
            Some(redirect()),
        );
        assert_eq!(p.to_path("repos/track"), PathBuf::from(".."));
        assert_eq!(p.to_path("repos/track/src"), PathBuf::from("."));
        assert_eq!(
            p.to_path("repos/kick"),
            PathBuf::from("../../../../../kick")
        );

        // Without a redirect the main checkout is used.
        let p = paths(Some("repos/track/.claude/worktrees/wt"), None);
        assert_eq!(p.to_path("repos/track"), PathBuf::from("../../.."));
    }
}
