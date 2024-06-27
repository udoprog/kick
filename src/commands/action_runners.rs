use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::rc::Rc;
use std::str;

use anyhow::{bail, Result};

use crate::action::ActionKind;
use crate::rstr::rformat;

use super::{
    build_steps, ActionConfig, BatchConfig, Env, Schedule, ScheduleGroup, ScheduleNodeAction,
    ScheduleOutputs,
};

#[derive(Debug)]
pub(super) struct ActionRunner {
    kind: ActionKind,
    defaults: BTreeMap<String, String>,
    outputs: BTreeMap<String, String>,
    action_path: Rc<Path>,
    repo_dir: Rc<Path>,
}

impl ActionRunner {
    pub(super) fn new(
        kind: ActionKind,
        defaults: BTreeMap<String, String>,
        outputs: BTreeMap<String, String>,
        action_path: Rc<Path>,
        repo_dir: Rc<Path>,
    ) -> Self {
        Self {
            kind,
            defaults,
            outputs,
            action_path,
            repo_dir,
        }
    }

    /// Get the state directory associated with the action.
    pub(super) fn repo_dir(&self) -> &Path {
        &self.repo_dir
    }

    /// Get the action path.
    pub(super) fn action_path(&self) -> &Path {
        &self.action_path
    }

    /// Get default input variables.
    pub(super) fn defaults(&self) -> impl Iterator<Item = (&str, &str)> {
        self.defaults.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Get output variables.
    pub(super) fn outputs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.outputs.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

#[derive(Default, Debug)]
pub(crate) struct ActionRunners {
    runners: HashMap<String, ActionRunner>,
}

impl ActionRunners {
    /// Test if we contain the given runner.
    pub(super) fn contains(&self, key: &str) -> bool {
        self.runners.contains_key(key)
    }

    /// Insert an action runner.
    pub(super) fn insert(&mut self, key: String, runner: ActionRunner) {
        self.runners.insert(key, runner);
    }

    /// Build the run configurations of an action.
    pub(super) fn build(
        &self,
        batch: &BatchConfig<'_, '_>,
        c: &ActionConfig<'_>,
    ) -> Result<RunnerSteps> {
        let exposed = c.action_name().to_exposed();

        let Some(action) = self.runners.get(exposed.as_ref()) else {
            bail!("Could not find action runner for {}", c.action_name());
        };

        let main;
        let mut pre = None;
        let mut post = None;

        match &action.kind {
            ActionKind::Node {
                main: main_path,
                pre: pre_path,
                pre_if,
                post: post_path,
                post_if,
                node_version,
            } => {
                let env = Env::new(batch, Some(action), Some(c))?;

                if let Some(path) = pre_path {
                    pre = Some(Schedule::Group(ScheduleGroup::new(
                        Some(rformat!("{} (post)", c.action_name()).as_rc()),
                        c.id().cloned(),
                        Rc::from([Schedule::NodeAction(ScheduleNodeAction::new(
                            path.clone(),
                            *node_version,
                            c.skipped(),
                            env.clone(),
                            pre_if.clone(),
                        ))]),
                    )));
                }

                if let Some(path) = post_path {
                    post = Some(Schedule::Group(ScheduleGroup::new(
                        Some(rformat!("{} (post)", c.action_name()).as_rc()),
                        c.id().cloned(),
                        Rc::from([Schedule::NodeAction(ScheduleNodeAction::new(
                            path.clone(),
                            *node_version,
                            c.skipped(),
                            env.clone(),
                            post_if.clone(),
                        ))]),
                    )));
                }

                let outputs = action
                    .outputs()
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    .collect::<BTreeMap<_, _>>();

                let mut group = ScheduleGroup::new(
                    Some(c.action_name().as_rc()),
                    c.id().cloned(),
                    Rc::from([Schedule::NodeAction(ScheduleNodeAction::new(
                        main_path.clone(),
                        *node_version,
                        c.skipped(),
                        env.clone(),
                        None,
                    ))]),
                );

                if !outputs.is_empty() {
                    group = group.with_outputs(ScheduleOutputs {
                        env,
                        outputs: Rc::new(outputs),
                    });
                }

                main = Schedule::Group(group);
            }
            ActionKind::Composite { steps } => {
                main = build_steps(
                    batch,
                    Some(c),
                    c.id(),
                    Some(c.action_name()),
                    steps,
                    Some(action),
                )?;
            }
        }

        Ok(RunnerSteps { main, pre, post })
    }
}

pub(super) struct RunnerSteps {
    pub(super) main: Schedule,
    pub(super) pre: Option<Schedule>,
    pub(super) post: Option<Schedule>,
}
