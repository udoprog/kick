use std::rc::Rc;

use anyhow::Result;

use crate::ctxt::Ctxt;
use crate::workflows::Step;

use super::new_env;
use super::{
    ActionConfig, ActionRunner, ScheduleBasicCommand, ScheduleNodeAction, ScheduleRun,
    ScheduleStaticSetup, ScheduleUse,
};

/// A scheduled action.
#[derive(Clone)]
pub(super) enum Schedule {
    Push,
    Pop,
    BasicCommand(ScheduleBasicCommand),
    StaticSetup(ScheduleStaticSetup),
    NodeAction(ScheduleNodeAction),
    Run(ScheduleRun),
    Use(ScheduleUse),
}

/// Add jobs from a workflows, matrix, and associated steps.
pub(super) fn build_steps(
    cx: &Ctxt<'_>,
    steps: &[Step],
    runner: Option<&ActionRunner>,
    c: Option<&ActionConfig>,
) -> Result<Vec<Schedule>> {
    let (env, tree) = new_env(cx, runner, c)?;

    let mut commands = Vec::new();

    for step in steps {
        let mut tree = tree.clone();
        tree.extend(&step.tree);
        let tree = Rc::new(tree);

        if let Some(run) = &step.run {
            commands.push(Schedule::Run(ScheduleRun::new(
                run.clone(),
                step.clone(),
                tree.clone(),
                env.clone(),
            )));
        }

        if let Some(uses) = &step.uses {
            commands.push(Schedule::Use(ScheduleUse::new(
                uses.clone(),
                step.clone(),
                tree.clone(),
                env.clone(),
            )));
        }
    }

    Ok(commands)
}
