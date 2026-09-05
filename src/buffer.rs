//! One open directory: what is in it, where the cursor is, what is selected.
//!
//! A buffer outlives the tile showing it, so closing a tile costs nothing and
//! reopening the same directory is instant. Two tiles on one directory share
//! one buffer, and so one listing and one watcher.

use std::collections::HashSet;
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

/// An open directory.
#[derive(Debug)]
pub struct Buffer {
    pub path: PathBuf,
    pub listing: Listing,

    /// Every entry, sorted. Indices into this are what selection remembers.
    entries: Vec<Entry>,
    /// Indices into `entries` that the filter lets through, in shown order.
    /// The list widget draws this, so a filter never disturbs a selection.
    view: Vec<usize>,

    /// Where the keyboard cursor is, as a position in `view`.
    pub cursor: usize,
    /// Whether anyone has actually put the cursor somewhere.
    ///
    /// Until they have, a relist leaves it at the top. Following it by name
    /// from the start would latch it onto whichever entry happened to sort
    /// first in the first chunk to arrive, and then drag it down the listing
    /// as the rest came in -- which is how a directory of 100k files opened
    /// with the cursor on `file17`.
    placed: bool,
    /// Where a shift-extended selection started, as a position in `view`.
    pub anchor: usize,
    /// Selected entries, by index into `entries`.
    pub selection: HashSet<usize>,

    /// Directories already visited, for back and forward.
    pub history: Vec<PathBuf>,
    pub future: Vec<PathBuf>,

    /// Bumped on every relist, so a chunk from a listing that has been
    /// replaced can be recognised and dropped.
    pub generation: u64,

    /// When the entries were last sorted, so a streaming listing is not
    /// re-sorted once per chunk. `None` until the first chunk arrives.
    sorted: Option<Instant>,
}

impl Buffer {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            listing: Listing::Loading,
            entries: Vec::new(),
            view: Vec::new(),
            cursor: 0,
            placed: false,
            anchor: 0,
            selection: HashSet::new(),
            history: Vec::new(),
            future: Vec::new(),
            generation: 0,
            sorted: None,
        }
    }

    /// The entries the list should draw, in order.
    pub fn shown(&self) -> impl ExactSizeIterator<Item = &Entry> {
        self.view.iter().map(|index| &self.entries[*index])
    }

    /// How many rows the list has.
    pub fn rows(&self) -> usize {
        self.view.len()
    }

    /// The entry at a row, if there is one.
    pub fn at(&self, row: usize) -> Option<&Entry> {
        self.view.get(row).map(|index| &self.entries[*index])
    }

    /// Whether the entry at a row is selected.
    pub fn is_selected(&self, row: usize) -> bool {
        self.view
            .get(row)
            .is_some_and(|index| self.selection.contains(index))
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
        self.entries.extend(entries);

        let due = self
            .sorted
            .is_none_or(|last| last.elapsed() >= Duration::from_millis(100));

        if due {
            self.sorted = Some(Instant::now());
            self.rebuild(list);
        }
    }

    /// The listing finished: sort what arrived and settle the cursor.
    pub fn finish(&mut self, list: &List) {
        self.listing = Listing::Ready;
        self.rebuild(list);
    }

    pub fn fail(&mut self, problem: String) {
        self.listing = Listing::Failed(problem);
        self.entries.clear();
        self.view.clear();
        self.cursor = 0;
        self.anchor = 0;
    }

    /// Sort and filter, keeping the cursor on the entry it was on.
    ///
    /// By name rather than by index: a relist can insert anything anywhere,
    /// and a cursor that stays on a number rather than on a file jumps around
    /// whenever a directory changes underneath it.
    pub fn rebuild(&mut self, list: &List) {
        let on = self
            .placed
            .then(|| self.at(self.cursor).map(|entry| entry.name.clone()))
            .flatten();
        let selected: HashSet<String> = self
            .selection
            .iter()
            .filter_map(|index| self.entries.get(*index))
            .map(|entry| entry.name.clone())
            .collect();

        self.entries
            .sort_by(|left, right| entry::compare(left, right, list));

        self.view = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| list.show_hidden || !entry.hidden)
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
                self.view
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
        if self.view.is_empty() {
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
        if let Some(index) = self.view.get(row) {
            self.selection.insert(*index);
        }
        self.cursor = row.min(self.rows().saturating_sub(1));
        self.anchor = self.cursor;
    }

    /// Add or remove one row from the selection.
    pub fn toggle(&mut self, row: usize) {
        self.placed = true;
        if let Some(index) = self.view.get(row).copied()
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
        if self.view.is_empty() {
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

        self.selection = self.view[from..=to].iter().copied().collect();
        self.cursor = row;
    }

    pub fn select_all(&mut self) {
        self.selection = self.view.iter().copied().collect();
    }

    /// The selected entries, in the order they are shown.
    pub fn selected(&self) -> impl Iterator<Item = &Entry> {
        self.view
            .iter()
            .filter(|index| self.selection.contains(index))
            .map(|index| &self.entries[*index])
    }

    /// What the status line adds up.
    pub fn selected_size(&self) -> u64 {
        self.selected().map(|entry| entry.size).sum()
    }
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
        let mut buffer = Buffer::new(PathBuf::from("/tmp"));
        buffer.extend(
            names.iter().map(|name| entry(name)).collect(),
            &List::default(),
        );
        buffer.finish(&List::default());
        buffer
    }

    /// Hidden files are filtered out of the view but stay in `entries`, so
    /// toggling the setting does not need another listing.
    #[test]
    fn hidden_entries_leave_the_view_but_not_the_buffer() {
        let mut buffer = buffer(&["visible", ".hidden"]);
        assert_eq!(buffer.rows(), 1);

        let showing = List {
            show_hidden: true,
            ..List::default()
        };
        buffer.rebuild(&showing);
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
        let mut buffer = Buffer::new(PathBuf::from("/tmp"));

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
