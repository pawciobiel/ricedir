//! One directory entry, and the order they are shown in.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::config::{List, Sort};

/// What an entry is, as far as the list needs to know.
///
/// A symlink keeps its own variant rather than being resolved to what it
/// points at: a broken one must still be listed, and following one blindly is
/// how a recursive walk ends up somewhere it was never asked to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Directory,
    File,
    /// A symlink, and whether its target exists and is a directory.
    Link {
        directory: bool,
        broken: bool,
    },
    /// A socket, fifo, block or character device.
    Other,
}

impl Kind {
    /// Whether entering this should list a directory.
    pub fn is_directory(self) -> bool {
        matches!(
            self,
            Self::Directory
                | Self::Link {
                    directory: true,
                    ..
                }
        )
    }
}

/// One entry, as read once when the directory was listed.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub mode: u32,
    /// Where a symlink points, unresolved, for the detail layout to show.
    pub target: Option<PathBuf>,
    pub hidden: bool,
}

impl Entry {
    /// Read one entry from a path, without following it.
    ///
    /// `symlink_metadata` rather than `metadata`, so a broken symlink is an
    /// entry with a kind rather than an error that makes it disappear from a
    /// listing it is genuinely part of.
    pub fn read(path: PathBuf) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt;

        let metadata = std::fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();

        let name = path.file_name().map_or_else(
            || path.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        );

        let (kind, target) = if file_type.is_symlink() {
            // One `stat` through the link says both whether it is broken and
            // whether entering it would list a directory.
            let resolved = std::fs::metadata(&path);
            let kind = Kind::Link {
                directory: resolved.as_ref().is_ok_and(|found| found.is_dir()),
                broken: resolved.is_err(),
            };
            (kind, std::fs::read_link(&path).ok())
        } else if file_type.is_dir() {
            (Kind::Directory, None)
        } else if file_type.is_file() {
            (Kind::File, None)
        } else {
            (Kind::Other, None)
        };

        Ok(Self {
            hidden: name.starts_with('.'),
            name,
            path,
            kind,
            size: metadata.size(),
            modified: metadata.modified().ok(),
            mode: metadata.mode(),
            target,
        })
    }

    /// The part after the last dot, lowercased, for sorting and for the type
    /// column. A leading dot is a hidden file rather than an extension, so
    /// `.bashrc` has none.
    pub fn extension(&self) -> &str {
        self.name
            .rfind('.')
            .filter(|at| *at > 0)
            .map_or("", |at| &self.name[at + 1..])
    }
}

/// Order two entries the way the config asks for.
pub fn compare(left: &Entry, right: &Entry, list: &List) -> Ordering {
    if list.directories_first {
        let ordering = right.kind.is_directory().cmp(&left.kind.is_directory());
        if ordering != Ordering::Equal {
            // Directories stay at the top whichever way the sort is reversed:
            // reversing a listing should turn the order round, not scatter the
            // directories through the middle of it.
            return ordering;
        }
    }

    let ordering = match list.sort {
        Sort::Name => natural(&left.name, &right.name, list.ignore_case),
        Sort::Size => left.size.cmp(&right.size),
        Sort::Modified => left.modified.cmp(&right.modified),
        Sort::Extension => natural(left.extension(), right.extension(), list.ignore_case),
    };

    // Any sort but name leaves ties, and a listing whose ties fall in readdir
    // order changes every time the directory is read.
    let ordering = ordering.then_with(|| natural(&left.name, &right.name, list.ignore_case));

    if list.sort_reversed {
        ordering.reverse()
    } else {
        ordering
    }
}

/// Compare two names with runs of digits read as numbers.
///
/// Plain byte order puts `file10` before `file2`, which is wrong every time
/// anyone has numbered anything. Written here rather than pulled in: it is
/// forty lines, and the case rule is ours.
fn natural(left: &str, right: &str, ignore_case: bool) -> Ordering {
    let mut left = left.chars().peekable();
    let mut right = right.chars().peekable();

    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,

            (Some(one), Some(other)) if one.is_ascii_digit() && other.is_ascii_digit() => {
                let ordering = digits(&mut left).cmp(&digits(&mut right));
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }

            (Some(one), Some(other)) => {
                left.next();
                right.next();

                let (one, other) = if ignore_case {
                    (
                        one.to_lowercase().to_string(),
                        other.to_lowercase().to_string(),
                    )
                } else {
                    (one.to_string(), other.to_string())
                };

                let ordering = one.cmp(&other);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
        }
    }
}

/// Take one run of digits, as a number.
///
/// `u128` and a saturating add, because a filename may hold as many digits as
/// it likes and a panic in a comparator would take the whole listing with it.
fn digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> u128 {
    let mut value: u128 = 0;

    while let Some(digit) = chars.peek().and_then(|c| c.to_digit(10)) {
        chars.next();
        value = value.saturating_mul(10).saturating_add(u128::from(digit));
    }

    value
}

/// Whether a path is one ricedir should refuse to walk into.
///
/// Not a security check — it is a courtesy. `/proc` is millions of entries
/// that change while they are being read.
pub fn is_pseudo(path: &Path) -> bool {
    matches!(path.to_str(), Some("/proc" | "/sys"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.to_owned(),
            path: PathBuf::from(name),
            kind: Kind::File,
            size: 0,
            modified: None,
            mode: 0o644,
            target: None,
            hidden: name.starts_with('.'),
        }
    }

    fn order(names: &[&str], list: &List) -> Vec<String> {
        let mut entries: Vec<Entry> = names.iter().map(|name| entry(name)).collect();
        entries.sort_by(|left, right| compare(left, right, list));
        entries.into_iter().map(|entry| entry.name).collect()
    }

    /// Byte order puts `file10` before `file2`, which is wrong every time
    /// anyone has numbered anything.
    #[test]
    fn digits_sort_as_numbers() {
        assert_eq!(
            order(&["file10", "file2", "file1"], &List::default()),
            ["file1", "file2", "file10"]
        );
    }

    /// A run of digits longer than any integer must not panic the comparator,
    /// because a panic there takes the whole listing with it.
    #[test]
    fn absurdly_long_numbers_do_not_panic() {
        let huge = "9".repeat(400);
        let names = [huge.as_str(), "1"];
        assert_eq!(order(&names, &List::default()).len(), 2);
    }

    /// Leading zeroes name the same number, so the tie has to be broken by
    /// something rather than left to readdir order.
    #[test]
    fn equal_numbers_still_order() {
        let sorted = order(&["a007", "a7"], &List::default());
        assert_eq!(sorted.len(), 2);
        assert_ne!(sorted[0], sorted[1]);
    }

    /// A newline or a quote in a name is legal on Linux and has to sort like
    /// any other character rather than being special anywhere.
    #[test]
    fn hostile_names_sort() {
        let sorted = order(&["b", "a\nb", "a'b", "a\"b"], &List::default());
        assert_eq!(sorted.len(), 4);
        assert_eq!(sorted.last().map(String::as_str), Some("b"));
    }

    /// Reversing the sort must not scatter the directories through it.
    #[test]
    fn directories_stay_first_when_reversed() {
        let mut entries = [entry("a-file"), entry("z-dir"), entry("b-file")];
        entries[1].kind = Kind::Directory;

        let list = List {
            sort_reversed: true,
            ..List::default()
        };
        entries.sort_by(|left, right| compare(left, right, &list));

        assert_eq!(entries[0].name, "z-dir");
    }

    /// A symlink to a directory is entered like a directory, so it has to sort
    /// like one too.
    #[test]
    fn a_link_to_a_directory_counts_as_one() {
        assert!(
            Kind::Link {
                directory: true,
                broken: false
            }
            .is_directory()
        );
        assert!(
            !Kind::Link {
                directory: false,
                broken: true
            }
            .is_directory()
        );
    }

    /// A leading dot makes a file hidden; it does not give it an extension.
    #[test]
    fn a_dotfile_has_no_extension() {
        assert_eq!(entry(".bashrc").extension(), "");
        assert_eq!(entry("notes.md").extension(), "md");
        assert_eq!(entry("archive.tar.gz").extension(), "gz");
        assert_eq!(entry("README").extension(), "");
    }
}
