use std::path::Path;
use std::rc::Rc;

use anyhow::Result;

use crate::rstr::RStr;

use super::{Env, Run};

#[derive(Clone)]
pub(crate) struct ScheduleNodeAction {
    id: Option<Rc<RStr>>,
    uses: Rc<RStr>,
    path: Rc<Path>,
    node_version: u32,
    skipped: Option<String>,
    env: Env,
}

impl ScheduleNodeAction {
    pub(crate) fn new(
        id: Option<Rc<RStr>>,
        uses: Rc<RStr>,
        path: Rc<Path>,
        node_version: u32,
        skipped: Option<&str>,
        env: Env,
    ) -> Self {
        Self {
            id,
            uses,
            path,
            node_version,
            skipped: skipped.map(str::to_owned),
            env,
        }
    }

    pub(super) fn build(self) -> Result<Run> {
        let (env, _) = self.env.build(None)?;

        let run = Run::node(self.node_version, self.path)
            .with_id(self.id.map(|id| id.as_ref().to_owned()))
            .with_name(Some(self.uses))
            .with_skipped(self.skipped)
            .with_env(env);

        Ok(self.env.decorate(run))
    }
}
