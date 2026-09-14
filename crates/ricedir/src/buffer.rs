//! One open directory: what is in it, where the cursor is, what is selected.
//!
//! A buffer outlives the tile showing it, so closing a tile costs nothing and
//! reopening the same directory is instant. Two tiles on one directory share
//! one buffer, and so one listing and one watcher.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use iced::Task;
use iced::futures::SinkExt;

use crate::config::List;
use crate::entry::{self, Entry};

/// How many entries cross the channel at once.
///
/// One message per entry would make a directory of 100k files 100k trips
/// through the Elm loop; one message for the lot would leave a slow mount
/// blank until it finished. A chunk paints early and costs little.
const CHUNK: usize = 512;

/// What the listing thread has to say.
#[derive(Debug, Clone)]
pub enum Update {
    /// Another chunk, in readdir order. Sorting waits for [`Update::Done`].
    Entries(Vec<Entry>),
    Done,
    /// The directory could not be read at all.
    Failed(String),
}

/// Whether this buffer is showing what is really there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listing {
    Loading,
    Ready,
    Failed(String),
}

/// Where the cursor and the selection were, by name rather than by index.
#[derive(Debug, Default)]
struct Remembered {
    cursor: Option<String>,
    selected: HashSet<String>,
}

/// An open directory.
#[derive(Debug)]
pub struct Buffer {
    pub path: PathBuf,
    pub listing: Listing,

    /// Every entry, sorted. Indices into this are what selection remembers.
    entries: Vec<Entry>,
    /// Indices into `entries` that the filter lets through, in shown order.
    /// The list widget draws this, so a filter never disturbs a selection.
    visible: Vec<usize>,

    /// A replacement listing being collected, while the old one stays up.
    ///
    /// `None` means stream straight into `entries`, which is what a first
    /// listing does: there is nothing on screen to preserve, and a slow mount
    /// should paint as it arrives rather than after. `Some` means something
    /// is already showing -- a relist, or a tile coming back into view -- and
    /// clearing it first would blank the tile for as long as the read takes.
    /// On a directory of 100,000 that is long enough to look broken.
    arriving: Option<Vec<Entry>>,

    /// Where the keyboard cursor is, as a position in `visible`.
    pub cursor: usize,
    /// Whether anyone has actually put the cursor somewhere.
    ///
    /// Until they have, a relist leaves it at the top. Following it by name
    /// from the start would latch it onto whichever entry happened to sort
    /// first in the first chunk to arrive, and then drag it down the listing
    /// as the rest came in -- which is how a directory of 100k files opened
    /// with the cursor on `file17`.
    placed: bool,
    /// Where a shift-extended selection started, as a position in `visible`.
    pub anchor: usize,
    /// Selected entries, by index into `entries`.
    pub selection: HashSet<usize>,

    /// What the filter box says. Empty means everything.
    pub filter: String,
    /// Whether the filter box is on screen for this directory.
    ///
    /// Hidden until asked for, because a box that is always there is a box
    /// that is always in the way.
    pub filtering: bool,

    /// The path being typed, when this directory's path bar is showing its
    /// text face. `None` is the breadcrumbs.
    ///
    /// The draft, not the path: what is typed has to survive being wrong. A
    /// half-finished path names nothing, and replacing the buffer's own path
    /// with it would relist on every keystroke.
    ///
    /// Per buffer for the same reason the view is. It was one field on the
    /// window once, and then a tile split while a path was half typed handed
    /// the draft to whichever tile the keyboard moved to next.
    pub typing_path: Option<String>,

    /// How this directory is arranged, and whether the dotfiles show.
    ///
    /// Per buffer, not per window: see [`crate::config::View`].
    pub view: crate::config::View,

    /// Directories already visited, for back and forward.
    pub history: Vec<PathBuf>,
    pub future: Vec<PathBuf>,

    /// Bumped on every relist, so a chunk from a listing that has been
    /// replaced can be recognised and dropped.
    pub generation: u64,

    /// When the entries were last sorted, so a streaming listing is not
    /// re-sorted once per chunk. `None` until the first chunk arrives.
    sorted: Option<Instant>,

    /// Something changed that could not be applied, so read it all again.
    ///
    /// Set when a change arrives mid-relist: the entries being reconciled are
    /// the ones about to be thrown away, and the replacement was read before
    /// the change happened, so neither is right. Without this the change is
    /// simply lost and the listing stops matching the disk until somebody
    /// presses F5.
    stale: bool,
}

impl Buffer {
    /// A fresh buffer. `view` comes from the config, or from the tile that
    /// split to make this one.
    pub fn new(path: PathBuf, view: crate::config::View) -> Self {
        Self {
            path,
            view,
            listing: Listing::Loading,
            entries: Vec::new(),
            visible: Vec::new(),
            arriving: None,
            cursor: 0,
            placed: false,
            anchor: 0,
            selection: HashSet::new(),
            filter: String::new(),
            filtering: false,
            typing_path: None,
            history: Vec::new(),
            future: Vec::new(),
            generation: 0,
            sorted: None,
            stale: false,
        }
    }

    /// The entries the list should draw, in order.
    pub fn shown(&self) -> impl ExactSizeIterator<Item = &Entry> {
        self.visible.iter().map(|index| &self.entries[*index])
    }

    /// How many rows the list has.
    pub const fn rows(&self) -> usize {
        self.visible.len()
    }

    /// The entry at a row, if there is one.
    pub fn at(&self, row: usize) -> Option<&Entry> {
        self.visible.get(row).map(|index| &self.entries[*index])
    }

    /// Whether the entry at a row is selected.
    pub fn is_selected(&self, row: usize) -> bool {
        self.visible
            .get(row)
            .is_some_and(|index| self.selection.contains(index))
    }

    /// Begin a replacement listing, keeping what is on screen.
    ///
    /// A buffer showing nothing has nothing to keep, so it streams straight
    /// into `entries` and paints as the chunks land -- which is what makes a
    /// slow mount bearable.
    pub fn start_arriving(&mut self) {
        self.sorted = None;
        self.arriving = if self.entries.is_empty() {
            None
        } else {
            Some(Vec::with_capacity(self.entries.len()))
        };
    }

    /// About to show a different directory: drop what belonged to the old one.
    ///
    /// A filter is about the listing in front of you. Carrying it into the
    /// next directory shows a handful of its files with nothing on screen
    /// saying why the rest are missing -- which is exactly what "filter, then
    /// open a folder" did.
    ///
    /// Not the same as a relist. Watching a directory that is being filtered
    /// must keep the filter, or typing into the box while files arrive would
    /// undo itself; see `a_relist_must_not_clear_the_filter`.
    pub fn leaving(&mut self, list: &List) {
        if self.filter.is_empty() && !self.filtering {
            return;
        }
        self.filter.clear();
        self.filtering = false;
        self.rebuild(list);
    }

    /// Whether a replacement listing is being read behind what is showing.
    pub const fn refreshing(&self) -> bool {
        self.arriving.is_some()
    }

    /// Whether something changed that could not be applied, clearing the mark.
    ///
    /// Asked once a listing lands, so the caller can read it again.
    pub const fn take_stale(&mut self) -> bool {
        let was = self.stale;
        self.stale = false;
        was
    }

    /// How much of a replacement listing has arrived so far.
    ///
    /// Says how big the directory is turning out to be as well as that
    /// something is happening, which is more than a moving bar can. It is
    /// not enough on its own, though: it only changes when a chunk lands, so
    /// on a slow mount it moves rarely and on a hung one never. The bar
    /// beside it runs off a clock for that reason.
    pub fn read_so_far(&self) -> usize {
        self.arriving.as_ref().map_or(0, Vec::len)
    }

    /// Take another chunk from the listing thread.
    ///
    /// Sorting is throttled rather than done per chunk. Every chunk means one
    /// sort of everything that has arrived so far, which over 196 chunks of a
    /// 100k directory is quadratic: measured at 7.9 core-seconds against 0.48
    /// for a small one. Ten sorts a second keeps a growing listing readable
    /// and costs a fraction of that, and [`Self::finish`] always sorts, so
    /// what settles is right however the chunks fell.
    pub fn extend(&mut self, entries: Vec<Entry>, list: &List) {
        // A replacement listing is collected out of sight and shown all at
        // once. Sorting it as it grows would cost the same quadratic work for
        // a result nobody sees, and the old listing is still on screen.
        if let Some(arriving) = &mut self.arriving {
            arriving.extend(entries);
            return;
        }

        self.entries.extend(entries);

        let due = self
            .sorted
            .is_none_or(|last| last.elapsed() >= Duration::from_millis(100));

        if due {
            self.sorted = Some(Instant::now());
            self.rebuild(list);
        }
    }

    /// Bring named paths up to date, without reading the directory again.
    ///
    /// **One rule: re-read every path the events mentioned, and let the
    /// answer decide.** It exists now, so insert it or replace what is there;
    /// it does not, so take it out. That covers create, remove, a metadata
    /// change, and every shape of rename, without pairing anything.
    ///
    /// The reading happens on a worker thread and arrives here already done,
    /// as `(path, what it is now)` pairs -- `None` meaning gone. An earlier
    /// version called `symlink_metadata` in this function, which put a
    /// blocking syscall on the Elm loop: microseconds on a local disk, tens
    /// of milliseconds each over sshfs, and for ever on a hung NFS mount,
    /// with the whole window frozen behind it.
    ///
    /// Pairing was the hazard this was expected to have. Measured on this
    /// machine, notify 8 emits a rename *three* ways at once -- `Name(From)`,
    /// `Name(To)` and a synthesised `Name(Both)`, all sharing a tracker id --
    /// while a move out of the directory gives only `From` and a move in only
    /// `To`. Re-reading is right for all five without knowing which it was,
    /// and applying the same event twice changes nothing.
    ///
    /// Returns `false` when it declined, which today means a replacement
    /// listing is already on its way and would undo the work.
    pub fn reconcile(&mut self, read: Vec<(PathBuf, Option<Entry>)>, list: &List) -> bool {
        if self.arriving.is_some() {
            // Applied to entries that are about to be replaced, and the
            // replacement was read before this happened. Neither is right,
            // so say so and let the caller read it again afterwards.
            self.stale = true;
            return false;
        }

        let was = self.remembered();

        for (path, found) in read {
            // A path in another directory is not ours. inotify names the
            // watched directory itself for some events, and that is not a
            // row either.
            if path.parent() != Some(self.path.as_path()) {
                continue;
            }

            let at = self.entries.iter().position(|entry| entry.path == path);

            match (found, at) {
                // Still there: replace, so a changed size or time is picked
                // up -- and so the row moves if the sort is by that.
                (Some(entry), Some(at)) => self.entries[at] = entry,
                (Some(entry), None) => self.entries.push(entry),
                (None, Some(at)) => {
                    self.entries.remove(at);
                }
                (None, None) => {}
            }
        }

        // `entries` is near-sorted -- at most a handful of rows are out of
        // place -- and Rust's stable sort walks existing runs, so this is far
        // cheaper than it looks. What it buys is that `visible`, the cursor
        // and the selection are all rebuilt by name, so none of the index
        // shifting an insert causes has to be reasoned about here.
        self.rebuild_from(list, was);
        true
    }

    /// The listing finished: swap in what arrived, sort it, settle the cursor.
    pub fn finish(&mut self, list: &List) {
        self.listing = Listing::Ready;

        // Remembered against the entries being replaced, not against the
        // replacement -- the selection is indices into the old vector.
        let was = self.remembered();
        if let Some(arriving) = self.arriving.take() {
            self.entries = arriving;
        }

        self.rebuild_from(list, was);
    }

    pub fn fail(&mut self, problem: String) {
        self.listing = Listing::Failed(problem);
        // Including a replacement that was part-collected. A directory that
        // cannot be read has no listing to show, old or new: keeping the last
        // good one would say the files are still there.
        self.arriving = None;
        self.entries.clear();
        self.visible.clear();
        self.cursor = 0;
        self.anchor = 0;
    }

    /// What the cursor and the selection were on, by name.
    ///
    /// Taken before `entries` is touched, because both are indices into it:
    /// once the vector is replaced the numbers point at whatever happens to
    /// be in those slots now.
    fn remembered(&self) -> Remembered {
        Remembered {
            cursor: self
                .placed
                .then(|| self.at(self.cursor).map(|entry| entry.name.clone()))
                .flatten(),
            selected: self
                .selection
                .iter()
                .filter_map(|index| self.entries.get(*index))
                .map(|entry| entry.name.clone())
                .collect(),
        }
    }

    /// Sort and filter, keeping the cursor on the entry it was on.
    ///
    /// By name rather than by index: a relist can insert anything anywhere,
    /// and a cursor that stays on a number rather than on a file jumps around
    /// whenever a directory changes underneath it.
    pub fn rebuild(&mut self, list: &List) {
        let was = self.remembered();
        self.rebuild_from(list, was);
    }

    /// [`Self::rebuild`], with what to look for passed in.
    ///
    /// Apart so a swapped-in listing can be remembered against the entries it
    /// is replacing rather than against itself.
    fn rebuild_from(&mut self, list: &List, was: Remembered) {
        let Remembered {
            cursor: on,
            selected,
        } = was;

        self.entries
            .sort_by(|left, right| entry::compare(left, right, list));

        let filter = Filter::new(&self.filter);

        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.view.show_hidden || !entry.hidden)
            .filter(|(_, entry)| filter.admits(&entry.name))
            .map(|(index, _)| index)
            .collect();

        self.selection = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| selected.contains(&entry.name))
            .map(|(index, _)| index)
            .collect();

        self.cursor = if self.placed {
            on.and_then(|name| {
                self.visible
                    .iter()
                    .position(|index| self.entries[*index].name == name)
            })
            .unwrap_or(self.cursor)
            .min(self.rows().saturating_sub(1))
        } else {
            0
        };
        self.anchor = self.anchor.min(self.rows().saturating_sub(1));
    }

    /// Move the cursor by a number of rows, stopping at either end.
    ///
    /// Clamping rather than wrapping: holding an arrow key at the bottom of a
    /// directory should stop there, not reappear at the top.
    pub fn move_cursor(&mut self, by: isize) {
        self.placed = true;
        if self.visible.is_empty() {
            self.cursor = 0;
            return;
        }

        let last = self.rows() - 1;
        self.cursor = self.cursor.saturating_add_signed(by).min(last);
    }

    pub fn move_to(&mut self, row: usize) {
        self.placed = true;
        self.cursor = row.min(self.rows().saturating_sub(1));
    }

    /// Select exactly the row under the cursor, dropping everything else.
    pub fn select_only(&mut self, row: usize) {
        self.placed = true;
        self.selection.clear();
        if let Some(index) = self.visible.get(row) {
            self.selection.insert(*index);
        }
        self.cursor = row.min(self.rows().saturating_sub(1));
        self.anchor = self.cursor;
    }

    /// Add or remove one row from the selection.
    pub fn toggle(&mut self, row: usize) {
        self.placed = true;
        if let Some(index) = self.visible.get(row).copied()
            && !self.selection.remove(&index)
        {
            self.selection.insert(index);
        }
        self.cursor = row.min(self.rows().saturating_sub(1));
        self.anchor = self.cursor;
    }

    /// Select every row between the anchor and this one.
    pub fn extend_to(&mut self, row: usize) {
        self.placed = true;
        if self.visible.is_empty() {
            self.cursor = 0;
            return;
        }

        let last = self.rows() - 1;
        let row = row.min(last);
        let anchor = self.anchor.min(last);
        let (from, to) = if anchor <= row {
            (anchor, row)
        } else {
            (row, anchor)
        };

        self.selection = self.visible[from..=to].iter().copied().collect();
        self.cursor = row;
    }

    pub fn select_all(&mut self) {
        self.selection = self.visible.iter().copied().collect();
    }

    /// Select what is not selected.
    pub fn invert(&mut self) {
        self.selection = self
            .visible
            .iter()
            .copied()
            .filter(|index| !self.selection.contains(index))
            .collect();
    }

    /// Select the cells a rubber band covered.
    ///
    /// Ranges rather than a list of indices, so a band dragged over a large
    /// directory costs what it selects and not what the directory holds. In
    /// the list layouts `columns` is `None` and the range is whole rows; in
    /// the grid it is a rectangle, and the cells outside it are skipped.
    pub fn select_band(
        &mut self,
        rows: &std::ops::Range<usize>,
        columns: Option<&std::ops::Range<usize>>,
        across: usize,
        add: bool,
    ) {
        if !add {
            self.selection.clear();
        }

        let last = self.visible.len();

        match columns {
            None => {
                let from = rows.start.min(last);
                let to = rows.end.min(last);
                self.selection
                    .extend(self.visible[from..to].iter().copied());
            }
            Some(columns) => {
                for line in rows.clone() {
                    for column in columns.clone() {
                        // `across` may be larger than the real width; the
                        // widget has already clamped `columns` to what was on
                        // screen, so a too-large stride only ends the row.
                        let Some(at) = line
                            .checked_mul(across)
                            .and_then(|base| base.checked_add(column))
                        else {
                            continue;
                        };
                        if let Some(index) = self.visible.get(at) {
                            self.selection.insert(*index);
                        }
                    }
                }
            }
        }
    }

    /// The selected entries, in the order they are shown.
    pub fn selected(&self) -> impl Iterator<Item = &Entry> {
        self.visible
            .iter()
            .filter(|index| self.selection.contains(index))
            .map(|index| &self.entries[*index])
    }

    /// What the status line adds up.
    pub fn selected_size(&self) -> u64 {
        self.selected().map(|entry| entry.size).sum()
    }
}

/// What the filter box means.
///
/// A substring by default, because that is what typing into a box means to
/// most people. A leading `:` switches to a glob, for when it does not.
struct Filter<'a> {
    text: &'a str,
    glob: bool,
    lowered: String,
}

impl<'a> Filter<'a> {
    fn new(text: &'a str) -> Self {
        match text.strip_prefix(':') {
            Some(glob) => Self {
                text: glob,
                glob: true,
                lowered: String::new(),
            },
            None => Self {
                text,
                glob: false,
                lowered: text.to_lowercase(),
            },
        }
    }

    fn admits(&self, name: &str) -> bool {
        if self.text.is_empty() {
            return true;
        }

        if self.glob {
            return crate::open::mime::glob_matches(self.text, name);
        }

        name.to_lowercase().contains(&self.lowered)
    }
}

/// Read named paths on a thread of its own, and say what each is now.
///
/// `None` against a path means it is not there any more. Same reasoning as
/// [`list`]: these are blocking syscalls, and on a stalled mount each one can
/// take as long as it likes without the window noticing.
pub fn examine(paths: BTreeSet<PathBuf>) -> Task<Vec<(PathBuf, Option<Entry>)>> {
    Task::future(async move {
        let (sender, receiver) = tokio::sync::oneshot::channel();

        std::thread::spawn(move || {
            let read = paths
                .into_iter()
                .map(|path| {
                    let found = Entry::read(path.clone()).ok();
                    (path, found)
                })
                .collect();
            // The receiver going means the buffer was closed or relisted, and
            // there is nobody left to tell.
            let _ = sender.send(read);
        });

        receiver.await.unwrap_or_default()
    })
}

/// Read a directory on a thread of its own, a chunk at a time.
///
/// A thread rather than an async task: `read_dir` and `stat` are blocking
/// syscalls with nothing to overlap, and a network mount that stalls would
/// otherwise stall whatever else shared the executor.
pub fn list(path: PathBuf) -> Task<Update> {
    Task::run(
        iced::stream::channel(4, async move |mut output| {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(4);

            std::thread::spawn(move || read(&path, &sender));

            while let Some(update) = receiver.recv().await {
                // The receiver going means the buffer was closed or relisted,
                // and there is nobody left to tell.
                if output.send(update).await.is_err() {
                    return;
                }
            }
        }),
        |update| update,
    )
}

/// The listing thread itself.
fn read(path: &Path, sender: &tokio::sync::mpsc::Sender<Update>) {
    let directory = match std::fs::read_dir(path) {
        Ok(directory) => directory,
        Err(error) => {
            let _ = sender.blocking_send(Update::Failed(error.to_string()));
            return;
        }
    };

    let mut chunk = Vec::with_capacity(CHUNK);

    for found in directory {
        let Ok(found) = found else { continue };

        // An entry that cannot be read is skipped rather than failing the
        // listing: a file can be deleted between `read_dir` and the `stat`,
        // and one racing file should not empty the window.
        if let Ok(entry) = Entry::read(found.path()) {
            chunk.push(entry);
        }

        if chunk.len() >= CHUNK {
            let full = std::mem::replace(&mut chunk, Vec::with_capacity(CHUNK));
            if sender.blocking_send(Update::Entries(full)).is_err() {
                return;
            }
        }
    }

    if !chunk.is_empty() && sender.blocking_send(Update::Entries(chunk)).is_err() {
        return;
    }

    let _ = sender.blocking_send(Update::Done);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Kind;

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.to_owned(),
            path: PathBuf::from(name),
            kind: Kind::File,
            size: 1,
            modified: None,
            mode: 0o644,
            target: None,
            hidden: name.starts_with('.'),
        }
    }

    fn buffer(names: &[&str]) -> Buffer {
        let mut buffer = Buffer::new(PathBuf::from("/tmp"), List::default().view());
        buffer.extend(
            names.iter().map(|name| entry(name)).collect(),
            &List::default(),
        );
        buffer.finish(&List::default());
        buffer
    }

    /// A scratch directory of this test's own, so two tests never collide.
    fn scratch(name: &str) -> PathBuf {
        crate::testing::scratch(&format!("reconcile-{name}"))
    }

    /// What the worker hands back for a set of paths: each one, and what it
    /// is now. This is [`examine`] without the thread.
    fn examined<I: IntoIterator<Item = PathBuf>>(paths: I) -> Vec<(PathBuf, Option<Entry>)> {
        paths
            .into_iter()
            .map(|path| {
                let found = Entry::read(path.clone()).ok();
                (path, found)
            })
            .collect()
    }

    /// The same, for an event naming one path.
    fn one(path: PathBuf) -> Vec<(PathBuf, Option<Entry>)> {
        examined([path])
    }

    /// Listed from a real directory, so `reconcile` has something to `stat`.
    fn listed(root: &Path, names: &[&str]) -> Buffer {
        let list = List::default();
        let mut buffer = Buffer::new(root.to_path_buf(), list.view());
        let entries = names
            .iter()
            .map(|name| Entry::read(root.join(name)).expect("the file was just made"))
            .collect();
        buffer.extend(entries, &list);
        buffer.finish(&list);
        buffer
    }

    /// One `stat` per named path, and the answer decides. Create, remove and
    /// a metadata change all go the same way, without pairing anything.
    #[test]
    fn reconcile_reads_the_named_paths_and_nothing_else() {
        let root = scratch("basics");
        std::fs::write(root.join("stays"), b"x").expect("write");
        std::fs::write(root.join("goes"), b"x").expect("write");

        let list = List::default();
        let mut buffer = listed(&root, &["stays", "goes"]);
        assert_eq!(buffer.rows(), 2);

        // The directory changes underneath: one arrives, one leaves, one
        // grows.
        std::fs::write(root.join("arrives"), b"x").expect("write");
        std::fs::remove_file(root.join("goes")).expect("remove");
        std::fs::write(root.join("stays"), b"much longer than before").expect("write");

        let touched: BTreeSet<PathBuf> = ["arrives", "goes", "stays"]
            .iter()
            .map(|name| root.join(name))
            .collect();
        assert!(buffer.reconcile(examined(touched), &list));

        let shown: Vec<&str> = buffer.shown().map(|e| e.name.as_str()).collect();
        assert_eq!(shown, ["arrives", "stays"]);
        assert_eq!(
            buffer.at(1).map(|e| e.size),
            Some(23),
            "the size should have been re-read"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// notify 8 emits a rename three ways at once -- `From`, `To` and a
    /// synthesised `Both`, all with one tracker id -- measured on this
    /// machine. Re-reading is right for all of them, and applying the same
    /// event twice has to change nothing.
    #[test]
    fn reconcile_handles_a_rename_however_it_is_reported() {
        let root = scratch("rename");
        std::fs::write(root.join("before"), b"x").expect("write");

        let list = List::default();
        let mut buffer = listed(&root, &["before"]);
        std::fs::rename(root.join("before"), root.join("after")).expect("rename");

        // Both paths in one set, which is what `Name(Both)` gives.
        let both: BTreeSet<PathBuf> = [root.join("before"), root.join("after")]
            .into_iter()
            .collect();
        buffer.reconcile(examined(both), &list);
        let shown: Vec<&str> = buffer.shown().map(|e| e.name.as_str()).collect();
        assert_eq!(shown, ["after"]);

        // The `From` and `To` events for the same rename arrive as well.
        // Applying them after the fact must not duplicate the row or lose it.
        buffer.reconcile(one(root.join("before")), &list);
        buffer.reconcile(one(root.join("after")), &list);
        let shown: Vec<&str> = buffer.shown().map(|e| e.name.as_str()).collect();
        assert_eq!(shown, ["after"], "applying it again changed nothing");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A path in another directory is not a row here. inotify names the
    /// watched directory itself for some events, and that is not one either.
    #[test]
    fn reconcile_ignores_paths_that_are_not_in_this_directory() {
        let root = scratch("elsewhere");
        std::fs::write(root.join("mine"), b"x").expect("write");

        let list = List::default();
        let mut buffer = listed(&root, &["mine"]);

        let strangers: BTreeSet<PathBuf> = [
            root.clone(),
            PathBuf::from("/etc/hostname"),
            root.join("deeper").join("nested"),
        ]
        .into_iter()
        .collect();
        buffer.reconcile(examined(strangers), &list);

        let shown: Vec<&str> = buffer.shown().map(|e| e.name.as_str()).collect();
        assert_eq!(shown, ["mine"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The cursor and the selection are indices into `entries`, and an insert
    /// shifts everything after it. Reconcile has to leave both on the files
    /// they were on, not on the numbers.
    #[test]
    fn reconcile_keeps_the_cursor_and_the_selection() {
        let root = scratch("cursor");
        for name in ["b", "c", "d"] {
            std::fs::write(root.join(name), b"x").expect("write");
        }

        let list = List::default();
        let mut buffer = listed(&root, &["b", "c", "d"]);
        buffer.toggle(2); // select `d`
        buffer.move_to(1); // cursor on `c`

        // `a` sorts before all of them, so every index shifts by one.
        std::fs::write(root.join("a"), b"x").expect("write");
        buffer.reconcile(one(root.join("a")), &list);

        assert_eq!(
            buffer.at(buffer.cursor).map(|e| e.name.as_str()),
            Some("c"),
            "the cursor should still be on its file"
        );
        let picked: Vec<&str> = buffer.selected().map(|e| e.name.as_str()).collect();
        assert_eq!(picked, ["d"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A replacement listing is already on its way, so touching the entries
    /// now would be undone by the swap -- and worse, would be applied to the
    /// listing being replaced.
    #[test]
    fn reconcile_declines_while_a_relist_is_in_flight() {
        let root = scratch("inflight");
        std::fs::write(root.join("one"), b"x").expect("write");

        let list = List::default();
        let mut buffer = listed(&root, &["one"]);
        buffer.start_arriving();

        std::fs::write(root.join("two"), b"x").expect("write");
        assert!(!buffer.reconcile(one(root.join("two")), &list));
        assert_eq!(buffer.rows(), 1, "and it left the listing alone");

        // Declining is not dropping. The replacement was read before this
        // happened, so it is wrong too, and somebody has to be told to read
        // the directory again -- otherwise the change is lost until F5.
        assert!(buffer.take_stale(), "it should have said so");
        assert!(!buffer.take_stale(), "and only once");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A relist keeps the old listing up until the new one is complete.
    /// Clearing first blanks the tile for as long as the read takes, which on
    /// a directory of 100,000 reads as the program losing the files.
    #[test]
    fn a_relist_shows_the_old_listing_until_the_new_one_lands() {
        let list = List::default();
        let mut buffer = buffer(&["a", "b", "c"]);
        assert_eq!(buffer.rows(), 3);

        buffer.start_arriving();
        assert!(buffer.refreshing(), "and it says so");
        assert_eq!(buffer.rows(), 3, "still the old three");

        // Chunks land out of sight.
        buffer.extend(vec![entry("x")], &list);
        assert_eq!(buffer.rows(), 3, "still the old three");
        buffer.extend(vec![entry("y")], &list);
        assert_eq!(buffer.rows(), 3, "still the old three");

        buffer.finish(&list);
        assert!(!buffer.refreshing());
        let shown: Vec<&str> = buffer.shown().map(|e| e.name.as_str()).collect();
        assert_eq!(shown, ["x", "y"], "swapped all at once");
    }

    /// The cursor and the selection are indices into `entries`, so a swap has
    /// to remember them against the vector being replaced. Reading them after
    /// the swap points at whatever landed in those slots.
    #[test]
    fn a_swap_keeps_the_cursor_and_the_selection_by_name() {
        let list = List::default();
        let mut buffer = buffer(&["one", "two", "three"]);

        // Sorted, that is: one, three, two. Select `three`, then put the
        // cursor on `two` -- in that order, because `toggle` moves the cursor
        // to the row it toggles.
        buffer.toggle(1);
        buffer.move_to(2);
        assert_eq!(
            buffer.at(buffer.cursor).map(|e| e.name.as_str()),
            Some("two")
        );

        // The directory changed: `one` is gone, `four` arrived.
        buffer.start_arriving();
        buffer.extend(vec![entry("two"), entry("three"), entry("four")], &list);
        buffer.finish(&list);

        assert_eq!(
            buffer.at(buffer.cursor).map(|e| e.name.as_str()),
            Some("two"),
            "the cursor followed its file"
        );
        let picked: Vec<&str> = buffer.selected().map(|e| e.name.as_str()).collect();
        assert_eq!(picked, ["three"], "and so did the selection");
    }

    /// A directory that cannot be read has no listing to show, old or new.
    /// Keeping the last good one would say the files are still there.
    #[test]
    fn a_failed_relist_drops_what_was_showing() {
        let mut buffer = buffer(&["a", "b"]);
        buffer.start_arriving();
        buffer.extend(vec![entry("c")], &List::default());

        buffer.fail(String::from("permission denied"));

        assert_eq!(buffer.rows(), 0);
        assert!(
            !buffer.refreshing(),
            "and the part-read replacement went too"
        );
    }

    /// Hidden files leave the visible rows but stay in `entries`, so turning
    /// them on does not need another listing.
    #[test]
    fn hidden_entries_leave_the_rows_but_not_the_buffer() {
        let mut buffer = buffer(&["visible", ".hidden"]);
        assert_eq!(buffer.rows(), 1);

        // The buffer's own flag, not the config's: the config only says what
        // a buffer starts as.
        buffer.view.show_hidden = true;
        buffer.rebuild(&List::default());
        assert_eq!(buffer.rows(), 2);
    }

    /// A relist can insert anything anywhere. A cursor that remembers a number
    /// rather than a file jumps whenever a directory changes underneath it.
    #[test]
    fn the_cursor_stays_on_its_entry_across_a_relist() {
        let mut buffer = buffer(&["b", "c"]);
        buffer.move_to(1);
        assert_eq!(buffer.at(buffer.cursor).map(|e| e.name.as_str()), Some("c"));

        buffer.extend(vec![entry("a")], &List::default());
        assert_eq!(buffer.at(buffer.cursor).map(|e| e.name.as_str()), Some("c"));
    }

    /// Selection survives a resort for the same reason the cursor does.
    #[test]
    fn selection_survives_a_resort() {
        let mut buffer = buffer(&["b", "c"]);
        buffer.select_only(1);

        buffer.extend(vec![entry("a")], &List::default());
        let selected: Vec<&str> = buffer.selected().map(|e| e.name.as_str()).collect();
        assert_eq!(selected, ["c"]);
    }

    /// A listing arrives in chunks, and each one is sorted as it lands. If the
    /// cursor followed a name from the very first chunk it would latch onto
    /// whichever entry happened to sort first among the first 512, then be
    /// dragged down the list as the rest arrived. A directory of 100k files
    /// opened with the cursor on `file17` before this was fixed.
    #[test]
    fn an_untouched_cursor_stays_at_the_top_while_chunks_arrive() {
        let list = List::default();
        let mut buffer = Buffer::new(PathBuf::from("/tmp"), List::default().view());

        buffer.extend(vec![entry("m"), entry("n")], &list);
        assert_eq!(buffer.cursor, 0);

        buffer.extend(vec![entry("a"), entry("b")], &list);
        buffer.finish(&list);

        assert_eq!(buffer.cursor, 0);
        assert_eq!(buffer.at(0).map(|e| e.name.as_str()), Some("a"));
    }

    /// Once someone has put the cursor somewhere, it must follow that entry
    /// rather than that row number.
    #[test]
    fn a_placed_cursor_follows_its_entry() {
        let list = List::default();
        let mut buffer = buffer(&["m", "n"]);

        buffer.move_to(0);
        buffer.extend(vec![entry("a"), entry("b")], &list);

        assert_eq!(buffer.at(buffer.cursor).map(|e| e.name.as_str()), Some("m"));
    }

    /// A filter narrows what is shown without touching what is selected, so
    /// clearing it brings the selection back rather than losing it.
    #[test]
    fn a_filter_narrows_the_view_and_keeps_the_selection() {
        let list = List::default();
        let mut buffer = buffer(&["alpha.txt", "beta.txt", "gamma.md"]);

        buffer.select_all();
        assert_eq!(buffer.selected().count(), 3);

        buffer.filter = String::from("a.txt");
        buffer.rebuild(&list);
        assert_eq!(buffer.rows(), 2, "alpha.txt and beta.txt");

        buffer.filter.clear();
        buffer.rebuild(&list);
        assert_eq!(buffer.rows(), 3);
        assert_eq!(buffer.selected().count(), 3, "the selection came back");
    }

    /// A filter belongs to the listing in front of you. Carrying it into the
    /// next directory shows a few of its files and nothing saying why the
    /// rest are missing -- which is what "filter, then open a folder" did.
    #[test]
    fn going_somewhere_else_drops_the_filter() {
        let list = List::default();
        let mut buffer = buffer(&["alpha.txt", "beta.txt", "gamma.md"]);

        buffer.filter = String::from("a.txt");
        buffer.filtering = true;
        buffer.rebuild(&list);
        assert_eq!(buffer.rows(), 2);

        buffer.leaving(&list);

        assert!(buffer.filter.is_empty());
        assert!(!buffer.filtering, "and the box goes with it");
        assert_eq!(buffer.rows(), 3, "everything is showing again");
    }

    /// Typing into a box means a substring to most people; a leading colon is
    /// how to ask for a glob instead.
    #[test]
    fn a_leading_colon_means_a_glob() {
        let list = List::default();
        let mut buffer = buffer(&["notes.md", "notes.txt", "read.md"]);

        buffer.filter = String::from(":*.md");
        buffer.rebuild(&list);
        assert_eq!(buffer.rows(), 2);

        // The same text without the colon is a substring, and matches nothing
        // because no name contains the literal `*.md`.
        buffer.filter = String::from("*.md");
        buffer.rebuild(&list);
        assert_eq!(buffer.rows(), 0);
    }

    /// A filter is case-insensitive: nobody typing `readme` means to exclude
    /// `README`.
    #[test]
    fn a_filter_ignores_case() {
        let list = List::default();
        let mut buffer = buffer(&["README", "readme.txt", "other"]);

        buffer.filter = String::from("READ");
        buffer.rebuild(&list);
        assert_eq!(buffer.rows(), 2);
    }

    /// A filter that matches nothing leaves an empty list that is still
    /// navigable, rather than a cursor pointing past the end.
    #[test]
    fn a_filter_matching_nothing_is_survivable() {
        let list = List::default();
        let mut buffer = buffer(&["a", "b", "c"]);
        buffer.move_to(2);

        buffer.filter = String::from("nothing matches this");
        buffer.rebuild(&list);

        assert_eq!(buffer.rows(), 0);
        assert_eq!(buffer.cursor, 0);
        buffer.move_cursor(1);
        assert_eq!(buffer.cursor, 0);
    }

    /// A band over whole rows selects the run it covered and nothing else.
    #[test]
    fn a_band_selects_the_rows_it_covered() {
        let mut buffer = buffer(&["a", "b", "c", "d", "e"]);

        buffer.select_band(&(1..4), None, 1, false);
        let selected: Vec<&str> = buffer.selected().map(|e| e.name.as_str()).collect();
        assert_eq!(selected, ["b", "c", "d"]);
    }

    /// A band that runs off the end of the listing clamps rather than
    /// panicking. Dragging past the last row is the normal way to select to
    /// the bottom.
    #[test]
    fn a_band_past_the_end_is_clamped() {
        let mut buffer = buffer(&["a", "b"]);

        buffer.select_band(&(0..999), None, 1, false);
        assert_eq!(buffer.selected().count(), 2);

        buffer.select_band(&(500..999), None, 1, false);
        assert_eq!(buffer.selected().count(), 0);
    }

    /// In the grid a band is a rectangle, so the cells outside its columns
    /// are left alone even though they lie between its first and last index.
    #[test]
    fn a_band_in_a_grid_is_a_rectangle() {
        // Three across: a b c / d e f / g h i
        let mut buffer = buffer(&["a", "b", "c", "d", "e", "f", "g", "h", "i"]);

        // The first two columns of the first two rows.
        buffer.select_band(&(0..2), Some(&(0..2)), 3, false);
        let selected: Vec<&str> = buffer.selected().map(|e| e.name.as_str()).collect();
        assert_eq!(selected, ["a", "b", "d", "e"], "not c, and not f");
    }

    /// Holding Ctrl adds to what is already selected rather than replacing
    /// it, so two bands can be drawn in different places.
    #[test]
    fn a_band_can_add_to_a_selection() {
        let mut buffer = buffer(&["a", "b", "c", "d"]);

        buffer.select_band(&(0..1), None, 1, false);
        buffer.select_band(&(3..4), None, 1, true);

        let selected: Vec<&str> = buffer.selected().map(|e| e.name.as_str()).collect();
        assert_eq!(selected, ["a", "d"]);
    }

    /// Inverting twice is the same as not inverting, and inverting nothing
    /// selects everything.
    #[test]
    fn inverting_turns_the_selection_over() {
        let mut buffer = buffer(&["a", "b", "c"]);

        buffer.invert();
        assert_eq!(buffer.selected().count(), 3);

        buffer.invert();
        assert_eq!(buffer.selected().count(), 0);

        buffer.select_only(1);
        buffer.invert();
        let selected: Vec<&str> = buffer.selected().map(|e| e.name.as_str()).collect();
        assert_eq!(selected, ["a", "c"]);
    }

    /// A filter hides rows, and inverting must not select something that is
    /// not on screen. Somebody who inverts a filtered listing means the rows
    /// they can see.
    #[test]
    fn inverting_only_reaches_what_is_shown() {
        let list = List::default();
        let mut buffer = buffer(&["keep-a", "keep-b", "other"]);

        buffer.filter = String::from("keep");
        buffer.rebuild(&list);
        buffer.select_only(0);
        buffer.invert();

        let selected: Vec<&str> = buffer.selected().map(|e| e.name.as_str()).collect();
        assert_eq!(selected, ["keep-b"], "not the hidden one");
    }

    /// Holding an arrow key at the end of a directory should stop there, not
    /// reappear at the other end.
    #[test]
    fn the_cursor_clamps_rather_than_wrapping() {
        let mut buffer = buffer(&["a", "b", "c"]);

        buffer.move_cursor(100);
        assert_eq!(buffer.cursor, 2);

        buffer.move_cursor(-100);
        assert_eq!(buffer.cursor, 0);
    }

    /// An empty directory still has to be navigable without panicking.
    #[test]
    fn an_empty_buffer_survives_every_movement() {
        let mut buffer = buffer(&[]);

        buffer.move_cursor(1);
        buffer.move_cursor(-1);
        buffer.move_to(50);
        buffer.select_only(50);
        buffer.toggle(50);
        buffer.extend_to(50);
        buffer.select_all();

        assert_eq!(buffer.cursor, 0);
        assert_eq!(buffer.rows(), 0);
        assert_eq!(buffer.selected().count(), 0);
    }

    /// Shift-click selects the whole run between the anchor and the click,
    /// in both directions.
    #[test]
    fn extending_works_in_both_directions() {
        let mut buffer = buffer(&["a", "b", "c", "d"]);

        buffer.select_only(2);
        buffer.extend_to(0);
        let selected: Vec<&str> = buffer.selected().map(|e| e.name.as_str()).collect();
        assert_eq!(selected, ["a", "b", "c"]);

        buffer.select_only(1);
        buffer.extend_to(3);
        let selected: Vec<&str> = buffer.selected().map(|e| e.name.as_str()).collect();
        assert_eq!(selected, ["b", "c", "d"]);
    }

    /// Ctrl-click adds and then removes the same row.
    #[test]
    fn toggling_adds_and_removes() {
        let mut buffer = buffer(&["a", "b"]);

        buffer.toggle(0);
        assert_eq!(buffer.selected().count(), 1);
        buffer.toggle(0);
        assert_eq!(buffer.selected().count(), 0);
    }

    /// The status line adds up what is selected, not what is listed.
    #[test]
    fn selected_size_counts_only_the_selection() {
        let mut buffer = buffer(&["a", "b", "c"]);
        assert_eq!(buffer.selected_size(), 0);

        buffer.select_all();
        assert_eq!(buffer.selected_size(), 3);
    }
}
