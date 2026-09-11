//! Noticing that a directory changed underneath us.
//!
//! Three things make this less obvious than it looks.
//!
//! **Watch the directory, never the file.** Editors and `mv` replace a file
//! rather than writing it, so a watch on an inode misses the change entirely
//! -- the inode is still there, unchanged, and nothing points at it any more.
//!
//! **Coalesce.** Extracting an archive is thousands of events in a second,
//! and one relist answers all of them. The debounce turns a storm into a
//! single message.
//!
//! **Only what is on screen.** inotify allows 110313 watches and 128
//! instances on this machine, and a long session opens a lot of directories.
//! A buffer nobody is looking at relists when it becomes visible instead.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use iced::Subscription;
use iced::futures::SinkExt;
use notify::{RecursiveMode, Watcher};

/// How long to wait for a storm to end before relisting.
///
/// Long enough that unpacking a tarball is one relist rather than hundreds,
/// short enough that saving a file in an editor feels immediate.
const SETTLE: Duration = Duration::from_millis(120);

/// How many touched paths are worth re-reading one at a time.
///
/// Past this, reading the whole directory again is cheaper than the stats and
/// bounds the memory the set can take. Unpacking a tarball goes this way.
const TOO_MANY: usize = 512;

/// What a settled burst of events amounts to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// These paths were mentioned. Re-read each and reconcile.
    Touched(BTreeSet<PathBuf>),
    /// Read the directory again from scratch: the kernel dropped events, or
    /// so many arrived that one at a time is the slower way.
    Rescan,
}

/// Whether an event could have changed what a listing shows.
///
/// Reading a directory is not a change to it, and saying otherwise costs more
/// than a wasted relist: inotify reports a *read* as `Access(Open)`, so a
/// relist opens the directory, the watch fires, and it relists again. With one
/// tile that loop never starts, because nothing reads the directory after the
/// first listing. With two tiles on one directory each one's read wakes the
/// other's watch, and they feed each other forever -- which is what the
/// flickering after a split was, measured at 33 events in four seconds.
fn changes_a_listing(kind: notify::EventKind) -> bool {
    use notify::EventKind;

    !matches!(kind, EventKind::Access(_))
        && !matches!(kind, EventKind::Other)
        && kind != EventKind::Any
}

/// Watch one directory, and say when it has settled after a change.
///
/// The subscription is keyed on the path *and* a generation, so navigating
/// away tears the old watch down. Without the generation iced recognises a
/// subscription it is already running and keeps the old stream -- which is
/// watching the directory that was left. ricebar's `app::subscription` has
/// the same note, and it cost real time there.
pub fn directory(path: PathBuf, generation: u64) -> Subscription<Change> {
    Subscription::run_with((path, generation), |(path, _)| {
        let path = path.clone();

        iced::stream::channel(1, async move |mut output| {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(1024);

            // Set when an event could not be queued, and read once the burst
            // settles. It cannot be a message, because the way it is found
            // out is that a message would not fit: an earlier version sent
            // `Rescan` down the same full channel, which failed in exactly
            // the same way and lost the fact silently. A directory of 900
            // new files then settled at 218 rows and stayed there.
            //
            // Genuinely shared between notify's thread and this task, which
            // is what an atomic is for -- and the only shared mutable state
            // in ricedir.
            let lost = std::sync::Arc::new(AtomicBool::new(false));
            let losing = std::sync::Arc::clone(&lost);

            // The watcher must outlive this scope or the watch is dropped with
            // it, which reads as "no directory ever changes".
            let watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    let Ok(event) = event else { return };

                    // The kernel's queue filled and it threw events away.
                    // notify says so as `Other` with `Rescan` set -- which the
                    // filter below drops, so this has to be looked at first.
                    // Missing it is how a listing quietly stops matching the
                    // disk, which is the fault nobody can reproduce.
                    if event.need_rescan() {
                        losing.store(true, Ordering::Relaxed);
                        let _ = sender.try_send(Change::Touched(BTreeSet::new()));
                        return;
                    }

                    if !changes_a_listing(event.kind) {
                        return;
                    }

                    // Never blocks: this is notify's own thread, and holding
                    // it up backs the kernel's queue into a real overflow.
                    // A send that does not fit sets the flag instead, and the
                    // burst settles into a rescan.
                    let touched = Change::Touched(event.paths.into_iter().collect());
                    if sender.try_send(touched).is_err() {
                        losing.store(true, Ordering::Relaxed);
                    }
                });

            let Ok(mut watcher) = watcher else {
                eprintln!("ricedir: cannot watch {}", path.display());
                return;
            };

            if let Err(error) = watcher.watch(&path, RecursiveMode::NonRecursive) {
                // A directory that cannot be watched still lists; it just does
                // not refresh. Worth one line, not a dialogue.
                eprintln!("ricedir: cannot watch {}: {error}", path.display());
                return;
            }

            while let Some(first) = receiver.recv().await {
                // Gather the burst rather than throwing it away. A path
                // touched fifty times is one entry in the set and so one
                // `stat` later, which is what makes the debounce worth
                // having twice over.
                let mut touched = BTreeSet::new();
                let mut rescan = false;
                let mut take = |change: Change| match change {
                    Change::Rescan => rescan = true,
                    Change::Touched(paths) => touched.extend(paths),
                };
                take(first);

                loop {
                    tokio::time::sleep(SETTLE).await;
                    let mut quiet = true;
                    while let Ok(change) = receiver.try_recv() {
                        take(change);
                        quiet = false;
                    }
                    if quiet {
                        break;
                    }
                }

                // Past a point, reading the directory again beats stat-ing
                // every path in it one at a time -- and so does having been
                // told that some events never made it here at all.
                let rescan = rescan || lost.swap(false, Ordering::Relaxed);
                let settled = if rescan || touched.len() > TOO_MANY {
                    Change::Rescan
                } else {
                    Change::Touched(touched)
                };

                if output.send(settled).await.is_err() {
                    return;
                }
            }

            drop(watcher);
        })
    })
}
