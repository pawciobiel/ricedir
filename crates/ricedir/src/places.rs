//! The short list of directories worth one click.
//!
//! Home, the XDG user directories, whatever is really mounted, and whatever
//! has been bookmarked. Nothing is guessed: each of the four comes from a file
//! the system already keeps, so the panel says what this machine actually has.

use std::path::{Path, PathBuf};

/// Where a place came from, which decides how it is grouped and drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Home,
    /// Documents, Downloads, Pictures and the rest.
    User,
    /// A mounted filesystem.
    Mount,
    /// Added by the person.
    Bookmark,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
    pub kind: Kind,
}

/// Filesystems that hold a person's files.
///
/// An allow-list, not a deny-list. This machine mounts twenty-one things and
/// three of them are worth showing; a deny-list would need a new entry every
/// time something invents a filesystem, and would show it wrongly until then.
const REAL: &[&str] = &[
    "ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "zfs", "jfs", "reiserfs", "vfat", "exfat",
    "ntfs", "ntfs3", "iso9660", "udf", "hfsplus", "apfs", "nfs", "nfs4", "cifs", "smb3", "fuseblk",
    "sshfs", "bcachefs",
];

/// Mount points to leave out however real their filesystem is.
///
/// Docker keeps dozens of overlays under `/var/lib/docker`, and a panel with
/// dozens of entries in it is a panel nobody reads.
const HIDDEN: &[&str] = &["/var/lib/docker", "/run", "/proc", "/sys", "/dev", "/boot"];

/// Everything worth one click, in the order it should be shown.
pub fn list() -> Vec<Place> {
    let home = std::env::var_os("HOME").map(PathBuf::from);

    let mut places = Vec::new();

    if let Some(home) = &home {
        places.push(Place {
            label: String::from("Home"),
            path: home.clone(),
            kind: Kind::Home,
        });

        let text = std::fs::read_to_string(user_dirs_path(home)).unwrap_or_default();
        places.extend(user_dirs(&text, home));
    }

    let text = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    places.extend(mounts(&text));

    if let Some(path) = bookmarks_path() {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        places.extend(bookmarks(&text));
    }

    // A directory that has been deleted is not a place. Checked once here
    // rather than at every draw.
    places.retain(|place| place.path.is_dir());
    places
}

fn user_dirs_path(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map_or_else(
        || home.join(".config/user-dirs.dirs"),
        |dir| PathBuf::from(dir).join("user-dirs.dirs"),
    )
}

/// Where ricedir keeps its own bookmarks.
///
/// Its own file, never GTK's: writing into another program's configuration is
/// how two programs come to disagree about what the person meant.
pub fn bookmarks_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(dir).join("ricedir/bookmarks"));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/ricedir/bookmarks"))
}

/// Read `user-dirs.dirs`.
///
/// The format is `XDG_PICTURES_DIR="$HOME/Pictures"`: shell-quoted, and either
/// relative to `$HOME` or absolute. `XDG_DESKTOP_DIR` pointing at `$HOME`
/// itself is how the spec says "there is no desktop directory", so it is
/// dropped rather than shown as a second Home.
fn user_dirs(text: &str, home: &Path) -> Vec<Place> {
    let mut places = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(name) = key
            .trim()
            .strip_prefix("XDG_")
            .and_then(|k| k.strip_suffix("_DIR"))
        else {
            continue;
        };

        let value = value.trim().trim_matches('"');
        let path = match value.strip_prefix("$HOME/") {
            Some(rest) => home.join(rest),
            None if value.starts_with('/') => PathBuf::from(value),
            None => continue,
        };

        if path == home {
            continue;
        }

        places.push(Place {
            label: title(name),
            path,
            kind: Kind::User,
        });
    }

    places
}

/// Read `/proc/self/mountinfo`.
///
/// The fields before the ` - ` separator are variable in number, so the
/// filesystem type is found by walking to the separator rather than by
/// counting. The mount point is always the fifth field, and the kernel escapes
/// a space in it as `\040`.
fn mounts(text: &str) -> Vec<Place> {
    let mut places = Vec::new();

    for line in text.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let Some(at) = fields.iter().position(|field| *field == "-") else {
            continue;
        };

        let (Some(point), Some(kind)) = (fields.get(4), fields.get(at + 1)) else {
            continue;
        };

        if !REAL.contains(kind) {
            continue;
        }

        let point = unescape(point);
        if point == Path::new("/") {
            places.push(Place {
                label: String::from("Filesystem"),
                path: point,
                kind: Kind::Mount,
            });
            continue;
        }

        if HIDDEN.iter().any(|hidden| point.starts_with(hidden)) {
            continue;
        }

        let label = point.file_name().map_or_else(
            || point.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        );

        places.push(Place {
            label,
            path: point,
            kind: Kind::Mount,
        });
    }

    places
}

/// Read ricedir's bookmarks: one path per line.
fn bookmarks(text: &str) -> Vec<Place> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let path = PathBuf::from(line);
            let label = path.file_name().map_or_else(
                || line.to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            Place {
                label,
                path,
                kind: Kind::Bookmark,
            }
        })
        .collect()
}

/// Add one bookmark, if it is not there already.
pub fn bookmark(path: &Path) -> std::io::Result<()> {
    use std::io::Write;

    let Some(file) = bookmarks_path() else {
        return Ok(());
    };

    let existing = std::fs::read_to_string(&file).unwrap_or_default();
    if existing.lines().any(|line| Path::new(line.trim()) == path) {
        return Ok(());
    }

    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut open = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)?;
    writeln!(open, "{}", path.display())
}

/// The kernel writes a space in a mount point as `\040`.
fn unescape(text: &str) -> PathBuf {
    if !text.contains('\\') {
        return PathBuf::from(text);
    }

    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();

    while let Some(character) = chars.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }

        let octal: String = chars.by_ref().take(3).collect();
        match u8::from_str_radix(&octal, 8) {
            Ok(byte) => out.push(byte as char),
            Err(_) => {
                out.push('\\');
                out.push_str(&octal);
            }
        }
    }

    PathBuf::from(out)
}

/// `DOWNLOAD` becomes `Download`.
///
/// Two of the spec's names are one word that reads as two, so they are named
/// rather than folded: `PUBLICSHARE` is not "Publicshare".
fn title(name: &str) -> String {
    match name {
        "PUBLICSHARE" => return String::from("Public"),
        "DOCUMENTS" => return String::from("Documents"),
        _ => {}
    }

    let lower = name.to_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => lower,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real format, verbatim from this machine.
    #[test]
    fn user_directories_are_read() {
        let text = r#"# This file is written by xdg-user-dirs-update
XDG_DESKTOP_DIR="$HOME/Desktop"
XDG_DOWNLOAD_DIR="$HOME/Downloads"
XDG_PICTURES_DIR="$HOME/Pictures"
"#;
        let places = user_dirs(text, Path::new("/home/someone"));

        assert_eq!(places.len(), 3);
        assert_eq!(places[1].label, "Download");
        assert_eq!(places[1].path, PathBuf::from("/home/someone/Downloads"));
        assert_eq!(places[1].kind, Kind::User);
    }

    /// `PUBLICSHARE` is one word in the spec and two in English.
    #[test]
    fn awkward_names_are_written_out() {
        let places = user_dirs(
            "XDG_PUBLICSHARE_DIR=\"$HOME/Public\"\nXDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\n",
            Path::new("/home/someone"),
        );
        assert_eq!(places[0].label, "Public");
        assert_eq!(places[1].label, "Download");
    }

    /// The spec says a user directory pointing at `$HOME` means there is none.
    /// Showing it would put Home in the list twice.
    #[test]
    fn a_user_directory_that_is_home_is_dropped() {
        let text = "XDG_DESKTOP_DIR=\"$HOME/\"\nXDG_MUSIC_DIR=\"$HOME/Music\"\n";
        let places = user_dirs(text, Path::new("/home/someone"));

        assert_eq!(places.len(), 1);
        assert_eq!(places[0].label, "Music");
    }

    /// An absolute path is allowed by the spec too.
    #[test]
    fn an_absolute_user_directory_is_read() {
        let places = user_dirs("XDG_MUSIC_DIR=\"/srv/music\"\n", Path::new("/home/someone"));
        assert_eq!(places[0].path, PathBuf::from("/srv/music"));
    }

    /// The mountinfo fields before ` - ` vary in number, so the type is found
    /// by walking to the separator. Counting fields reads the wrong column.
    #[test]
    fn mounts_are_found_past_a_variable_field_count() {
        let text = "\
22 27 0:20 / /sys rw,nosuid - sysfs sysfs rw
27 1 254:2 / / rw,relatime - ext4 /dev/dm-2 rw
36 27 8:1 / /boot rw shared:1 master:2 - ext4 /dev/sda1 rw
41 27 0:35 / /media/backup rw - ext4 /dev/sdb1 rw
";
        let places = mounts(text);
        let paths: Vec<&Path> = places.iter().map(|p| p.path.as_path()).collect();

        assert!(paths.contains(&Path::new("/")), "the root is a place");
        assert!(paths.contains(&Path::new("/media/backup")));
        assert!(!paths.contains(&Path::new("/sys")), "sysfs is not a place");
        assert!(!paths.contains(&Path::new("/boot")), "boot is hidden");
    }

    /// Docker mounts dozens of overlays. A panel with dozens of entries is a
    /// panel nobody reads.
    #[test]
    fn docker_overlays_are_left_out() {
        let text = "\
99 27 0:52 / /var/lib/docker/rootfs/overlayfs/abc rw - ext4 /dev/dm-2 rw
27 1 254:2 / / rw - ext4 /dev/dm-2 rw
";
        assert_eq!(mounts(text).len(), 1);
    }

    /// The kernel escapes a space in a mount point, and a disk called
    /// `My Backup` is an ordinary thing to own.
    #[test]
    fn a_space_in_a_mount_point_survives() {
        let text = "41 27 8:17 / /media/My\\040Backup rw - ext4 /dev/sdb1 rw\n";
        let places = mounts(text);

        assert_eq!(places.len(), 1);
        assert_eq!(places[0].path, PathBuf::from("/media/My Backup"));
        assert_eq!(places[0].label, "My Backup");
    }

    /// A bookmark is one path on one line, and a path may hold anything except
    /// a newline.
    #[test]
    fn bookmarks_are_one_path_per_line() {
        let text = "# mine\n/home/someone/src\n\n/srv/with a space\n";
        let places = bookmarks(text);

        assert_eq!(places.len(), 2);
        assert_eq!(places[0].label, "src");
        assert_eq!(places[1].path, PathBuf::from("/srv/with a space"));
        assert_eq!(places[1].kind, Kind::Bookmark);
    }

    /// Nothing here may panic on rubbish: `mountinfo` is a kernel interface
    /// and `user-dirs.dirs` is hand-edited by people.
    #[test]
    fn rubbish_is_survivable() {
        assert!(mounts("").is_empty());
        assert!(mounts("nonsense\n- \n     \n").is_empty());
        assert!(user_dirs("", Path::new("/home/someone")).is_empty());
        assert!(user_dirs("=\nXDG_=\nXDG_X_DIR=\n", Path::new("/home/someone")).is_empty());
        assert!(bookmarks("").is_empty());
    }
}
