use std::collections::{BTreeMap, VecDeque};

use anyhow::Result;

use crate::workflows::Tree;

use super::{ActionRunners, BatchConfig, Run, Schedule};

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

    pub(super) fn push(&mut self) {
        self.stack.push(Tree::new());
    }

    pub(super) fn pop(&mut self) {
        self.stack.pop();
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

    pub(super) fn advance(
        &mut self,
        batch: &BatchConfig<'_, '_>,
        runners: Option<&ActionRunners>,
    ) -> Result<Option<Run>> {
        loop {
            let command = 'next: {
                if let Some(item) = self.main.pop_front() {
                    break 'next item;
                }

                if let Some(item) = self.post.pop_front() {
                    break 'next item;
                };

                return Ok(None);
            };

            match command {
                Schedule::Push => {
                    self.push();
                }
                Schedule::Pop => {
                    self.pop();
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
                    let group = u.build(batch, self.tree(), runners)?;

                    for run in group.main.into_iter().rev() {
                        self.main.push_front(run);
                    }

                    for run in group.post.into_iter().rev() {
                        self.post.push_front(run);
                    }
                }
            }
        }
    }
}
