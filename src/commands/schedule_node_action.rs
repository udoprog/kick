use std::path::Path;
use std::rc::Rc;

use anyhow::Result;

use crate::workflows::Eval;

use super::{Env, Run};

#[derive(Clone)]
pub(crate) struct ScheduleNodeAction {
    path: Rc<Path>,
    node_version: u32,
    skipped: Option<String>,
    env: Env,
    condition: Option<String>,
}

impl ScheduleNodeAction {
    pub(crate) fn new(
        path: Rc<Path>,
        node_version: u32,
        skipped: Option<&str>,
        env: Env,
        condition: Option<String>,
    ) -> Self {
        Self {
            path,
            node_version,
            skipped: skipped.map(str::to_owned),
            env,
            condition,
        }
    }

    pub(super) fn build(self) -> Result<Run> {
        let skipped = 'skipped: {
            let Some(condition) = self.condition else {
                break 'skipped None;
            };

            let eval = Eval::new(&self.env.tree);

            if !eval.test(&condition)? {
                Some(condition)
            } else {
                None
            }
        };

        let run = Run::node(self.node_version, self.path)
            .with_skipped(self.skipped.or(skipped))
            .with_env(self.env.build_os_env());

        Ok(self.env.decorate(run))
    }
}
