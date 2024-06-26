use std::rc::Rc;

use anyhow::Result;

use crate::rstr::RStr;
use crate::workflows::Step;

use super::{
    ActionConfig, ActionRunner, BatchConfig, Env, ScheduleBasicCommand, ScheduleNodeAction,
    ScheduleOutputs, ScheduleRun, ScheduleStaticSetup, ScheduleUse, Session,
};

/// A scheduled action.
#[derive(Clone)]
pub(super) enum Schedule {
    Group {
        name: Option<Rc<RStr>>,
        id: Option<Rc<str>>,
        steps: Box<[Schedule]>,
        outputs: Option<ScheduleOutputs>,
    },
    BasicCommand(ScheduleBasicCommand),
    StaticSetup(ScheduleStaticSetup),
    NodeAction(ScheduleNodeAction),
    Run(ScheduleRun),
    Use(ScheduleUse),
}

impl Schedule {
    /// Add a preparation which matches the given schedule.
    pub(super) fn prepare(&self, session: &mut Session) -> Result<()> {
        match self {
            Schedule::Use(u) => {
                session.actions_mut().insert_action(u.uses())?;
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Add jobs from a workflows, matrix, and associated steps.
pub(super) fn build_steps(
    id: Option<&Rc<str>>,
    name: Option<&RStr>,
    batch: &BatchConfig<'_, '_>,
    steps: &[Rc<Step>],
    runner: Option<&ActionRunner>,
    c: Option<&ActionConfig>,
) -> Result<Schedule> {
    let env = Env::new(batch, runner, c)?;

    let mut group = Vec::new();

    if !steps.is_empty() {
        for step in steps {
            let mut env = env.clone();

            if !step.tree.is_empty() {
                let tree = env.tree.with_extended(&step.tree);
                env = env.with_tree(Rc::new(tree));
            }

            if let Some(run) = &step.run {
                group.push(Schedule::Run(ScheduleRun::new(
                    Box::from(run.as_str()),
                    step.clone(),
                    env.clone(),
                )));
            }

            if let Some(uses) = &step.uses {
                group.push(Schedule::Use(ScheduleUse::new(
                    uses.clone(),
                    step.clone(),
                    env.clone(),
                )));
            }
        }
    }

    Ok(Schedule::Group {
        name: name.map(RStr::as_rc),
        id: id.cloned(),
        steps: group.into(),
        outputs: None,
    })
}
