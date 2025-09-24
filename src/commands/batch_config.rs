use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::str;

use anyhow::{Result, bail};

use crate::config::{Distribution, Os};
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::rstr::{RStr, RString};
use crate::shell::Shell;
use crate::workflows::WorkflowManifests;

use super::{Colors, LoadedWorkflows, RunOn};

const GITHUB_SERVER: &str = "https://github.com";

/// A batch runner configuration.
pub(crate) struct BatchConfig<'a, 'cx> {
    pub(super) cx: &'a Ctxt<'cx>,
    pub(super) process_id: u32,
    pub(super) path: PathBuf,
    pub(super) shell: Shell,
    pub(super) colors: Colors,
    pub(super) env: BTreeMap<String, String>,
    pub(super) env_passthrough: BTreeSet<String>,
    pub(super) run_on: Vec<(RunOn, Os)>,
    pub(super) verbose: u8,
    pub(super) dry_run: bool,
    pub(super) exposed: bool,
    pub(super) matrix_ignore: HashSet<String>,
    pub(super) matrix_filter: Vec<(String, String)>,
    pub(super) fix: bool,
    pub(super) keep: bool,
    pub(super) github_server: Option<String>,
    pub(super) github_token: Option<Box<RStr>>,
}

impl<'a, 'cx> BatchConfig<'a, 'cx> {
    /// Construct a new batch configuration.
    pub(crate) fn new(cx: &'a Ctxt<'cx>, path: PathBuf, shell: Shell) -> Self {
        Self {
            cx,
            path,
            shell,
            process_id: std::process::id(),
            colors: Colors::new(),
            env: BTreeMap::new(),
            env_passthrough: BTreeSet::new(),
            run_on: Vec::new(),
            verbose: 0,
            dry_run: false,
            exposed: false,
            matrix_ignore: HashSet::new(),
            matrix_filter: Vec::new(),
            fix: false,
            keep: false,
            github_server: None,
            github_token: cx
                .github_auth()
                .and_then(|t| RString::redacted(t.as_secret()))
                .map(Box::<RStr>::from),
        }
    }

    /// Get the configured github server.
    pub(crate) fn github_server(&self) -> &str {
        self.github_server.as_deref().unwrap_or(GITHUB_SERVER)
    }

    /// A github token, if available.
    pub(crate) fn github_token(&self) -> Option<&RStr> {
        self.github_token.as_deref()
    }

    /// Set an environment variable.
    pub(crate) fn set_env(&mut self, key: impl AsRef<str>, env: impl AsRef<str>) {
        self.env
            .insert(key.as_ref().to_owned(), env.as_ref().to_owned());
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
        let run_on = RunOn::from_os(self, os, Distribution::Ubuntu)?;
        self.run_on.push((run_on, os.clone()));
        Ok(())
    }

    /// Add a run on.
    pub(crate) fn add_run_on(&mut self, run_on: RunOn, os: Os) -> Result<()> {
        if let RunOn::Wsl(..) = run_on
            && self.cx.system.wsl.is_empty()
        {
            bail!("WSL is not available");
        }

        self.run_on.push((run_on, os));
        Ok(())
    }

    /// Load workflows from a repository.
    pub(crate) fn load_github_workflows(&self, repo: &Repo) -> Result<LoadedWorkflows<'_, 'cx>> {
        let mut workflows = Vec::new();
        let wfs = WorkflowManifests::new(self.cx, repo)?;

        for workflow in wfs.workflows() {
            let workflow = workflow?;

            let mut jobs = Vec::new();

            for job in workflow.jobs(&self.matrix_ignore, &self.matrix_filter)? {
                jobs.push(job);
            }

            workflows.push((workflow, jobs));
        }

        Ok(LoadedWorkflows::new(self, workflows))
    }
}
