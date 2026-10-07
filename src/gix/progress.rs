//! Progress reporting for fetches of GitHub actions.
//!
//! Every concurrent fetch reports to one shared [`Fetches`]. A fetch stays
//! silent until gix learns that the pack it receives holds objects, which is
//! when it starts indexing them. Fetches which find everything in the cache
//! receive no pack, or an empty one, and print nothing.
//!
//! When stderr is a terminal, visible fetches are drawn as live lines by
//! prodash's line renderer, with gix's object and byte counters and their
//! throughput, and a finished fetch leaves one line behind. Otherwise a single
//! plain line is printed per fetch as it becomes visible.
//!
//! Each fetch reports to a private progress tree, and the renderer draws a
//! [`View`] combining the trees of the visible fetches, so that whatever a
//! fetch reported before it became visible shows up with it.

use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Instant;

use gix::odb::pack::bundle::write::ProgressId as BundleProgressId;
use gix::odb::pack::index::write::ProgressId as IndexProgressId;
use gix::progress::prodash::progress::Step;
use gix::progress::{AtomicStep, Id, MessageLevel, StepShared, Unit};
use gix::{Count, NestedProgress, Progress};
use prodash::messages::{Message, MessageCopyState};
use prodash::progress::{Key, Task};
use prodash::render::line;
use prodash::tree;

/// Frames per second of the line renderer.
const FRAMES_PER_SECOND: f32 = 10.0;

/// The shared progress of every concurrent fetch.
///
/// Dropping it stops the renderer, after which the terminal is free for other
/// output.
pub(crate) struct Fetches {
    shared: Arc<Shared>,
    next_id: AtomicU16,
}

impl Fetches {
    /// Construct progress for fetches, rendered to stderr.
    pub(crate) fn new() -> Self {
        let render = io::stderr().is_terminal().then(|| Render {
            visible: Mutex::new(Vec::new()),
            messages: tree::root::Options::default().into(),
            handle: Mutex::new(None),
        });

        Self {
            shared: Arc::new(Shared { render }),
            next_id: AtomicU16::new(0),
        }
    }

    /// Construct the progress of a fetch labelled `label`.
    pub(crate) fn fetch(&self, label: String) -> Fetch {
        let state = Arc::new(State {
            shared: self.shared.clone(),
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            label,
            tree: tree::root::Options::default().into(),
            visible: AtomicBool::new(false),
            started: OnceLock::new(),
            bytes: OnceLock::new(),
            objects: OnceLock::new(),
        });

        Fetch {
            state,
            logger: Logger::new(),
            item: None,
        }
    }
}

impl Drop for Fetches {
    fn drop(&mut self) {
        let Some(render) = &self.shared.render else {
            return;
        };

        if let Some(handle) = lock(&render.handle).take() {
            handle.shutdown_and_wait();
        }
    }
}

/// State shared by every fetch and the renderer.
struct Shared {
    /// Present if stderr is a terminal.
    render: Option<Render>,
}

struct Render {
    /// The trees of visible fetches, by fetch id.
    visible: Mutex<Vec<(u16, Arc<tree::Root>)>>,
    /// Holds the messages left behind by finished fetches.
    messages: Arc<tree::Root>,
    /// The line renderer, started when the first fetch becomes visible, so
    /// that nothing is written otherwise.
    handle: Mutex<Option<line::JoinHandle>>,
}

/// The state of one fetch, shared by every node of its progress.
struct State {
    shared: Arc<Shared>,
    id: u16,
    label: String,
    /// The private progress tree of the fetch.
    tree: Arc<tree::Root>,
    visible: AtomicBool,
    /// When the fetch started receiving a pack.
    started: OnceLock<Instant>,
    /// Bytes read of the pack.
    bytes: OnceLock<StepShared>,
    /// Objects indexed from the pack.
    objects: OnceLock<StepShared>,
}

impl State {
    /// Pick out counters to summarize the fetch with, and decide whether it
    /// becomes visible.
    fn track(&self, id: Id, item: &tree::Item) {
        if id == Id::from(BundleProgressId::ReadPackBytes) {
            _ = self.bytes.set(Count::counter(item));
        } else if id == Id::from(IndexProgressId::IndexObjects) {
            _ = self.objects.set(Count::counter(item));
        }
    }

    /// Called as a node identified by `id` is initialized.
    fn init(&self, id: Id, max: Option<Step>) {
        // The object count of the pack is known once its header is read.
        if id == Id::from(IndexProgressId::IndexObjects) && max.is_some_and(|max| max > 0) {
            self.show();
        }
    }

    fn show(&self) {
        if self.visible.swap(true, Ordering::AcqRel) {
            return;
        }

        let Some(render) = &self.shared.render else {
            _ = writeln!(io::stderr(), "Fetching {}", self.label);
            return;
        };

        lock(&render.visible).push((self.id, self.tree.clone()));

        let mut handle = lock(&render.handle);

        if handle.is_none() {
            let options = line::Options {
                frames_per_second: FRAMES_PER_SECOND,
                throughput: true,
                ..line::Options::default()
            }
            .auto_configure(line::StreamKind::Stderr);

            *handle = Some(line::render(
                io::stderr(),
                View(Arc::downgrade(&self.shared)),
                options,
            ));
        }
    }

    /// Hide the fetch, leaving `message` behind on a terminal.
    fn hide(&self, message: Option<String>) {
        if !self.visible.load(Ordering::Acquire) {
            return;
        }

        let Some(render) = &self.shared.render else {
            return;
        };

        lock(&render.visible).retain(|(id, _)| *id != self.id);

        if let Some(message) = message {
            // The renderer right-aligns the names of messages to the longest
            // one, so leave the name empty and put everything in the message.
            render.messages.add_child("").done(message);
        }
    }
}

/// The progress of one fetch, handed to gix.
///
/// The progress gix reports on this value itself (handshake and negotiation)
/// is only traced. Its children make up the progress of the fetch.
pub(crate) struct Fetch {
    state: Arc<State>,
    logger: Logger,
    /// The node of the fetch in its tree, created with its first child.
    item: Option<tree::Item>,
}

impl Fetch {
    /// Mark the fetch as finished, leaving a line behind on a terminal if it
    /// was visible.
    pub(crate) fn finish(self, ok: bool) {
        let state = &self.state;

        let message = ok.then(|| {
            let load = |slot: &OnceLock<StepShared>| slot.get().map(|s| s.load(Ordering::Relaxed));

            let summary = Summary {
                objects: load(&state.objects),
                bytes: load(&state.bytes),
                seconds: state
                    .started
                    .get()
                    .map_or(0.0, |started| started.elapsed().as_secs_f32()),
            };

            format!("Fetched {} ({summary})", state.label)
        });

        state.hide(message);
    }

    fn child(&mut self, name: String, id: Option<Id>) -> Child {
        let state = &self.state;

        let item = self.item.get_or_insert_with(|| {
            _ = state.started.set(Instant::now());
            state.tree.add_child(format!("Fetching {}", state.label))
        });

        Child::new(item, state, name, id)
    }
}

impl Progress for Fetch {
    fn init(&mut self, max: Option<Step>, unit: Option<Unit>) {
        self.logger.init(max, unit);
    }

    fn set_name(&mut self, name: String) {
        self.logger.set_name(name);
    }

    fn name(&self) -> Option<String> {
        self.logger.name()
    }

    fn id(&self) -> Id {
        self.logger.id()
    }

    fn message(&self, level: MessageLevel, message: String) {
        self.logger.message(level, message);
    }
}

impl NestedProgress for Fetch {
    type SubProgress = Child;

    fn add_child(&mut self, name: impl Into<String>) -> Self::SubProgress {
        self.child(name.into(), None)
    }

    fn add_child_with_id(&mut self, name: impl Into<String>, id: Id) -> Self::SubProgress {
        self.child(name.into(), Some(id))
    }
}

impl Count for Fetch {
    fn set(&self, step: Step) {
        self.logger.set(step);
    }

    fn step(&self) -> Step {
        self.logger.step()
    }

    fn inc_by(&self, step: Step) {
        self.logger.inc_by(step);
    }

    fn counter(&self) -> StepShared {
        self.logger.counter()
    }
}

/// A node in the progress tree of a fetch.
///
/// Its messages are only traced, so that a fetch leaves at most one line
/// behind.
pub(crate) struct Child {
    item: tree::Item,
    state: Arc<State>,
}

impl Child {
    fn new(parent: &mut tree::Item, state: &Arc<State>, name: String, id: Option<Id>) -> Self {
        let item = match id {
            Some(id) => {
                let item = parent.add_child_with_id(name, id);
                state.track(id, &item);
                item
            }
            None => parent.add_child(name),
        };

        Self {
            item,
            state: state.clone(),
        }
    }
}

impl Progress for Child {
    fn init(&mut self, max: Option<Step>, unit: Option<Unit>) {
        Progress::init(&mut self.item, max, unit);
        self.state.init(Progress::id(&self.item), max);
    }

    fn unit(&self) -> Option<Unit> {
        Progress::unit(&self.item)
    }

    fn max(&self) -> Option<Step> {
        Progress::max(&self.item)
    }

    fn set_max(&mut self, max: Option<Step>) -> Option<Step> {
        Progress::set_max(&mut self.item, max)
    }

    fn set_name(&mut self, name: String) {
        Progress::set_name(&mut self.item, name);
    }

    fn name(&self) -> Option<String> {
        Progress::name(&self.item)
    }

    fn id(&self) -> Id {
        Progress::id(&self.item)
    }

    fn message(&self, level: MessageLevel, message: String) {
        let name = Progress::name(&self.item).unwrap_or_default();
        tracing::trace!("{level:?}: {name}: {message}");
    }
}

impl NestedProgress for Child {
    type SubProgress = Child;

    fn add_child(&mut self, name: impl Into<String>) -> Self::SubProgress {
        Child::new(&mut self.item, &self.state, name.into(), None)
    }

    fn add_child_with_id(&mut self, name: impl Into<String>, id: Id) -> Self::SubProgress {
        Child::new(&mut self.item, &self.state, name.into(), Some(id))
    }
}

impl Count for Child {
    fn set(&self, step: Step) {
        Count::set(&self.item, step);
    }

    fn step(&self) -> Step {
        Count::step(&self.item)
    }

    fn inc_by(&self, step: Step) {
        Count::inc_by(&self.item, step);
    }

    fn counter(&self) -> StepShared {
        Count::counter(&self.item)
    }
}

/// What the line renderer draws: the trees of visible fetches, and the
/// messages of finished ones.
///
/// The top level of each tree holds the fetch alone, which is re-keyed by the
/// id of the fetch so that the trees do not collide.
struct View(Weak<Shared>);

#[derive(Clone)]
struct ViewRoot(Arc<Shared>);

impl ViewRoot {
    fn render(&self) -> &Render {
        self.0.render.as_ref().expect("only rendered to a terminal")
    }
}

impl prodash::WeakRoot for View {
    type Root = ViewRoot;

    fn upgrade(&self) -> Option<Self::Root> {
        self.0.upgrade().map(ViewRoot)
    }
}

impl prodash::Root for ViewRoot {
    type WeakRoot = View;

    fn messages_capacity(&self) -> usize {
        self.render().messages.messages_capacity()
    }

    fn num_tasks(&self) -> usize {
        lock(&self.render().visible)
            .iter()
            .map(|(_, tree)| tree.num_tasks())
            .sum()
    }

    fn sorted_snapshot(&self, out: &mut Vec<(Key, Task)>) {
        out.clear();

        let mut tasks = Vec::new();

        for (id, tree) in lock(&self.render().visible).iter() {
            tree.sorted_snapshot(&mut tasks);

            for (key, task) in tasks.drain(..) {
                out.push((rekey(key, *id), task));
            }
        }

        out.sort_by_key(|(key, _)| *key);
    }

    fn copy_messages(&self, out: &mut Vec<Message>) {
        self.render().messages.copy_messages(out);
    }

    fn copy_new_messages(
        &self,
        out: &mut Vec<Message>,
        prev: Option<MessageCopyState>,
    ) -> MessageCopyState {
        self.render().messages.copy_new_messages(out, prev)
    }

    fn downgrade(&self) -> Self::WeakRoot {
        View(Arc::downgrade(&self.0))
    }
}

/// Replace the top level of `key` with `id`.
fn rekey(key: Key, id: u16) -> Key {
    let mut out = Key::default().add_child(id);

    for level in 2..=key.level() {
        out = out.add_child(key[level]);
    }

    out
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The summary of a finished fetch.
struct Summary {
    objects: Option<usize>,
    bytes: Option<usize>,
    seconds: f32,
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(objects) = self.objects {
            write!(f, "{objects} objects, ")?;
        }

        if let Some(bytes) = self.bytes {
            write!(f, "{}, ", Bytes(bytes))?;
        }

        write!(f, "{:.1}s", self.seconds)
    }
}

/// Format a byte count with a binary prefix.
struct Bytes(usize);

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];

        if self.0 < 1024 {
            return write!(f, "{} B", self.0);
        }

        let mut value = self.0 as f64 / 1024.0;
        let mut unit = UNITS[0];

        for next in &UNITS[1..] {
            if value < 1024.0 {
                break;
            }

            value /= 1024.0;
            unit = next;
        }

        write!(f, "{value:.1} {unit}")
    }
}

/// Progress which only traces its messages.
struct Logger {
    name: Option<String>,
    counter: Arc<AtomicStep>,
    step: AtomicUsize,
    max: Option<usize>,
    unit: Option<Unit>,
}

impl Logger {
    fn new() -> Self {
        Self {
            name: None,
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

#[cfg(test)]
mod tests {
    use prodash::progress::Key;

    use super::{Bytes, rekey};

    #[test]
    fn format_bytes() {
        assert_eq!(Bytes(0).to_string(), "0 B");
        assert_eq!(Bytes(1023).to_string(), "1023 B");
        assert_eq!(Bytes(1024).to_string(), "1.0 KiB");
        assert_eq!(Bytes(1536 * 1024).to_string(), "1.5 MiB");
        assert_eq!(Bytes(3 * 1024 * 1024 * 1024).to_string(), "3.0 GiB");
    }

    #[test]
    fn rekey_replaces_top_level() {
        let key = Key::default().add_child(0).add_child(3).add_child(1);
        let expected = Key::default().add_child(7).add_child(3).add_child(1);
        assert_eq!(rekey(key, 7), expected);
        assert_eq!(rekey(Key::default().add_child(0), 7).level(), 1);
    }
}
