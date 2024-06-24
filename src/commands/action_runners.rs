use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::rc::Rc;
use std::str;

use anyhow::{bail, Result};

use crate::action::ActionKind;
use crate::rstr::RStr;

use super::{build_steps, ActionConfig, BatchConfig, Env, Schedule, ScheduleNodeAction};

#[derive(Debug)]
pub(super) struct ActionRunner {
    id: Box<str>,
    kind: ActionKind,
    defaults: BTreeMap<String, String>,
    action_path: Rc<Path>,
    repo_dir: Rc<Path>,
}

impl ActionRunner {
    pub(super) fn new(
        id: Box<str>,
        kind: ActionKind,
        defaults: BTreeMap<String, String>,
        action_path: Rc<Path>,
        repo_dir: Rc<Path>,
    ) -> Self {
        Self {
            id,
            kind,
            defaults,
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
        uses: &RStr,
    ) -> Result<(Vec<Schedule>, Vec<Schedule>)> {
        let exposed = uses.to_exposed();

        let Some(action) = self.runners.get(exposed.as_ref()) else {
            bail!("Could not find action runner for {uses}");
        };

        let mut main = Vec::new();
        let mut post = Vec::new();

        match &action.kind {
            ActionKind::Node {
                main_path,
                post_path,
                node_version,
            } => {
                let id = c.id().map(RStr::as_rc);
                let env = Env::new(batch, Some(action), Some(c))?;

                if let Some(path) = post_path {
                    post.push(Schedule::Push);
                    post.push(Schedule::NodeAction(ScheduleNodeAction::new(
                        id.clone(),
                        uses.as_rc(),
                        path.clone(),
                        *node_version,
                        c.skipped(),
                        env.clone(),
                    )));
                    post.push(Schedule::Pop);
                }

                main.push(Schedule::Push);
                main.push(Schedule::NodeAction(ScheduleNodeAction::new(
                    id.clone(),
                    uses.as_rc(),
                    main_path.clone(),
                    *node_version,
                    c.skipped(),
                    env,
                )));
                main.push(Schedule::Pop);
            }
            ActionKind::Composite { steps } => {
                let commands = build_steps(
                    action.id(),
                    c.action_name(),
                    batch,
                    steps,
                    Some(action),
                    Some(c),
                )?;
                main.extend(commands);
            }
        }

        Ok((main, post))
    }
}