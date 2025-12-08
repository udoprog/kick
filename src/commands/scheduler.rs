use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::rc::Rc;

use anyhow::{Context, Result, bail};
use termcolor::WriteColor;

use crate::config::Os;
use crate::rstr::{RStr, RString};
use crate::workflows::{Eval, Tree};

use super::{BatchConfig, Run, Schedule, ScheduleOutputs, Session};

struct StackEntry {
    name: Option<Rc<RStr>>,
    tree: Tree,
    id: Option<Rc<RStr>>,
    main: VecDeque<Schedule>,
    pre: VecDeque<Schedule>,
    post: VecDeque<Schedule>,
    outputs: Option<ScheduleOutputs>,
}

pub(super) struct Scheduler {
    /// Main queue of schedules to seed the scheduler.
    queue: VecDeque<Schedule>,
    /// Stack of tasks being executed.
    stack: Vec<StackEntry>,
    /// Current environment variables set.
    env: BTreeMap<String, String>,
    /// Current paths configured.
    paths: Vec<OsString>,
}

impl Scheduler {
    pub(super) fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            stack: Vec::new(),
            env: BTreeMap::new(),
            paths: Vec::new(),
        }
    }

    /// Get a complete id of the thing being scheduled.
    pub(super) fn id(&self, separator: &str, tail: Option<&RStr>) -> Option<RString> {
        let mut it = self.stack.iter().flat_map(|e| e.id.as_deref()).chain(tail);

        let first = it.next()?;

        let mut o = RString::with_capacity(first.len());
        o.push_rstr(first);

        for step in it {
            o.push_rstr(separator);
            o.push_rstr(step);
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
        self.queue.push_back(schedule);
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

    fn next_schedule(&mut self) -> Result<Option<Schedule>> {
        loop {
            let Some(e) = self.stack.last_mut() else {
                return Ok(self.queue.pop_front());
            };

            if let Some(item) = e.pre.pop_front() {
                return Ok(Some(item));
            }

            if let Some(item) = e.main.pop_front() {
                return Ok(Some(item));
            }

            if let Some(item) = e.post.pop_front() {
                // Defer running post action.
                self.queue.push_back(item);
                continue;
            };

            let e = self.stack.pop().context("Missing stack entry")?;

            if let Some(o) = e.outputs {
                let raw_env = BTreeMap::new();
                let env = o.env.extend_with(&e.tree, &raw_env)?;

                let id = e.id.context("Missing id to store outputs")?;
                let id = id.to_exposed();
                let eval = Eval::new(&env.tree);

                let mut values = BTreeMap::new();

                for (key, value) in o.outputs.as_ref() {
                    values.insert(key.clone(), eval.eval(&value)?.into_owned());
                }

                let tree = self.tree_mut().context("Missing scheduler tree")?;
                tree.insert_prefix(["steps", id.as_ref(), "outputs"], values);
            }
        }
    }

    pub(super) async fn advance<O>(
        &mut self,
        o: &mut O,
        batch: &BatchConfig<'_, '_>,
        session: &mut Session,
        os: &Os,
    ) -> Result<Option<Run>>
    where
        O: ?Sized + WriteColor,
    {
        while let Some(schedule) = self.next_schedule()? {
            schedule.prepare(session)?;

            // This will take care to synchronize any actions which are needed
            // to advance the scheduler.
            let remediations = session.prepare(batch, Eval::empty()).await?;

            if !remediations.is_empty() {
                if !batch.fix {
                    remediations.print(o, batch)?;
                    bail!("Failed to prepare commands, use `--fix` to try and fix the system");
                }

                remediations.apply(o, batch).await?;
            }

            match schedule {
                Schedule::Group(g) => {
                    self.stack.push(StackEntry {
                        name: g.name,
                        tree: Tree::new(),
                        id: g.id,
                        main: g.steps.iter().cloned().collect(),
                        pre: VecDeque::new(),
                        post: VecDeque::new(),
                        outputs: g.outputs,
                    });
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

                    let e = self
                        .stack
                        .last_mut()
                        .context("Missing stack entry to incorporate use")?;

                    for run in group.pre {
                        e.pre.push_back(run);
                    }

                    for run in group.main.into_iter().rev() {
                        e.main.push_front(run);
                    }

                    for run in group.post.into_iter().rev() {
                        e.post.push_front(run);
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
