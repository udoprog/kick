use std::rc::Rc;

use anyhow::Result;

use crate::rstr::RStr;
use crate::workflows::Step;

use super::{
    ActionConfig, ActionRunner, BatchConfig, Env, ScheduleBasicCommand, ScheduleNodeAction,
    ScheduleOutputs, ScheduleRun, ScheduleStaticSetup, ScheduleUse, Session,
};

#[derive(Clone)]
pub(super) struct ScheduleGroup {
    pub(super) name: Option<Rc<RStr>>,
    pub(super) id: Option<Rc<RStr>>,
    pub(super) steps: Box<[Schedule]>,
    pub(super) outputs: Option<ScheduleOutputs>,
}

impl ScheduleGroup {
    /// Construct a schedule group.
    pub(super) fn new(
        name: Option<Rc<RStr>>,
        id: Option<Rc<RStr>>,
        steps: Box<[Schedule]>,
    ) -> Self {
        Self {
            name,
            id,
            steps,
            outputs: None,
        }
    }

    /// Modify outputs of the group.
    pub(super) fn with_outputs(mut self, outputs: ScheduleOutputs) -> Self {
        self.outputs = Some(outputs);
        self
    }
}

/// A scheduled action.
#[derive(Clone)]
pub(super) enum Schedule {
    Group(ScheduleGroup),
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
    batch: &BatchConfig<'_, '_>,
    c: Option<&ActionConfig<'_>>,
    id: Option<&Rc<RStr>>,
    name: Option<&RStr>,
    steps: &[Rc<Step>],
    runner: Option<&ActionRunner>,
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

    Ok(Schedule::Group(ScheduleGroup::new(
        name.map(RStr::as_rc),
        id.cloned(),
        group.into(),
    )))
}
