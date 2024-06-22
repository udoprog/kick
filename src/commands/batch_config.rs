use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::str;

use anyhow::{anyhow, bail, Context, Result};

use crate::config::{Distribution, Os};
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::shell::Shell;
use crate::workflows::WorkflowManifests;

use super::{Colors, LoadedWorkflows, Prepare, RunOn};

/// A batch runner configuration.
pub(crate) struct BatchConfig<'a, 'cx> {
    pub(super) cx: &'a Ctxt<'cx>,
    pub(super) path: PathBuf,
    pub(super) shell: Shell,
    pub(super) colors: Colors,
    pub(super) env: BTreeMap<String, String>,
    pub(super) env_passthrough: BTreeSet<String>,
    pub(super) run_on: Vec<RunOn>,
    pub(super) verbose: u8,
    pub(super) dry_run: bool,
    pub(super) exposed: bool,
    pub(super) matrix_ignore: HashSet<String>,
}

impl<'a, 'cx> BatchConfig<'a, 'cx> {
    /// Construct a new batch configuration.
    pub(crate) fn new(cx: &'a Ctxt<'cx>, repo_path: PathBuf, shell: Shell) -> Self {
        Self {
            cx,
            path: repo_path,
            shell,
            colors: Colors::new(),
            env: BTreeMap::new(),
            env_passthrough: BTreeSet::new(),
            run_on: Vec::new(),
            verbose: 0,
            dry_run: false,
            exposed: false,
            matrix_ignore: HashSet::new(),
        }
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
        self.run_on.push(run_on);
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

    /// Load workflows from a repository.
    pub(crate) fn load_github_workflows(
        &self,
        repo: &Repo,
        prepare: &mut Prepare,
    ) -> Result<LoadedWorkflows<'_, 'cx>> {
        let mut workflows = Vec::new();
        let wfs = WorkflowManifests::new(self.cx, repo)?;

        for workflow in wfs.workflows() {
            let workflow = workflow?;

            let mut jobs = Vec::new();

            for job in workflow.jobs(&self.matrix_ignore)? {
                for (_, steps) in &job.matrices {
                    for step in &steps.steps {
                        if let Some(name) = &step.uses {
                            prepare.actions_mut().insert_action(name).with_context(|| {
                                anyhow!(
                                    "Uses statement in job `{}` and step `{}`",
                                    job.id,
                                    step.name()
                                )
                            })?;
                        }
                    }
                }

                jobs.push(job);
            }

            workflows.push((workflow, jobs));
        }

        Ok(LoadedWorkflows::new(self, workflows))
    }
}
