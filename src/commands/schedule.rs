use std::rc::Rc;

use anyhow::Result;

use crate::rstr::RStr;
use crate::workflows::Step;

use super::{
    ActionConfig, ActionRunner, BatchConfig, Env, ScheduleBasicCommand, ScheduleNodeAction,
    ScheduleRun, ScheduleStaticSetup, ScheduleUse, Session,
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
    job_id: &str,
    action_name: Option<&RStr>,
    batch: &BatchConfig<'_, '_>,
    steps: &[Rc<Step>],
    runner: Option<&ActionRunner>,
    c: Option<&ActionConfig>,
) -> Result<Vec<Schedule>> {
    let env = Env::new(batch, runner, c)?;

    let mut commands = Vec::new();

    if !steps.is_empty() {
        commands.push(Schedule::Push);

        for (index, step) in steps.iter().enumerate() {
            let mut env = env.clone();

            if !step.tree.is_empty() {
                let tree = env.tree.with_extended(&step.tree);
                env = env.with_tree(Rc::new(tree));
            }

            if let Some(run) = &step.run {
                commands.push(Schedule::Run(ScheduleRun::new(
                    format!("{}-{}", job_id, index).into(),
                    action_name.map(Box::from),
                    Box::from(run.as_str()),
                    step.clone(),
                    env.clone(),
                )));
            }

            if let Some(uses) = &step.uses {
                commands.push(Schedule::Use(ScheduleUse::new(
                    uses.clone(),
                    step.clone(),
                    env.clone(),
                )));
            }
        }

        commands.push(Schedule::Pop);
    }

    Ok(commands)
}