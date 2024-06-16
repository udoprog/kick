use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use gix::progress::prodash::progress::Step;
use gix::progress::{AtomicStep, Id, MessageLevel, StepShared, Unit};
use gix::{Count, NestedProgress, Progress};

pub(super) struct Logger {
    name: Option<String>,
    #[allow(unused)]
    id: Option<Id>,
    counter: Arc<AtomicStep>,
    step: AtomicUsize,
    max: Option<usize>,
    unit: Option<Unit>,
}

impl Logger {
    pub(super) fn new() -> Self {
        Self {
            name: None,
            id: None,
            counter: Arc::new(AtomicStep::new(0)),
            step: AtomicUsize::new(0),
            max: None,
            unit: None,
        }
    }
}

impl Progress for Logger {
    fn init(&mut self, max: Option<Step>, unit: Option<Unit>) {
        self.max = max;
        self.unit = unit;
    }

    fn set_name(&mut self, name: String) {
        self.name = Some(name);
    }

    fn name(&self) -> Option<String> {
        self.name.clone()
    }

    fn id(&self) -> Id {
        *b"LOGG"
    }

    fn message(&self, level: MessageLevel, message: String) {
        tracing::trace!("{level:?}: {message}")
    }
}

impl NestedProgress for Logger {
    type SubProgress = Logger;

    fn add_child(&mut self, name: impl Into<String>) -> Self::SubProgress {
        Logger {
            name: Some(name.into()),
            id: None,
            counter: self.counter.clone(),
            step: AtomicUsize::new(0),
            max: None,
            unit: None,
        }
    }

    fn add_child_with_id(&mut self, name: impl Into<String>, id: Id) -> Self::SubProgress {
        Logger {
            name: Some(name.into()),
            id: Some(id),
            counter: self.counter.clone(),
            step: AtomicUsize::new(0),
            max: None,
            unit: None,
        }
    }
}

impl Count for Logger {
    fn set(&self, step: Step) {
        self.step.store(step, Ordering::SeqCst);
    }

    fn step(&self) -> Step {
        self.step.load(Ordering::SeqCst)
    }

    fn inc_by(&self, step: Step) {
        self.step.fetch_add(step, Ordering::SeqCst);
    }

    fn counter(&self) -> StepShared {
        self.counter.clone()
    }
}
