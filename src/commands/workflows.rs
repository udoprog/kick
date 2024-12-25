use std::rc::Rc;
use std::str;

use anyhow::{bail, Result};

use crate::config::{Distribution, Os};
use crate::rstr::RStr;
use crate::workflows::{Job, Matrix, Steps, WorkflowManifest};

use super::{build_steps, Batch, BatchConfig, RunOn};

/// A collection of loaded workflows.
pub(crate) struct LoadedWorkflow<'a, 'cx> {
    workflows: &'a LoadedWorkflows<'a, 'cx>,
    manifest: &'a WorkflowManifest<'a, 'cx>,
    jobs: &'a [Job],
}

impl<'cx> LoadedWorkflow<'_, 'cx> {
    /// Get the identifier of the workflow.
    pub(crate) fn id(&self) -> &str {
        self.manifest.id()
    }

    /// Iterate over all jobs in the current workflow.
    pub(crate) fn jobs(&self) -> impl Iterator<Item = LoadedJob<'_, 'cx>> + '_ {
        self.jobs.iter().map(|job| LoadedJob {
            workflows: self.workflows,
            job,
        })
    }
}

/// A single loaded job.
pub(crate) struct LoadedJob<'a, 'cx> {
    workflows: &'a LoadedWorkflows<'a, 'cx>,
    job: &'a Job,
}

impl<'cx> LoadedJob<'_, 'cx> {
    /// Get the identifier of the job.
    pub(crate) fn id(&self) -> &str {
        &self.job.id
    }

    /// Iterate over all matrices of the current job.
    pub(crate) fn matrices(&self) -> impl Iterator<Item = LoadedJobMatrix<'_, 'cx>> + '_ {
        self.job
            .matrices
            .iter()
            .map(|(matrix, steps)| LoadedJobMatrix {
                workflows: self.workflows,
                matrix,
                steps,
            })
    }
}

/// A single loaded job.
pub(crate) struct LoadedJobMatrix<'a, 'cx> {
    workflows: &'a LoadedWorkflows<'a, 'cx>,
    matrix: &'a Matrix,
    steps: &'a Steps,
}

impl LoadedJobMatrix<'_, '_> {
    /// Get the expanded name of a job.
    pub(crate) fn name(&self) -> Option<&RStr> {
        self.steps.name.as_deref()
    }

    /// Get the matrix associated with the loaded job.
    pub(crate) fn matrix(&self) -> &Matrix {
        self.matrix
    }

    /// Build a batch from the current job matrix.
    pub(crate) fn build(
        &self,
        parent_step_id: Option<&Rc<RStr>>,
        same_os: bool,
        current_os: &Os,
    ) -> Result<Batch> {
        self.workflows
            .build_batch(self.matrix, self.steps, parent_step_id, same_os, current_os)
    }
}

/// Loaded workflows.
pub(crate) struct LoadedWorkflows<'a, 'cx> {
    batch: &'a BatchConfig<'a, 'cx>,
    workflows: Vec<(WorkflowManifest<'a, 'cx>, Vec<Job>)>,
}

impl<'a, 'cx> LoadedWorkflows<'a, 'cx> {
    /// Construct a new collection of loaded workflows.
    pub(super) fn new(
        batch: &'a BatchConfig<'a, 'cx>,
        workflows: Vec<(WorkflowManifest<'a, 'cx>, Vec<Job>)>,
    ) -> Self {
        Self { batch, workflows }
    }

    /// Iterate over workflows.
    pub(crate) fn iter(&self) -> impl Iterator<Item = LoadedWorkflow<'_, 'cx>> + '_ {
        self.workflows
            .iter()
            .map(|(manifest, jobs)| LoadedWorkflow {
                workflows: self,
                manifest,
                jobs,
            })
    }

    /// Add jobs from a workflows, matrix, and associated steps.
    pub(super) fn build_batch(
        &self,
        matrix: &Matrix,
        steps: &Steps,
        parent_step_id: Option<&Rc<RStr>>,
        same_os: bool,
        current_os: &Os,
    ) -> Result<Batch> {
        let runs_on = steps.runs_on.to_exposed();

        let (os, dist) = match runs_on.split_once('-').map(|(os, _)| os) {
            Some("ubuntu") => (Os::Linux, Distribution::Ubuntu),
            Some("windows") => (Os::Windows, Distribution::Other),
            Some("macos") => (Os::Mac, Distribution::Other),
            _ => bail!("Unsupported runs-on directive: {}", steps.runs_on),
        };

        let (run_on, os) = if same_os {
            (RunOn::Same, current_os.clone())
        } else {
            (RunOn::from_os(self.batch, &os, dist)?, os)
        };

        let commands = build_steps(self.batch, None, parent_step_id, None, &steps.steps, None)?;

        Ok(Batch::new(
            run_on,
            os,
            vec![commands],
            if !matrix.is_empty() {
                Some(matrix.clone())
            } else {
                None
            },
        ))
    }
}
