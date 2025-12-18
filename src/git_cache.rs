use anyhow::{Context, Result};
use relative_path::RelativePath;

use crate::Repo;
use crate::ctxt::Paths;
use crate::system::System;

/// Git cache for repository states.
pub(crate) struct GitCache<'repo, 'a> {
    repos: &'repo [Repo],
    paths: Paths<'a>,
    system: &'a System,
    fetch: bool,
    dirty_init: bool,
    dirty: Vec<&'repo RelativePath>,
    cached_init: bool,
    cached: Vec<&'repo RelativePath>,
    outdated_init: bool,
    outdated: Vec<&'repo RelativePath>,
    unreleased_init: bool,
    unreleased: Vec<&'repo RelativePath>,
}

impl<'repo, 'a> GitCache<'repo, 'a> {
    pub(crate) fn new(
        repos: &'repo [Repo],
        paths: Paths<'a>,
        system: &'a System,
        fetch: bool,
    ) -> Self {
        Self {
            repos,
            paths,
            system,
            fetch,
            dirty_init: false,
            dirty: Vec::new(),
            cached_init: false,
            cached: Vec::new(),
            outdated_init: false,
            outdated: Vec::new(),
            unreleased_init: false,
            unreleased: Vec::new(),
        }
    }

    pub(crate) fn dirty_set(&mut self) -> Result<&[&'repo RelativePath]> {
        if !self.dirty_init {
            let git = self
                .system
                .git
                .first()
                .context("no working git command found")?;

            for repo in self.repos {
                let path = self.paths.to_path(repo.path());

                if git.is_dirty(&path)? {
                    self.dirty.push(repo.path());
                }
            }

            self.dirty_init = true;
        }

        Ok(&self.dirty)
    }

    pub(crate) fn cached_set(&mut self) -> Result<&[&'repo RelativePath]> {
        if !self.cached_init {
            let git = self
                .system
                .git
                .first()
                .context("no working git command found")?;

            for repo in self.repos {
                let path = self.paths.to_path(repo.path());

                if git.is_cached(&path)? {
                    self.cached.push(repo.path());
                }
            }

            self.cached_init = true;
        }

        Ok(&self.cached)
    }

    pub(crate) fn outdated_set(&mut self) -> Result<&[&'repo RelativePath]> {
        if !self.outdated_init {
            let git = self
                .system
                .git
                .first()
                .context("no working git command found")?;

            for repo in self.repos {
                let path = self.paths.to_path(repo.path());

                if git.is_outdated(&path, self.fetch)? {
                    self.outdated.push(repo.path());
                }
            }

            self.outdated_init = true;
        }

        Ok(&self.outdated)
    }

    pub(crate) fn unreleased_set(&mut self) -> Result<&[&'repo RelativePath]> {
        if !self.unreleased_init {
            let git = self
                .system
                .git
                .first()
                .context("no working git command found")?;

            for repo in self.repos {
                let path = self.paths.to_path(repo.path());

                let outcome = 'outcome: {
                    let Some(describe) = git.describe_tags(&path, self.fetch)? else {
                        tracing::trace!("No tags to describe");
                        break 'outcome true;
                    };

                    if describe.offset.is_none() {
                        tracing::trace!("No offset detected (tag: {})", describe.tag);
                        break 'outcome true;
                    }

                    false
                };

                if outcome {
                    self.unreleased.push(repo.path());
                }
            }

            self.unreleased_init = true;
        }

        Ok(&self.unreleased)
    }
}
