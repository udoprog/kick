use std::collections::{BTreeMap, VecDeque};

use anyhow::{bail, Result};
use termcolor::WriteColor;

use crate::workflows::Tree;

use super::{BatchConfig, Prepare, Run, Schedule};

pub(super) struct Scheduler {
    stack: Vec<Tree>,
    env: BTreeMap<String, String>,
    main: VecDeque<Schedule>,
    post: VecDeque<Schedule>,
}

impl Scheduler {
    pub(super) fn new() -> Self {
        Self {
            stack: Vec::new(),
            env: BTreeMap::new(),
            main: VecDeque::new(),
            post: VecDeque::new(),
        }
    }

    /// Push back a schedule onto the main queue.
    pub(super) fn push_back(&mut self, schedule: Schedule) {
        self.main.push_back(schedule);
    }

    pub(super) fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    pub(super) fn tree(&self) -> Option<&Tree> {
        self.stack.last()
    }

    pub(super) fn env_mut(&mut self) -> &mut BTreeMap<String, String> {
        &mut self.env
    }

    pub(super) fn tree_mut(&mut self) -> Option<&mut Tree> {
        self.stack.last_mut()
    }

    fn next_schedule(&mut self) -> Option<Schedule> {
        if let Some(item) = self.main.pop_front() {
            return Some(item);
        }

        if let Some(item) = self.post.pop_front() {
            return Some(item);
        };

        None
    }

    pub(super) fn advance<O>(
        &mut self,
        o: &mut O,
        batch: &BatchConfig<'_, '_>,
        prepare: &mut Prepare,
    ) -> Result<Option<Run>>
    where
        O: ?Sized + WriteColor,
    {
        while let Some(schedule) = self.next_schedule() {
            schedule.prepare(prepare)?;

            // This will take care to synchronize any actions which are needed
            // to advance the scheduler.
            let remediations = prepare.prepare(batch)?;

            if !remediations.is_empty() {
                if !batch.fix {
                    remediations.print(o, batch)?;
                    bail!("Failed to prepare commands, use `--fix` to try and fix the system");
                }

                remediations.apply(o, batch)?;
            }

            match schedule {
                Schedule::Push => {
                    self.stack.push(Tree::new());
                }
                Schedule::Pop => {
                    self.stack.pop();
                }
                Schedule::BasicCommand(command) => {
                    let run = command.build();
                    return Ok(Some(run));
                }
                Schedule::StaticSetup(setup) => {
                    let run = setup.build();
                    return Ok(Some(run));
                }
                Schedule::NodeAction(node) => {
                    let run = node.build()?;
                    return Ok(Some(run));
                }
                Schedule::Run(run) => {
                    let run = run.build(self.tree())?;
                    return Ok(Some(run));
                }
                Schedule::Use(u) => {
                    let group = u.build(batch, self.tree(), prepare.runners())?;

                    for run in group.main.into_iter().rev() {
                        self.main.push_front(run);
                    }

                    for run in group.post.into_iter().rev() {
                        self.post.push_front(run);
                    }
                }
            }
        }

        Ok(None)
    }
}
