use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::rc::Rc;
use std::str;

use anyhow::{bail, Result};

use crate::action::ActionKind;
use crate::rstr::{rformat, RStr};

use super::schedule_outputs::ScheduleOutputs;
use super::{build_steps, ActionConfig, BatchConfig, Env, Schedule, ScheduleNodeAction};

#[derive(Debug)]
pub(super) struct ActionRunner {
    id: Box<str>,
    kind: ActionKind,
    defaults: BTreeMap<String, String>,
    outputs: BTreeMap<String, String>,
    action_path: Rc<Path>,
    repo_dir: Rc<Path>,
}

impl ActionRunner {
    pub(super) fn new(
        id: Box<str>,
        kind: ActionKind,
        defaults: BTreeMap<String, String>,
        outputs: BTreeMap<String, String>,
        action_path: Rc<Path>,
        repo_dir: Rc<Path>,
    ) -> Self {
        Self {
            id,
            kind,
            defaults,
            outputs,
            action_path,
            repo_dir,
        }
    }

    /// Get the identifier of the action.
    pub(super) fn id(&self) -> &str {
        &self.id
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

        let mut main = Vec::new();
        let mut pre = Vec::new();
        let mut post = Vec::new();

        match &action.kind {
            ActionKind::Node {
                main: main_path,
                pre: pre_path,
                pre_if,
                post: post_path,
                post_if,
                node_version,
            } => {
                let id = c.id().map(RStr::as_rc);
                let env = Env::new(batch, Some(action), Some(c))?;

                if let Some(path) = pre_path {
                    pre.push(Schedule::Push {
                        name: Some(rformat!("{} (pre)", c.action_name()).as_rc()),
                        id: id.clone(),
                    });
                    pre.push(Schedule::NodeAction(ScheduleNodeAction::new(
                        id.clone(),
                        path.clone(),
                        *node_version,
                        c.skipped(),
                        env.clone(),
                        pre_if.clone(),
                    )));
                    pre.push(Schedule::Pop);
                }

                if let Some(path) = post_path {
                    post.push(Schedule::Push {
                        name: Some(rformat!("{} (post)", c.action_name()).as_rc()),
                        id: id.clone(),
                    });
                    post.push(Schedule::NodeAction(ScheduleNodeAction::new(
                        id.clone(),
                        path.clone(),
                        *node_version,
                        c.skipped(),
                        env.clone(),
                        post_if.clone(),
                    )));
                    post.push(Schedule::Pop);
                }

                main.push(Schedule::Push {
                    name: Some(c.action_name().as_rc()),
                    id: id.clone(),
                });
                main.push(Schedule::NodeAction(ScheduleNodeAction::new(
                    id.clone(),
                    main_path.clone(),
                    *node_version,
                    c.skipped(),
                    env.clone(),
                    None,
                )));

                let outputs = action
                    .outputs()
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    .collect::<BTreeMap<_, _>>();

                if outputs.is_empty() {
                    post.push(Schedule::Pop);
                } else {
                    main.push(Schedule::Outputs(ScheduleOutputs {
                        env,
                        outputs: Rc::new(outputs),
                    }));
                }
            }
            ActionKind::Composite { steps } => {
                let commands = build_steps(
                    action.id(),
                    c.id(),
                    Some(c.action_name()),
                    batch,
                    steps,
                    Some(action),
                    Some(c),
                )?;
                main.extend(commands);
            }
        }

        Ok(RunnerSteps { main, pre, post })
    }
}

pub(super) struct RunnerSteps {
    pub(super) main: Vec<Schedule>,
    pub(super) pre: Vec<Schedule>,
    pub(super) post: Vec<Schedule>,
}
