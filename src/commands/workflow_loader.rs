use std::collections::HashSet;
use std::str;

use anyhow::{anyhow, Context, Result};

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::workflows::WorkflowManifests;

use super::{Actions, LoadedWorkflows, Prepare};

/// A system of commands to be run.
pub(crate) struct WorkflowLoader<'a, 'cx> {
    cx: &'a Ctxt<'cx>,
    matrix_ignore: HashSet<String>,
}

impl<'a, 'cx> WorkflowLoader<'a, 'cx> {
    /// Create a new command system.
    pub(crate) fn new(cx: &'a Ctxt<'cx>) -> Self {
        Self {
            cx,
            matrix_ignore: HashSet::new(),
        }
    }

    /// Insert a matrix variable to ignore.
    pub(crate) fn ignore_matrix_variable<S>(&mut self, variable: S)
    where
        S: AsRef<str>,
    {
        self.matrix_ignore.insert(variable.as_ref().to_owned());
    }

    /// Load workflows from a repository.
    pub(crate) fn load_github_workflows(
        &self,
        repo: &Repo,
        prepare: &mut Prepare,
    ) -> Result<LoadedWorkflows<'a, 'cx>> {
        let mut workflows = Vec::new();
        let wfs = WorkflowManifests::new(self.cx, repo)?;

        for workflow in wfs.workflows() {
            let workflow = workflow?;

            let mut jobs = Vec::new();

            for job in workflow.jobs(&self.matrix_ignore)? {
                for (_, steps) in &job.matrices {
                    for step in &steps.steps {
                        if let Some(name) = &step.uses {
                            let actions = prepare.actions.get_or_insert_with(Actions::default);

                            actions.insert_action(name).with_context(|| {
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

        Ok(LoadedWorkflows::new(self.cx, workflows))
    }
}
