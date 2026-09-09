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

use std::path::PathBuf;
use std::time::Duration;

use iced::Subscription;
use iced::futures::SinkExt;
use notify::{RecursiveMode, Watcher};

/// How long to wait for a storm to end before relisting.
///
/// Long enough that unpacking a tarball is one relist rather than hundreds,
/// short enough that saving a file in an editor feels immediate.
const SETTLE: Duration = Duration::from_millis(120);

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
pub fn directory(path: PathBuf, generation: u64) -> Subscription<()> {
    Subscription::run_with((path, generation), |(path, _)| {
        let path = path.clone();

        iced::stream::channel(1, async move |mut output| {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(64);

            // The watcher must outlive this scope or the watch is dropped with
            // it, which reads as "no directory ever changes".
            let watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    let Ok(event) = event else { return };

                    if !changes_a_listing(event.kind) {
                        return;
                    }

                    // Blocking, from notify's own thread. A full channel means
                    // the loop below is already behind on relisting, and the
                    // event it would have queued asks for the same thing.
                    let _ = sender.try_send(());
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

            while receiver.recv().await.is_some() {
                // Swallow whatever else arrives while the dust settles.
                loop {
                    tokio::time::sleep(SETTLE).await;
                    if receiver.try_recv().is_err() {
                        break;
                    }
                }

                if output.send(()).await.is_err() {
                    return;
                }
            }

            drop(watcher);
        })
    })
}
