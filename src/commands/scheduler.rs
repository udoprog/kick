use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::rc::Rc;

use anyhow::{bail, Context, Result};
use termcolor::WriteColor;

use crate::config::Os;
use crate::rstr::{RStr, RString};
use crate::workflows::{Eval, Tree};

use super::{BatchConfig, Run, Schedule, Session};

struct StackEntry {
    name: Option<Rc<RStr>>,
    tree: Tree,
    id: Option<Rc<str>>,
}

pub(super) struct Scheduler {
    stack: Vec<StackEntry>,
    env: BTreeMap<String, String>,
    main: VecDeque<Schedule>,
    pre: VecDeque<Schedule>,
    post: VecDeque<Schedule>,
    paths: Vec<OsString>,
}

impl Scheduler {
    pub(super) fn new() -> Self {
        Self {
            stack: Vec::new(),
            env: BTreeMap::new(),
            main: VecDeque::new(),
            pre: VecDeque::new(),
            post: VecDeque::new(),
            paths: Vec::new(),
        }
    }

    /// Get a complete id of the thing being scheduled.
    pub(super) fn id(&self, separator: &str, tail: Option<&str>) -> Option<String> {
        let mut it = self.stack.iter().flat_map(|e| e.id.as_deref()).chain(tail);

        let first = it.next()?;

        let mut o = String::with_capacity(first.len());
        o.push_str(first);

        for step in it {
            o.push_str(separator);
            o.push_str(step);
        }

        Some(o)
    }

    /// Get the current name of the thing being scheduled.
    pub(super) fn name(&self, separator: &str, tail: Option<&RStr>) -> Option<RString> {
        let mut it = self
            .stack
            .iter()
            .flat_map(|e| e.name.as_deref())
            .chain(tail);

        let first = it.next()?;

        let mut o = RString::with_capacity(first.len());

        o.push_rstr(first);

        for step in it {
            o.push_rstr(separator);
            o.push_rstr(step);
        }

        Some(o)
    }

    /// Push back a schedule onto the main queue.
    pub(super) fn push_back(&mut self, schedule: Schedule) {
        self.main.push_back(schedule);
    }

    pub(super) fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    pub(super) fn paths(&self) -> &[OsString] {
        &self.paths
    }

    pub(super) fn tree(&self) -> &Tree {
        self.stack.last().map(|e| &e.tree).unwrap_or_default()
    }

    pub(super) fn env_mut(&mut self) -> &mut BTreeMap<String, String> {
        &mut self.env
    }

    pub(super) fn paths_mut(&mut self) -> &mut Vec<OsString> {
        &mut self.paths
    }

    pub(super) fn tree_mut(&mut self) -> Option<&mut Tree> {
        Some(&mut self.stack.last_mut()?.tree)
    }

    fn next_schedule(&mut self) -> Option<Schedule> {
        if let Some(item) = self.pre.pop_front() {
            return Some(item);
        }

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
        session: &mut Session,
        os: &Os,
    ) -> Result<Option<Run>>
    where
        O: ?Sized + WriteColor,
    {
        while let Some(schedule) = self.next_schedule() {
            schedule.prepare(session)?;

            // This will take care to synchronize any actions which are needed
            // to advance the scheduler.
            let remediations = session.prepare(batch, Eval::empty())?;

            if !remediations.is_empty() {
                if !batch.fix {
                    remediations.print(o, batch)?;
                    bail!("Failed to prepare commands, use `--fix` to try and fix the system");
                }

                remediations.apply(o, batch)?;
            }

            match schedule {
                Schedule::Push {
                    name,
                    id: parent_step_id,
                } => {
                    self.stack.push(StackEntry {
                        name,
                        tree: Tree::new(),
                        id: parent_step_id,
                    });
                }
                Schedule::Pop => {
                    self.stack.pop();
                }
                Schedule::Outputs(outputs) => {
                    let Some(StackEntry { tree, id, .. }) = self.stack.pop() else {
                        bail!("Missing tree for outputs");
                    };

                    let raw_env = BTreeMap::new();
                    let env = outputs.env.extend_with(&tree, &raw_env)?;

                    let id = id.context("Missing id to store outputs")?;
                    let eval = Eval::new(&env.tree);

                    let mut values = BTreeMap::new();

                    for (key, value) in outputs.outputs.as_ref() {
                        values.insert(key.clone(), eval.eval(&value)?.into_owned());
                    }

                    let tree = self.tree_mut().context("Missing scheduler tree")?;
                    tree.insert_prefix(["steps", id.as_ref(), "outputs"], values);
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
                    let group = u.build(batch, self.tree(), session.runners(), os)?;

                    for run in group.pre {
                        self.pre.push_back(run);
                    }

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

    /// Insert new outputs with an associated id.
    pub(super) fn insert_new_outputs<'a>(
        &mut self,
        id: &str,
        values: impl IntoIterator<Item = &'a (String, String)>,
    ) -> Result<()> {
        let tree = self.tree_mut().context("Missing scheduler tree")?;
        tree.insert_prefix(["steps", id, "outputs"], values.into_iter().cloned());
        Ok(())
    }
}
