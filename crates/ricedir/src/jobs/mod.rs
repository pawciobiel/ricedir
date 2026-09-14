//! The job engine: what is going to happen, and then what happened.
//!
//! Nothing here touches `App`, and nothing in `App` touches a worker. A job
//! is built on one thread, run on another, and says what it did through a
//! channel that ends in `update` -- which stays the only writer. See
//! `## Decided: how a job tells the window what changed` in `TODO.md`.
//!
//! **A plan is built before a byte moves.** The whole list of steps, the
//! total size and every clash are worked out first, so the window can say
//! what a job will do rather than discovering it halfway through. It also
//! means cancelling early costs nothing: the plan is the only thing that has
//! happened yet.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use iced::Task;
use iced::futures::SinkExt;

use crate::entry::Kind;

/// How deep a walk goes before it gives up.
///
/// A symlink to a directory is copied as a link and never walked, so a loop
/// cannot be made that way. This is for the honest depth nobody meant --
/// `node_modules` inside `node_modules` -- and for a tree built to be deep.
const DEPTH: usize = 64;

/// How much is read and written at a time.
const CHUNK: usize = 256 * 1024;

/// How often a worker says how far it has got.
///
/// Thirty times a second, whatever the file count. One message per file
/// would make a copy of 200,000 small files slower than the copy.
const REPORT: std::time::Duration = std::time::Duration::from_millis(33);

/// How long a paused worker sleeps between looks at its control.
const NAP: std::time::Duration = std::time::Duration::from_millis(50);

/// Which job. Never reused, so a late message from a job that is gone is
/// dropped rather than landing on whatever took its place in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(pub u64);

/// What a job does.
///
/// One kind per job. Copy came first because it only ever creates, so a fault
/// in a new engine could not cost anybody a file. Trash follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Copy,
    /// Copy, then take the original away.
    ///
    /// On one filesystem this is a rename: one syscall for a whole tree,
    /// whatever it holds. Across two it cannot be, so it becomes a copy and
    /// a delete, and the plan says which it turned out to be.
    Move,
    /// Remove, for good. There is no trash yet and no undo, so a person is
    /// asked before a job of this kind is ever made -- see `Dialogue`.
    Delete,
}

impl Work {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::Move => "Move",
            Self::Delete => "Delete",
        }
    }
}

/// One step of a plan, in the order it has to happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub from: PathBuf,
    pub to: PathBuf,
    pub what: What,
    /// Something is already at `to`. The step does nothing until a person
    /// answers; see [`Plan::resolve`].
    pub clash: Option<Clash>,
    /// Take away what is at `to` first. Only [`Plan::resolve`] sets this.
    pub replace: bool,
}

impl Step {
    /// A step with nothing in the way.
    const fn plain(from: PathBuf, to: PathBuf, what: What) -> Self {
        Self {
            from,
            to,
            what,
            clash: None,
            replace: false,
        }
    }
}

/// What is already at a destination, measured against what would replace it.
///
/// Both answers are worked out during the walk, while the plan is on a
/// thread. Asking again at the moment a person picks "newer only" would mean
/// a stat per clash on the Elm loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clash {
    /// The source was changed more recently than what is there.
    pub newer: bool,
    /// The source holds more bytes than what is there.
    pub larger: bool,
}

/// What to do about every destination that is already there.
///
/// One answer for the whole job. Asking per file is how a copy of a thousand
/// entries becomes a thousand dialogues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolve {
    /// Leave what is there. The source is not copied.
    Skip,
    /// Replace what is there.
    Overwrite,
    /// Keep both, by giving the arrival a free name beside the other.
    KeepBoth,
    /// Replace only where the source was changed more recently.
    Newer,
    /// Replace only where the source is bigger.
    Larger,
}

impl Resolve {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Skip => "Skip",
            Self::Overwrite => "Overwrite",
            Self::KeepBoth => "Keep both",
            Self::Newer => "Newer only",
            Self::Larger => "Larger only",
        }
    }

    /// Every answer, in the order the dialogue offers them.
    pub const ALL: [Self; 5] = [
        Self::Skip,
        Self::Overwrite,
        Self::KeepBoth,
        Self::Newer,
        Self::Larger,
    ];
}

/// What a step makes at the other end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum What {
    /// A directory. Always listed before anything inside it, so the parent
    /// is there when its children arrive.
    Directory,
    /// A file, and how many bytes it held when the plan was built.
    File(u64),
    /// A symlink, and the text it points at.
    ///
    /// The link is copied, never followed. Following one is how a copy of
    /// `~/Downloads` ends up copying the whole disk through a link somebody
    /// left in it.
    Link(PathBuf),
    /// Take this away. A directory goes with everything inside it.
    ///
    /// One step per thing that was selected, not one per file, and the
    /// reason is safety rather than tidiness. A path-based walk that removed
    /// entries one at a time can be redirected: swap a directory half way
    /// down for a symlink between the plan and the delete, and `a/b/c`
    /// resolves somewhere else entirely. `std::fs::remove_dir_all` is
    /// written with `openat` and `unlinkat` against exactly that race, so
    /// a whole tree goes in one call rather than in steps of ours.
    ///
    /// The cost is that progress and cancel land between things, not inside
    /// one. `bytes` is what the walk counted, so the bar still means
    /// something. Per-entry progress waits for the `rustix` item in `M2`.
    Remove {
        /// Whether this is a directory, and so goes with its contents.
        tree: bool,
        /// What the walk found under it, for the bar.
        bytes: u64,
    },
    /// Move on one filesystem: one `rename` for a whole tree, whatever it
    /// holds. `bytes` is only what the walk counted, so the bar still means
    /// something; no byte is read or written.
    Rename(u64),
}

/// Everything a job will do, worked out before it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub work: Work,
    /// Where it all goes.
    pub into: PathBuf,
    pub steps: Vec<Step>,
    /// What the files add up to. Directories and links count as nothing,
    /// which is what makes a percentage mean anything on a real tree.
    pub bytes: u64,
    /// Destinations that are already there and have not been answered for.
    ///
    /// Empty once [`Plan::resolve`] has run. Nothing is guessed: a job with
    /// anything in this list waits in [`State::Asking`], because guessing is
    /// how a file manager eats somebody's work.
    pub clashes: Vec<PathBuf>,
    /// What the walk could not read, and why. The job still runs.
    pub skipped: Vec<(PathBuf, String)>,
}

impl Plan {
    /// How many steps will actually be taken.
    pub const fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether the plan would do nothing at all.
    pub const fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Apply one answer to every destination that is already there.
    ///
    /// The walk kept the clashing steps rather than dropping them, so this
    /// needs no second walk and no second stat. A step that is not kept goes,
    /// and only the kept ones add to `bytes` -- `plan()` leaves a clashing
    /// step out of the total, so "skip" is what the bar already assumed.
    pub fn resolve(&mut self, how: Resolve) {
        let mut taken: Vec<PathBuf> = Vec::new();

        self.steps.retain_mut(|step| {
            let Some(clash) = step.clash else {
                return true;
            };

            let keep = match how {
                Resolve::Skip => false,
                Resolve::Overwrite | Resolve::KeepBoth => true,
                Resolve::Newer => clash.newer,
                Resolve::Larger => clash.larger,
            };

            if !keep {
                return false;
            }

            step.clash = None;
            if how == Resolve::KeepBoth {
                step.to = beside(&step.to, &taken);
                taken.push(step.to.clone());
            } else {
                step.replace = true;
            }
            true
        });

        // Only now, because `beside` may have moved a step somewhere that
        // costs the same but the skipped ones cost nothing.
        self.bytes += self
            .steps
            .iter()
            .filter(|step| step.replace || taken.contains(&step.to))
            .map(|step| match step.what {
                What::File(bytes) | What::Rename(bytes) => bytes,
                _ => 0,
            })
            .sum::<u64>();

        self.clashes.clear();
    }

    /// One line saying what this will do, for the panel and the notice.
    pub fn describe(&self) -> String {
        if self.work == Work::Delete {
            let things = self.steps.len();
            return format!(
                "delete {things} thing{}",
                if things == 1 { "" } else { "s" }
            );
        }

        let files = self
            .steps
            .iter()
            .filter(|step| matches!(step.what, What::File(_)))
            .count();

        format!(
            "{} {files} file{} into {}",
            self.work.name().to_lowercase(),
            if files == 1 { "" } else { "s" },
            self.into.display()
        )
    }
}

/// What a job is doing now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Being worked out. No byte has moved and cancelling costs nothing.
    Planning,
    /// Planned, and waiting for a person to say what to do about the
    /// destinations that are already there. Nothing runs until they answer.
    Asking,
    /// Planned, waiting for a worker.
    Queued,
    Running,
    Paused,
    Done,
    /// Stopped part way. The path is the file left half written, if the
    /// stop landed inside one.
    Cancelled(Option<PathBuf>),
    /// Never started. The plan itself could not be built.
    Failed(String),
}

impl State {
    pub const fn running(&self) -> bool {
        matches!(self, Self::Running | Self::Paused)
    }

    /// Whether this job is finished with, one way or another.
    pub const fn over(&self) -> bool {
        matches!(self, Self::Done | Self::Cancelled(_) | Self::Failed(_))
    }
}

/// What a worker has been told to do.
///
/// The one genuinely shared thing in the engine. `update` cannot reach a
/// running thread and a thread must never reach `App`, so pause and cancel
/// are an atomic both sides can see -- the same shape `watch.rs` uses for a
/// channel that overflowed.
#[derive(Debug, Default)]
pub struct Control(AtomicU8);

const RUN: u8 = 0;
const PAUSE: u8 = 1;
const CANCEL: u8 = 2;

impl Control {
    pub fn pause(&self) {
        // A cancelled job stays cancelled. Pausing one would leave a worker
        // sleeping over a job nobody is going to finish.
        let _ = self
            .0
            .compare_exchange(RUN, PAUSE, Ordering::Relaxed, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        let _ = self
            .0
            .compare_exchange(PAUSE, RUN, Ordering::Relaxed, Ordering::Relaxed);
    }

    /// Asks the worker to stop. It stops between chunks, so a big file ends
    /// within a quarter of a megabyte rather than at the end of the file.
    pub fn cancel(&self) {
        self.0.store(CANCEL, Ordering::Relaxed);
    }

    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed) == CANCEL
    }

    /// Wait here while paused. `true` means carry on, `false` means stop.
    fn carry_on(&self) -> bool {
        loop {
            match self.0.load(Ordering::Relaxed) {
                CANCEL => return false,
                PAUSE => std::thread::sleep(NAP),
                _ => return true,
            }
        }
    }
}

/// One job, as the window knows it.
#[derive(Debug)]
pub struct Job {
    pub id: Id,
    pub work: Work,
    pub into: PathBuf,
    /// `None` while it is still being worked out.
    pub plan: Option<Plan>,
    pub state: State,
    /// Bytes written so far, against `plan.bytes`.
    pub done: u64,
    /// Steps taken so far, against `plan.len()`.
    pub steps: usize,
    /// What went wrong along the way. A job carries on past an error: one
    /// unreadable file must not abandon the other nine hundred.
    pub errors: Vec<(PathBuf, String)>,
    control: Arc<Control>,
}

impl Job {
    /// How far through, from 0 to 1.
    ///
    /// By bytes where there are any, and by steps otherwise -- a tree of
    /// empty files has no bytes to count and still takes time.
    pub fn fraction(&self) -> f32 {
        let Some(plan) = &self.plan else {
            return 0.0;
        };

        // A bar 180 pixels wide, not an accounting figure. The last few
        // bytes of an exabyte do not show.
        if plan.bytes > 0 {
            return (self.done as f32 / plan.bytes as f32).clamp(0.0, 1.0);
        }
        if plan.steps.is_empty() {
            return 1.0;
        }
        (self.steps as f32 / plan.steps.len() as f32).clamp(0.0, 1.0)
    }

    /// One short line for the panel.
    pub fn describe(&self) -> String {
        match &self.state {
            State::Planning => format!("{} \u{2014} working it out", self.work.name()),
            // The reason goes in the notice, which is as wide as the
            // window. The panel is 180 pixels and a path does not fit.
            State::Failed(_) => format!("{} \u{2014} refused", self.work.name()),
            State::Cancelled(_) => format!("{} \u{2014} stopped", self.work.name()),
            State::Done if self.errors.is_empty() => format!("{} \u{2014} done", self.work.name()),
            State::Done => format!(
                "{} \u{2014} done, {} failed",
                self.work.name(),
                self.errors.len()
            ),
            State::Paused => format!("{} \u{2014} held", self.work.name()),
            State::Queued => format!("{} \u{2014} waiting", self.work.name()),
            State::Asking => format!("{} \u{2014} asking", self.work.name()),
            State::Running => {
                let percent = (self.fraction() * 100.0) as u8;
                format!("{} {percent}%", self.work.name())
            }
        }
    }

    /// The longer line for the notice, when a job is over.
    ///
    /// The panel is 180 pixels wide and a path is not. This is where a half
    /// written file gets named, because pretending a cancelled copy can be
    /// undone is worse than saying which file to look at.
    pub fn ending(&self) -> String {
        use std::fmt::Write;

        let mut said = self.describe();

        if let State::Failed(why) = &self.state {
            let _ = write!(said, ": {why}");
        }

        if let State::Cancelled(Some(partial)) = &self.state {
            let _ = write!(
                said,
                ". {} is only part copied, and is still there",
                partial.display()
            );
        }

        // The first one, and how many more. A notice that listed nine
        // hundred failures would be a notice nobody reads.
        if let Some((path, why)) = self.errors.first() {
            let _ = write!(said, ". {}: {why}", path.display());
            if self.errors.len() > 1 {
                let _ = write!(said, " ({} more)", self.errors.len() - 1);
            }
        }

        said
    }
}

/// Every job this window knows about, and how many may run at once.
#[derive(Debug)]
pub struct Queue {
    jobs: Vec<Job>,
    /// Never reused. See [`Id`].
    next: u64,
    workers: usize,
}

impl Queue {
    pub const fn new(workers: usize) -> Self {
        Self {
            jobs: Vec::new(),
            next: 0,
            workers,
        }
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Job> {
        self.jobs.iter()
    }

    pub const fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn get(&self, id: Id) -> Option<&Job> {
        self.jobs.iter().find(|job| job.id == id)
    }

    fn find(&mut self, id: Id) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|job| job.id == id)
    }

    /// Take a job on, and start working out what it would do.
    ///
    /// The walk runs on a thread: a source of 100,000 entries would hold the
    /// window for as long as it took to count them.
    pub fn add(&mut self, work: Work, sources: Vec<PathBuf>, into: PathBuf) -> (Id, Task<Message>) {
        let id = Id(self.next);
        self.next += 1;

        self.jobs.push(Job {
            id,
            work,
            into: into.clone(),
            plan: None,
            state: State::Planning,
            done: 0,
            steps: 0,
            errors: Vec::new(),
            control: Arc::new(Control::default()),
        });

        (id, planning(id, work, sources, into))
    }

    /// A plan arrived. Put it on its job, or fail the job with why not.
    pub fn planned(&mut self, id: Id, made: Result<Plan, String>) {
        let Some(job) = self.find(id) else {
            return;
        };

        match made {
            Ok(plan) if plan.is_empty() => {
                job.state = State::Failed(String::from("there was nothing to do"));
                job.plan = Some(plan);
            }
            Ok(plan) => {
                job.state = if plan.clashes.is_empty() {
                    State::Queued
                } else {
                    State::Asking
                };
                job.plan = Some(plan);
            }
            Err(why) => job.state = State::Failed(why),
        }
    }

    /// Answer the clashes on a job, and let it go.
    ///
    /// A plan that resolves to nothing at all fails rather than running: a
    /// job that skipped every one of its steps did nothing, and saying so
    /// beats a bar that jumps to done.
    pub fn resolve(&mut self, id: Id, how: Resolve) {
        let Some(job) = self.find(id) else {
            return;
        };
        if job.state != State::Asking {
            return;
        }

        let Some(plan) = &mut job.plan else {
            return;
        };

        plan.resolve(how);
        job.state = if plan.is_empty() {
            State::Failed(String::from("there was nothing left to do"))
        } else {
            State::Queued
        };
    }

    /// The job, if any, that is waiting to be asked about its clashes.
    pub fn asking(&self) -> Option<&Job> {
        self.jobs.iter().find(|job| job.state == State::Asking)
    }

    /// Start whatever is waiting, up to the worker count.
    ///
    /// Called after anything that could free a worker or add a job, so the
    /// queue drains itself rather than needing somebody to remember.
    pub fn start_ready(&mut self) -> Task<Message> {
        let mut room = self
            .workers
            .saturating_sub(self.jobs.iter().filter(|job| job.state.running()).count());

        let mut started = Vec::new();
        for job in &mut self.jobs {
            if room == 0 {
                break;
            }
            if job.state != State::Queued {
                continue;
            }

            let Some(plan) = job.plan.clone() else {
                continue;
            };
            job.state = State::Running;
            room -= 1;
            started.push(working(job.id, plan, Arc::clone(&job.control)));
        }

        Task::batch(started)
    }

    /// What a worker said. Returns whether a worker came free.
    pub fn update(&mut self, id: Id, said: Update) -> bool {
        let Some(job) = self.find(id) else {
            return false;
        };

        match said {
            Update::Progress { bytes, steps } => {
                job.done = bytes;
                job.steps = steps;
                // A paused job stops sending, so the state is set from this
                // side: the window knows it asked.
                false
            }
            Update::Failed(path, why) => {
                job.errors.push((path, why));
                false
            }
            Update::Ended(End::Done) => {
                job.state = State::Done;
                if let Some(plan) = &job.plan {
                    job.done = plan.bytes;
                    job.steps = plan.steps.len();
                }
                true
            }
            Update::Ended(End::Cancelled { partial }) => {
                job.state = State::Cancelled(partial);
                true
            }
        }
    }

    pub fn pause(&mut self, id: Id) {
        if let Some(job) = self.find(id)
            && job.state == State::Running
        {
            job.control.pause();
            job.state = State::Paused;
        }
    }

    pub fn resume(&mut self, id: Id) {
        if let Some(job) = self.find(id)
            && job.state == State::Paused
        {
            job.control.resume();
            job.state = State::Running;
        }
    }

    /// Ask a job to stop.
    ///
    /// A job that never started is over at once; a running one has to be
    /// told, and says for itself what it left behind.
    pub fn cancel(&mut self, id: Id) {
        let Some(job) = self.find(id) else {
            return;
        };

        job.control.cancel();
        if !job.state.running() {
            job.state = State::Cancelled(None);
        }
    }

    /// Take a finished job off the list.
    pub fn dismiss(&mut self, id: Id) {
        self.jobs.retain(|job| job.id != id || !job.state.over());
    }
}

/// What a worker says while it works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// How far it has got. At most [`REPORT`] often, so a big tree does not
    /// flood the loop.
    Progress {
        bytes: u64,
        steps: usize,
    },
    /// One step failed. The job carries on.
    Failed(PathBuf, String),
    Ended(End),
}

/// How a job stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    Done,
    /// Somebody cancelled it. A file half written is named rather than
    /// quietly left: pretending a half-written copy can be undone is worse
    /// than saying which one it is.
    Cancelled {
        partial: Option<PathBuf>,
    },
}

/// What the loop hears from here.
#[derive(Debug, Clone)]
pub enum Message {
    Planned(Id, Result<Plan, String>),
    Said(Id, Update),
}

/// Work out a plan, on a thread.
fn planning(id: Id, work: Work, sources: Vec<PathBuf>, into: PathBuf) -> Task<Message> {
    Task::future(async move {
        let (sender, receiver) = tokio::sync::oneshot::channel();

        std::thread::spawn(move || {
            let _ = sender.send(plan(work, &sources, &into));
        });

        let made = receiver
            .await
            .unwrap_or_else(|_| Err(String::from("the plan was lost")));

        Message::Planned(id, made)
    })
}

/// Run a plan, on a thread, reporting as it goes.
fn working(id: Id, plan: Plan, control: Arc<Control>) -> Task<Message> {
    Task::run(
        iced::stream::channel(8, async move |mut output| {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(8);

            std::thread::spawn(move || carry_out(&plan, &control, &sender));

            while let Some(said) = receiver.recv().await {
                if output.send(Message::Said(id, said)).await.is_err() {
                    return;
                }
            }
        }),
        |message| message,
    )
}

/// Walk the sources and write down every step.
///
/// Blocking, and never called from `update`.
pub fn plan(work: Work, sources: &[PathBuf], into: &Path) -> Result<Plan, String> {
    if work != Work::Delete && !into.is_dir() {
        return Err(format!("{} is not a directory", into.display()));
    }

    let mut plan = Plan {
        work,
        into: into.to_path_buf(),
        steps: Vec::new(),
        bytes: 0,
        clashes: Vec::new(),
        skipped: Vec::new(),
    };

    if work == Work::Delete {
        for source in sources {
            count(source, 0, &mut plan);
        }
        return Ok(plan);
    }

    // A move that stays on one filesystem is a rename, which is one syscall
    // however big the tree is. Asked here rather than at the moment of the
    // move, so the plan can say which of the two this turned out to be.
    let landing = device(into);

    for source in sources {
        // Into itself, or into something inside itself. Both copy for ever,
        // and the second is the one nobody sees coming.
        if source == into {
            return Err(format!("{} is where it already is", source.display()));
        }
        if into.starts_with(source) {
            return Err(format!("{} is inside {}", into.display(), source.display()));
        }

        let Some(name) = source.file_name() else {
            plan.skipped
                .push((source.clone(), String::from("it has no name")));
            continue;
        };

        let to = into.join(name);

        if source.parent() == Some(into) {
            plan.skipped
                .push((source.clone(), String::from("it is already there")));
            continue;
        }

        if work == Work::Move && landing.is_some() && device(source) == landing {
            rename(source, &to, &mut plan);
            continue;
        }

        walk(source, &to, 0, &mut plan);

        // Across two filesystems a move cannot be a rename, so it is a copy
        // and then a delete. The removals go last, and `carry_out` drops them
        // if any step failed: a move must never take the original away when
        // the copy did not arrive.
        if work == Work::Move {
            plan.steps.push(Step::plain(
                source.clone(),
                source.clone(),
                What::Remove {
                    tree: source.is_dir(),
                    bytes: 0,
                },
            ));
        }
    }

    Ok(plan)
}

/// Which filesystem a path is on, or `None` if it cannot be read.
fn device(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;

    std::fs::metadata(path).ok().map(|found| found.dev())
}

/// A move on one filesystem: one step, whatever the tree holds.
fn rename(from: &Path, to: &Path, plan: &mut Plan) {
    let found = match crate::entry::Entry::read(from.to_path_buf()) {
        Ok(found) => found,
        Err(error) => {
            plan.skipped.push((from.to_path_buf(), error.to_string()));
            return;
        }
    };

    // The walk is only for the bar, the same as a delete. A rename moves a
    // whole tree without reading a byte of it.
    let bytes = if found.kind == Kind::Directory {
        under(from, 0, plan)
    } else {
        found.size
    };

    let mut step = Step::plain(from.to_path_buf(), to.to_path_buf(), What::Rename(bytes));

    // `rename` replaces a file at the destination without a word, so a clash
    // has to be caught here rather than left to the syscall.
    match clash(&found, to) {
        Ok(None) => plan.bytes += bytes,
        Ok(Some(found)) => {
            step.clash = Some(found);
            plan.clashes.push(to.to_path_buf());
        }
        Err(why) => {
            plan.skipped.push((from.to_path_buf(), why));
            return;
        }
    }

    plan.steps.push(step);
}

/// What is already at `to`, measured against what would replace it.
///
/// `Ok(None)` means the way is clear. An error means this pair can never be
/// resolved, whatever a person answers, so the step is dropped instead.
fn clash(source: &crate::entry::Entry, to: &Path) -> Result<Option<Clash>, String> {
    let Ok(there) = to.symlink_metadata() else {
        return Ok(None);
    };

    if there.is_dir() {
        return Err(String::from("a directory is already there"));
    }

    // A time that cannot be read counts as older, so "newer only" copies
    // rather than silently doing nothing.
    let newer = match (source.modified, there.modified().ok()) {
        (Some(mine), Some(theirs)) => mine > theirs,
        _ => true,
    };

    Ok(Some(Clash {
        newer,
        larger: source.size > there.len(),
    }))
}

/// A free name beside `path`: `note.txt`, then `note (2).txt`.
///
/// `taken` is what this plan has already claimed and not yet written, so two
/// clashes on one name do not both land on `(2)`. If nothing is free the
/// original comes back unchanged, and the step then fails on `create_new`
/// and is reported -- which loses nothing, unlike guessing.
fn beside(path: &Path, taken: &[PathBuf]) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let name = path.file_name().unwrap_or_default().to_string_lossy();

    // A leading dot is part of the name, not an extension: `.bashrc` is all
    // stem. `archive.tar.gz` keeps only the last suffix, which is what every
    // other file manager does.
    let (stem, extension) = match name.rfind('.') {
        Some(at) if at > 0 => (&name[..at], &name[at..]),
        _ => (name.as_ref(), ""),
    };

    (2..10_000)
        .map(|number| parent.join(format!("{stem} ({number}){extension}")))
        .find(|candidate| !taken.contains(candidate) && candidate.symlink_metadata().is_err())
        .unwrap_or_else(|| path.to_path_buf())
}

/// One thing to delete, and a count of what is under it.
///
/// One step, whatever the tree holds. The walk is only to say how much this
/// is -- a person about to delete something for good has a right to know it
/// is 12,000 files and not three. See [`What::Remove`] for why the removal
/// itself is not made of steps.
fn count(path: &Path, depth: usize, plan: &mut Plan) {
    let found = match crate::entry::Entry::read(path.to_path_buf()) {
        Ok(found) => found,
        Err(error) => {
            plan.skipped.push((path.to_path_buf(), error.to_string()));
            return;
        }
    };

    // A directory, and not a link that points at one. A link is one thing to
    // unlink, and what it points at is none of this job's business.
    let tree = found.kind == Kind::Directory;
    let bytes = if tree {
        under(path, depth, plan)
    } else {
        found.size
    };

    plan.bytes += bytes;
    plan.steps.push(Step::plain(
        path.to_path_buf(),
        path.to_path_buf(),
        What::Remove { tree, bytes },
    ));
}

/// What a directory holds, in bytes. Counted, never touched.
fn under(path: &Path, depth: usize, plan: &mut Plan) -> u64 {
    if depth > DEPTH {
        plan.skipped
            .push((path.to_path_buf(), format!("more than {DEPTH} deep")));
        return 0;
    }

    let listing = match std::fs::read_dir(path) {
        Ok(listing) => listing,
        Err(error) => {
            plan.skipped.push((path.to_path_buf(), error.to_string()));
            return 0;
        }
    };

    let mut bytes = 0;
    for child in listing.flatten() {
        let path = child.path();
        let Ok(found) = crate::entry::Entry::read(path.clone()) else {
            continue;
        };

        if found.kind == Kind::Directory {
            bytes += under(&path, depth + 1, plan);
        } else {
            bytes += found.size;
        }
    }
    bytes
}

/// One entry, and everything under it.
fn walk(from: &Path, to: &Path, depth: usize, plan: &mut Plan) {
    if depth > DEPTH {
        plan.skipped
            .push((from.to_path_buf(), format!("more than {DEPTH} deep")));
        return;
    }

    let found = match crate::entry::Entry::read(from.to_path_buf()) {
        Ok(found) => found,
        Err(error) => {
            plan.skipped.push((from.to_path_buf(), error.to_string()));
            return;
        }
    };

    // Already there. A directory is merged into, which is what every file
    // manager does and what makes copying into a half-copied tree work.
    // Anything else is a question for a person.
    let there = to.symlink_metadata();
    match found.kind {
        Kind::Directory => {
            if let Ok(there) = &there {
                if !there.is_dir() {
                    plan.skipped.push((
                        from.to_path_buf(),
                        String::from("a file of that name is already there"),
                    ));
                    return;
                }
            } else {
                plan.steps.push(Step::plain(
                    from.to_path_buf(),
                    to.to_path_buf(),
                    What::Directory,
                ));
            }

            let listing = match std::fs::read_dir(from) {
                Ok(listing) => listing,
                Err(error) => {
                    plan.skipped.push((from.to_path_buf(), error.to_string()));
                    return;
                }
            };

            for child in listing {
                let Ok(child) = child else { continue };
                let name = child.file_name();
                walk(&from.join(&name), &to.join(&name), depth + 1, plan);
            }
        }

        Kind::File | Kind::Link { .. } => {
            let what = if found.kind == Kind::File {
                What::File(found.size)
            } else {
                match std::fs::read_link(from) {
                    Ok(target) => What::Link(target),
                    Err(error) => {
                        plan.skipped.push((from.to_path_buf(), error.to_string()));
                        return;
                    }
                }
            };

            let mut step = Step::plain(from.to_path_buf(), to.to_path_buf(), what);

            // A clashing step is kept rather than dropped, so answering it
            // later needs no second walk. Its bytes stay out of the total
            // until `Plan::resolve` says it will happen.
            match clash(&found, to) {
                // A link counts as nothing, the same as a directory: a
                // percentage only means something if it is bytes that move.
                Ok(None) if found.kind == Kind::File => plan.bytes += found.size,
                Ok(None) => {}
                Ok(Some(found)) => {
                    step.clash = Some(found);
                    plan.clashes.push(to.to_path_buf());
                }
                Err(why) => {
                    plan.skipped.push((from.to_path_buf(), why));
                    return;
                }
            }

            plan.steps.push(step);
        }

        Kind::Other => plan.skipped.push((
            from.to_path_buf(),
            String::from("it is not a file, a directory or a link"),
        )),
    }
}

/// The worker itself. Blocking, on a thread of its own.
fn carry_out(plan: &Plan, control: &Control, sender: &tokio::sync::mpsc::Sender<Update>) {
    // A test that names the wrong path copies or deletes for real, and a
    // delete leaves nothing behind to notice it by. The run stops here rather
    // than reporting an error the test could ignore. `cfg(test)`, so the
    // shipped binary carries none of it.
    #[cfg(test)]
    for step in &plan.steps {
        // What the step changes. A removal takes `from` away; everything else
        // writes `to`. Reading a path outside `tmp/` harms nothing.
        let touched = match step.what {
            What::Remove { .. } => &step.from,
            _ => &step.to,
        };
        assert!(
            crate::testing::inside(touched),
            "a test may only change what is under {}: {}",
            crate::testing::root().display(),
            touched.display()
        );
    }

    let mut done = 0u64;
    let mut last = std::time::Instant::now();
    let mut failures = 0usize;

    for (at, step) in plan.steps.iter().enumerate() {
        let steps = at;
        if !control.carry_on() {
            let _ = sender.blocking_send(Update::Ended(End::Cancelled { partial: None }));
            return;
        }

        // A move across two filesystems is a copy and then a delete, and the
        // deletes are the last steps. One failed copy stops all of them: it
        // is better to leave a file in both places than in neither, and
        // working out which source the failure belonged to would mean the
        // plan carrying a tree of its own.
        if plan.work == Work::Move && failures > 0 && matches!(step.what, What::Remove { .. }) {
            continue;
        }

        let outcome = match &step.what {
            What::Directory => std::fs::create_dir_all(&step.to).map(|()| 0),

            What::Link(target) => {
                if step.replace {
                    let _ = std::fs::remove_file(&step.to);
                }
                std::os::unix::fs::symlink(target, &step.to).map(|()| 0)
            }

            What::Rename(bytes) => {
                // `rename` replaces a file at the destination without a word,
                // so a step that was not resolved to replace must find the
                // way clear. `clash` said it was when the plan was built;
                // this is the guard against it changing since.
                let gone = if step.replace || step.to.symlink_metadata().is_err() {
                    std::fs::rename(&step.from, &step.to)
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        "something arrived there while the job waited",
                    ))
                };

                if gone.is_ok() {
                    done += bytes;
                }
                gone.map(|()| 0)
            }

            What::File(_) => match copy(step, control, &mut done, steps, &mut last, sender) {
                Copied::Whole => Ok(0),
                Copied::Failed(error) => Err(error),
                Copied::Stopped => {
                    let _ = sender.blocking_send(Update::Ended(End::Cancelled {
                        partial: Some(step.to.clone()),
                    }));
                    return;
                }
            },

            What::Remove { tree, bytes } => {
                let gone = if *tree {
                    std::fs::remove_dir_all(&step.from)
                } else {
                    // `remove_file` unlinks a symlink rather than what it
                    // points at, which is what makes deleting a link safe.
                    std::fs::remove_file(&step.from)
                };

                if gone.is_ok() {
                    done += bytes;
                }
                gone.map(|()| 0)
            }
        };

        if let Err(error) = outcome {
            failures += 1;
            let _ = sender.blocking_send(Update::Failed(step.to.clone(), error.to_string()));
        }

        if last.elapsed() >= REPORT {
            last = std::time::Instant::now();
            let _ = sender.blocking_send(Update::Progress {
                bytes: done,
                steps: at + 1,
            });
        }
    }

    let _ = sender.blocking_send(Update::Ended(End::Done));
}

/// How one file ended.
enum Copied {
    Whole,
    Stopped,
    Failed(std::io::Error),
}

/// Copy one file, a chunk at a time, looking at the control between chunks.
///
/// A plain read and write loop. `copy_file_range` and reflinks are the next
/// item; this is the fallback they fall back to, so it is the one that has
/// to be right first.
fn copy(
    step: &Step,
    control: &Control,
    done: &mut u64,
    steps: usize,
    last: &mut std::time::Instant,
    sender: &tokio::sync::mpsc::Sender<Update>,
) -> Copied {
    use std::io::{Read, Write};

    let mut read = match std::fs::File::open(&step.from) {
        Ok(file) => file,
        Err(error) => return Copied::Failed(error),
    };

    // `create_new` unless a person said to replace this one. The plan said
    // nothing was there, and between the plan and now somebody may have made
    // it; overwriting on a race is exactly the fault the clash list exists to
    // avoid. `truncate` where replacing, or the tail of a longer file is left
    // on the end of a shorter one.
    let mut write = match std::fs::File::options()
        .write(true)
        .create_new(!step.replace)
        .create(step.replace)
        .truncate(step.replace)
        .open(&step.to)
    {
        Ok(file) => file,
        Err(error) => return Copied::Failed(error),
    };

    let mut buffer = vec![0u8; CHUNK];

    loop {
        if !control.carry_on() {
            return Copied::Stopped;
        }

        let got = match read.read(&mut buffer) {
            Ok(0) => break,
            Ok(got) => got,
            Err(error) => return Copied::Failed(error),
        };

        if let Err(error) = write.write_all(&buffer[..got]) {
            return Copied::Failed(error);
        }

        *done += got as u64;

        if last.elapsed() >= REPORT {
            *last = std::time::Instant::now();
            // `try_send`, not `blocking_send`: a window that is busy must
            // never hold up the copy. The next report carries the same
            // number a moment later, which is what makes progress the kind
            // of traffic that may be dropped.
            let _ = sender.try_send(Update::Progress {
                bytes: *done,
                steps,
            });
        }
    }

    // The mode, so an executable stays one. Times and ownership are a job
    // for the properties panel, not for a copy nobody asked to preserve.
    if let Ok(metadata) = read.metadata() {
        use std::os::unix::fs::PermissionsExt;
        let _ = write.set_permissions(std::fs::Permissions::from_mode(
            metadata.permissions().mode(),
        ));
    }

    if control.cancelled() {
        return Copied::Stopped;
    }

    Copied::Whole
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree to copy, under a name of this test's own so two tests never
    /// share one.
    fn tree(name: &str) -> PathBuf {
        let root = crate::testing::scratch(&format!("jobs-{name}"));
        std::fs::create_dir_all(root.join("from/inner")).expect("should make the tree");
        std::fs::create_dir_all(root.join("into")).expect("should make the destination");
        std::fs::write(root.join("from/one.txt"), b"one").expect("should write");
        std::fs::write(root.join("from/inner/two.txt"), b"two two").expect("should write");
        root
    }

    /// A plan names every step before anything happens, and a directory
    /// always comes before what is inside it.
    #[test]
    fn a_plan_lists_parents_before_children() {
        let root = tree("order");
        let made = plan(Work::Copy, &[root.join("from")], &root.join("into")).expect("should plan");

        let mut seen: Vec<&Path> = Vec::new();
        for step in &made.steps {
            if let Some(parent) = step.to.parent()
                && parent.starts_with(root.join("into"))
                && parent != root.join("into")
            {
                assert!(
                    seen.contains(&parent),
                    "{} arrives before its directory",
                    step.to.display()
                );
            }
            seen.push(&step.to);
        }

        assert_eq!(made.bytes, 3 + 7, "the files add up");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Copying a directory into itself, or into something inside itself,
    /// never ends. Both are refused before a byte moves.
    #[test]
    fn a_directory_cannot_be_copied_into_itself() {
        let root = tree("itself");
        let from = root.join("from");

        assert!(plan(Work::Copy, std::slice::from_ref(&from), &from).is_err());
        assert!(plan(Work::Copy, std::slice::from_ref(&from), &from.join("inner")).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A destination that is already there is a question for a person. The
    /// step is kept so the answer needs no second walk, but it costs nothing
    /// and does nothing until [`Plan::resolve`] runs.
    #[test]
    fn a_file_that_is_already_there_is_a_clash() {
        let root = tree("clash");
        std::fs::create_dir_all(root.join("into/from")).expect("should make it");
        std::fs::write(root.join("into/from/one.txt"), b"mine").expect("should write");

        let made = plan(Work::Copy, &[root.join("from")], &root.join("into")).expect("should plan");
        let at = root.join("into/from/one.txt");

        assert_eq!(made.clashes, std::slice::from_ref(&at));

        let step = made
            .steps
            .iter()
            .find(|step| step.to == at)
            .expect("the step is kept, so the answer needs no second walk");
        assert!(step.clash.is_some());
        assert!(!step.replace, "and nothing replaces anything yet");
        assert_eq!(
            made.bytes, 7,
            "only `two two` counts; the clashing file is not in the total"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Each answer does one plain thing, and only "skip" leaves the plan as
    /// it was before anybody asked.
    #[test]
    fn every_answer_to_a_clash_does_what_it_says() {
        let root = tree("resolve");
        std::fs::create_dir_all(root.join("into/from")).expect("should make it");
        std::fs::write(root.join("into/from/one.txt"), b"mine").expect("should write");
        let at = root.join("into/from/one.txt");

        // Said rather than assumed. The order the test wrote the two files in
        // is not a time to test against: it made the destination the newer of
        // the two, which is the opposite of what reads naturally here.
        let long_ago = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        std::fs::File::options()
            .write(true)
            .open(&at)
            .expect("should open")
            .set_times(std::fs::FileTimes::new().set_modified(long_ago))
            .expect("should set the time");

        let made = || plan(Work::Copy, &[root.join("from")], &root.join("into")).expect("plans");
        let of = |plan: &Plan, path: &Path| plan.steps.iter().any(|step| step.to == path);

        let mut skipped = made();
        skipped.resolve(Resolve::Skip);
        assert!(!of(&skipped, &at), "skip drops the step");
        assert_eq!(skipped.bytes, 7, "and adds nothing to the total");
        assert!(skipped.clashes.is_empty(), "the question is answered");

        let mut over = made();
        over.resolve(Resolve::Overwrite);
        let step = over.steps.iter().find(|step| step.to == at).expect("kept");
        assert!(step.replace, "overwrite replaces what is there");
        assert_eq!(over.bytes, 7 + 3, "and now its bytes count");

        let mut both = made();
        both.resolve(Resolve::KeepBoth);
        assert!(!of(&both, &at), "keep both writes somewhere else");
        assert!(
            of(&both, &root.join("into/from/one (2).txt")),
            "beside the one that is there, with a free name"
        );

        // `one.txt` holds `one`, three bytes, against `mine`, which is four.
        let mut larger = made();
        larger.resolve(Resolve::Larger);
        assert!(!of(&larger, &at), "the source is smaller, so it is left");

        // And the destination was set to 1970, so the source is the newer.
        let mut newer = made();
        newer.resolve(Resolve::Newer);
        assert!(of(&newer, &at), "the source is newer, so it replaces");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A move inside one filesystem is a rename: one step for a whole tree,
    /// and no byte read.
    #[test]
    fn a_move_on_one_filesystem_is_one_rename() {
        let root = tree("rename");
        let made = plan(Work::Move, &[root.join("from")], &root.join("into")).expect("should plan");

        assert_eq!(made.steps.len(), 1, "one step, whatever the tree holds");
        assert_eq!(made.steps[0].what, What::Rename(3 + 7));
        assert_eq!(made.steps[0].to, root.join("into/from"));

        let (sender, _receiver) = tokio::sync::mpsc::channel(64);
        carry_out(&made, &Control::default(), &sender);

        assert!(!root.join("from").exists(), "the original is gone");
        assert!(root.join("into/from/one.txt").is_file(), "and arrived");
        assert!(root.join("into/from/inner/two.txt").is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A move onto a name that is taken must not go through: `rename`
    /// replaces a file without a word, so the clash has to be caught first.
    #[test]
    fn a_move_onto_a_name_that_is_taken_asks_first() {
        let root = tree("rename-clash");
        std::fs::write(root.join("into/from"), b"do not lose me").expect("should write");

        let made = plan(Work::Move, &[root.join("from")], &root.join("into")).expect("should plan");
        assert_eq!(made.clashes, [root.join("into/from")]);

        let (sender, _receiver) = tokio::sync::mpsc::channel(64);
        carry_out(&made, &Control::default(), &sender);

        assert_eq!(
            std::fs::read(root.join("into/from")).expect("still there"),
            b"do not lose me",
            "an unanswered clash moves nothing"
        );
        assert!(root.join("from").is_dir(), "and the source stays put");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A move that cannot rename must never take the original away when the
    /// copy did not arrive.
    #[test]
    fn a_failed_copy_keeps_the_original() {
        let root = tree("move-failed");

        // A cross-filesystem move, built by hand: the copy steps, then the
        // removal, which is the shape `plan` makes when the devices differ.
        let mut made =
            plan(Work::Copy, &[root.join("from")], &root.join("into")).expect("should plan");
        made.work = Work::Move;
        made.steps.push(Step::plain(
            root.join("from"),
            root.join("from"),
            What::Remove {
                tree: true,
                bytes: 0,
            },
        ));

        // Something is already at one destination, so that step fails.
        std::fs::create_dir_all(root.join("into/from")).expect("should make it");
        std::fs::write(root.join("into/from/one.txt"), b"in the way").expect("should write");

        let (sender, _receiver) = tokio::sync::mpsc::channel(64);
        carry_out(&made, &Control::default(), &sender);

        assert!(
            root.join("from/one.txt").is_file(),
            "one step failed, so nothing is removed"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A symlink is copied as a symlink. Following one is how a copy of a
    /// small directory turns into a copy of the whole disk.
    #[test]
    fn a_link_is_copied_and_not_followed() {
        let root = tree("link");
        std::os::unix::fs::symlink("/etc", root.join("from/away")).expect("should link");

        let made = plan(Work::Copy, &[root.join("from")], &root.join("into")).expect("should plan");

        let link = made
            .steps
            .iter()
            .find(|step| step.to.ends_with("away"))
            .expect("the link is a step");
        assert_eq!(link.what, What::Link(PathBuf::from("/etc")));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// And the whole thing actually copies, with the tree and the bytes
    /// arriving intact.
    #[test]
    fn a_plan_carried_out_copies_the_tree() {
        let root = tree("run");
        let made = plan(Work::Copy, &[root.join("from")], &root.join("into")).expect("should plan");

        let (sender, mut receiver) = tokio::sync::mpsc::channel(64);
        let control = Control::default();
        carry_out(&made, &control, &sender);
        drop(sender);

        let mut ended = None;
        while let Ok(said) = receiver.try_recv() {
            if let Update::Ended(end) = said {
                ended = Some(end);
            }
        }
        assert_eq!(ended, Some(End::Done));

        assert_eq!(
            std::fs::read(root.join("into/from/one.txt")).expect("copied"),
            b"one"
        );
        assert_eq!(
            std::fs::read(root.join("into/from/inner/two.txt")).expect("copied"),
            b"two two"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Cancelling before it starts stops it, and nothing is written.
    #[test]
    fn a_cancelled_job_writes_nothing() {
        let root = tree("cancel");
        let made = plan(Work::Copy, &[root.join("from")], &root.join("into")).expect("should plan");

        let (sender, mut receiver) = tokio::sync::mpsc::channel(64);
        let control = Control::default();
        control.cancel();
        carry_out(&made, &control, &sender);
        drop(sender);

        assert_eq!(
            receiver.try_recv(),
            Ok(Update::Ended(End::Cancelled { partial: None }))
        );
        assert!(!root.join("into/from").exists(), "nothing was made");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A delete is one step per thing chosen, whatever the tree holds -- and
    /// the walk still says how big it is, because a person about to delete
    /// something for good has a right to know it is not three files.
    #[test]
    fn a_delete_is_one_step_and_a_real_count() {
        let root = tree("delete");
        let made = plan(
            Work::Delete,
            std::slice::from_ref(&root.join("from")),
            &root,
        )
        .expect("should plan");

        assert_eq!(made.steps.len(), 1, "one thing was chosen");
        assert_eq!(
            made.steps[0].what,
            What::Remove {
                tree: true,
                bytes: 3 + 7
            }
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// And it really removes it, tree and all.
    #[test]
    fn a_delete_takes_the_whole_tree() {
        let root = tree("removed");
        let made = plan(
            Work::Delete,
            std::slice::from_ref(&root.join("from")),
            &root,
        )
        .expect("should plan");

        let (sender, _receiver) = tokio::sync::mpsc::channel(64);
        carry_out(&made, &Control::default(), &sender);

        assert!(!root.join("from").exists());
        assert!(root.join("into").exists(), "and nothing else went with it");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A symlink is unlinked, and what it points at is left alone. Deleting
    /// a link to `/etc` must not be a way to delete `/etc`.
    #[test]
    fn deleting_a_link_leaves_its_target() {
        let root = tree("unlink");
        let away = root.join("from/away");
        std::os::unix::fs::symlink(root.join("into"), &away).expect("should link");

        let made = plan(Work::Delete, std::slice::from_ref(&away), &root).expect("should plan");
        assert_eq!(
            made.steps[0].what,
            What::Remove {
                tree: false,
                bytes: made.bytes
            },
            "a link is one thing to unlink, not a tree to walk"
        );

        let (sender, _receiver) = tokio::sync::mpsc::channel(64);
        carry_out(&made, &Control::default(), &sender);

        assert!(!away.exists());
        assert!(root.join("into").is_dir(), "the target is still there");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A plan with one step, for the queue tests. Nothing here is run: a
    /// `Task` that is dropped is never polled, so no thread starts.
    fn one_step() -> Plan {
        Plan {
            work: Work::Copy,
            into: PathBuf::from("/tmp"),
            steps: vec![Step::plain(
                PathBuf::from("/tmp/a"),
                PathBuf::from("/tmp/b"),
                What::File(1),
            )],
            bytes: 1,
            clashes: Vec::new(),
            skipped: Vec::new(),
        }
    }

    /// The guard is the reason a wrong path in a test is not a lost file.
    ///
    /// `one_step` names `/tmp/b`, which is outside the repository's `tmp/`.
    /// The queue tests hand that plan about safely, because a dropped `Task`
    /// never runs it; the moment anything does run it, it stops.
    #[test]
    #[should_panic(expected = "a test may only change what is under")]
    fn a_copy_outside_tmp_stops_the_test() {
        let (sender, _receiver) = tokio::sync::mpsc::channel(64);
        carry_out(&one_step(), &Control::default(), &sender);
    }

    /// And the same for a delete, which is the one with nothing to put back.
    #[test]
    #[should_panic(expected = "a test may only change what is under")]
    fn a_delete_outside_tmp_stops_the_test() {
        let plan = Plan {
            work: Work::Delete,
            into: PathBuf::from("/etc"),
            steps: vec![Step::plain(
                PathBuf::from("/etc/passwd"),
                PathBuf::from("/etc/passwd"),
                What::Remove {
                    tree: false,
                    bytes: 1,
                },
            )],
            bytes: 1,
            clashes: Vec::new(),
            skipped: Vec::new(),
        };

        let (sender, _receiver) = tokio::sync::mpsc::channel(64);
        carry_out(&plan, &Control::default(), &sender);
    }

    /// A job waits for its plan before it is worth starting, and a plan that
    /// cannot be built fails the job rather than leaving it waiting for ever.
    #[test]
    fn a_job_waits_for_its_plan() {
        let mut queue = Queue::new(2);
        let (id, task) = queue.add(
            Work::Copy,
            vec![PathBuf::from("/tmp/a")],
            PathBuf::from("/tmp"),
        );
        drop(task);

        assert_eq!(
            queue.get(id).map(|job| job.state.clone()),
            Some(State::Planning)
        );
        drop(queue.start_ready());
        assert_eq!(
            queue.get(id).map(|job| job.state.clone()),
            Some(State::Planning),
            "nothing starts before its plan"
        );

        queue.planned(id, Err(String::from("no")));
        assert!(matches!(
            queue.get(id).map(|job| job.state.clone()),
            Some(State::Failed(_))
        ));
    }

    /// The worker count is a limit. The rest wait, and a queue that started
    /// everything at once would make a disk slower, not faster.
    #[test]
    fn only_as_many_run_as_there_are_workers() {
        let mut queue = Queue::new(1);

        let mut ids = Vec::new();
        for _ in 0..3 {
            let (id, task) = queue.add(
                Work::Copy,
                vec![PathBuf::from("/tmp/a")],
                PathBuf::from("/tmp"),
            );
            drop(task);
            queue.planned(id, Ok(one_step()));
            ids.push(id);
        }

        drop(queue.start_ready());

        let running = queue.iter().filter(|job| job.state.running()).count();
        assert_eq!(running, 1, "one worker, one job");
        assert_eq!(
            queue.get(ids[1]).map(|job| job.state.clone()),
            Some(State::Queued)
        );

        // The first one ends, and the next takes its place.
        assert!(
            queue.update(ids[0], Update::Ended(End::Done)),
            "a worker came free"
        );
        drop(queue.start_ready());
        assert_eq!(
            queue.get(ids[1]).map(|job| job.state.clone()),
            Some(State::Running)
        );
    }

    /// Cancelling a job that never started is over at once. Waiting for a
    /// worker to notice would leave a row that says "running" and is not.
    #[test]
    fn cancelling_a_waiting_job_ends_it_now() {
        let mut queue = Queue::new(1);
        let (id, task) = queue.add(
            Work::Copy,
            vec![PathBuf::from("/tmp/a")],
            PathBuf::from("/tmp"),
        );
        drop(task);
        queue.planned(id, Ok(one_step()));

        queue.cancel(id);
        assert_eq!(
            queue.get(id).map(|job| job.state.clone()),
            Some(State::Cancelled(None))
        );

        drop(queue.start_ready());
        assert_eq!(
            queue.get(id).map(|job| job.state.clone()),
            Some(State::Cancelled(None)),
            "and it does not then start"
        );
    }

    /// Only a finished job leaves the panel. Dismissing a running one would
    /// leave a worker nobody can see or stop.
    #[test]
    fn only_a_finished_job_can_be_dismissed() {
        let mut queue = Queue::new(1);
        let (id, task) = queue.add(
            Work::Copy,
            vec![PathBuf::from("/tmp/a")],
            PathBuf::from("/tmp"),
        );
        drop(task);
        queue.planned(id, Ok(one_step()));
        drop(queue.start_ready());

        queue.dismiss(id);
        assert!(queue.get(id).is_some(), "a running job stays");

        queue.update(id, Update::Ended(End::Done));
        queue.dismiss(id);
        assert!(queue.get(id).is_none());
    }

    /// Pause holds, resume lets go, and cancel wins over both. A paused job
    /// that could not be cancelled would leave a thread asleep for ever.
    #[test]
    fn cancel_beats_pause() {
        let control = Control::default();
        control.pause();
        control.cancel();
        assert!(!control.carry_on(), "a cancelled job does not wait");

        // And pausing after that changes nothing.
        control.pause();
        assert!(!control.carry_on());
    }
}
