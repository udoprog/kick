use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::str;

use anyhow::{bail, Result};

use crate::config::{Distribution, Os};
use crate::ctxt::Ctxt;
use crate::shell::Shell;

use super::{BatchOptions, Colors, RunOn};

/// A batch runner configuration.
pub(crate) struct BatchConfig<'a, 'cx> {
    pub(super) cx: &'a Ctxt<'cx>,
    pub(super) repo_path: &'a Path,
    pub(super) shell: Shell,
    pub(super) colors: Colors,
    pub(super) env: BTreeMap<String, String>,
    pub(super) env_passthrough: BTreeSet<String>,
    pub(super) run_on: Vec<RunOn>,
    pub(super) verbose: u8,
    pub(super) dry_run: bool,
    pub(super) exposed: bool,
}

impl<'a, 'cx> BatchConfig<'a, 'cx> {
    /// Construct a new batch configuration.
    pub(crate) fn new(cx: &'a Ctxt<'cx>, repo_path: &'a Path, shell: Shell) -> Self {
        Self {
            cx,
            repo_path,
            shell,
            colors: Colors::new(),
            env: BTreeMap::new(),
            env_passthrough: BTreeSet::new(),
            run_on: Vec::new(),
            verbose: 0,
            dry_run: false,
            exposed: false,
        }
    }

    /// Add options from [`BatchOptions`].
    pub(crate) fn add_opts(&mut self, opts: &BatchOptions) -> Result<()> {
        for &run_on in &opts.run_on {
            self.add_run_on(run_on.to_run_on())?;
        }

        if opts.exposed {
            self.exposed = true;
        }

        self.verbose = opts.verbose;

        if opts.dry_run {
            self.dry_run = true;
        }

        for env in &opts.env {
            self.parse_env(env)?;
        }

        Ok(())
    }

    /// Parse an environment.
    pub(crate) fn parse_env(&mut self, env: &str) -> Result<()> {
        if let Some((key, value)) = env.split_once('=') {
            self.env.insert(key.to_owned(), value.to_owned());
        } else {
            self.env_passthrough.insert(env.to_owned());
        }

        Ok(())
    }

    /// Add an operating system.
    pub(crate) fn add_os(&mut self, os: &Os) -> Result<()> {
        self.run_on
            .push(RunOn::from_os(self.cx, os, Distribution::Ubuntu)?);
        Ok(())
    }

    /// Add a run on.
    pub(crate) fn add_run_on(&mut self, run_on: RunOn) -> Result<()> {
        if let RunOn::Wsl(..) = run_on {
            if self.cx.system.wsl.is_empty() {
                bail!("WSL is not available");
            }
        }

        self.run_on.push(run_on);
        Ok(())
    }
}
