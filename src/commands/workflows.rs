use std::str;

use anyhow::{bail, Result};

use crate::config::{Distribution, Os};
use crate::ctxt::Ctxt;
use crate::rstr::RStr;
use crate::workflows::{Job, Matrix, Steps, WorkflowManifest};

use super::build_steps;
use super::{Batch, RunOn};

/// A collection of loaded workflows.
pub(crate) struct LoadedWorkflow<'a, 'cx> {
    workflows: &'a LoadedWorkflows<'a, 'cx>,
    manifest: &'a WorkflowManifest<'a, 'cx>,
    jobs: &'a [Job],
}

impl<'a, 'cx> LoadedWorkflow<'a, 'cx> {
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

impl<'a, 'cx> LoadedJob<'a, 'cx> {
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

impl<'a, 'cx> LoadedJobMatrix<'a, 'cx> {
    /// Get the expanded name of a job.
    pub(crate) fn name(&self) -> Option<&RStr> {
        self.steps.name.as_deref()
    }

    /// Get the matrix associated with the loaded job.
    pub(crate) fn matrix(&self) -> &Matrix {
        self.matrix
    }

    /// Build a batch from the current job matrix.
    pub(crate) fn build(&self, same_os: bool) -> Result<Batch> {
        self.workflows.build_batch(self.matrix, self.steps, same_os)
    }
}

/// Loaded workflows.
pub(crate) struct LoadedWorkflows<'a, 'cx> {
    cx: &'a Ctxt<'cx>,
    workflows: Vec<(WorkflowManifest<'a, 'cx>, Vec<Job>)>,
}

impl<'a, 'cx> LoadedWorkflows<'a, 'cx> {
    /// Construct a new collection of loaded workflows.
    pub(super) fn new(
        cx: &'a Ctxt<'cx>,
        workflows: Vec<(WorkflowManifest<'a, 'cx>, Vec<Job>)>,
    ) -> Self {
        Self { cx, workflows }
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
        same_os: bool,
    ) -> Result<Batch> {
        let runs_on = steps.runs_on.to_exposed();

        let (os, dist) = match runs_on.split_once('-').map(|(os, _)| os) {
            Some("ubuntu") => (Os::Linux, Distribution::Ubuntu),
            Some("windows") => (Os::Windows, Distribution::Other),
            Some("macos") => (Os::Mac, Distribution::Other),
            _ => bail!("Unsupported runs-on directive: {}", steps.runs_on),
        };

        let run_on = if same_os {
            RunOn::Same
        } else {
            RunOn::from_os(self.cx, &os, dist)?
        };

        let commands = build_steps(self.cx, &steps.steps, None, None)?;

        Ok(Batch::new(
            commands,
            run_on,
            if !matrix.is_empty() {
                Some(matrix.clone())
            } else {
                None
            },
        ))
    }
}
