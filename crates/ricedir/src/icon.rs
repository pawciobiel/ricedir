//! One glyph per kind of file.
//!
//! Every codepoint here was read out of `lsd` on this machine rather than
//! looked up: `lsd --icon always` was run over a directory of one file per
//! extension and the glyphs it printed were recorded. `CLAUDE.md` says to
//! verify a codepoint by rendering it rather than from memory, and a guessed
//! one gave ricebar a wrong icon once already.
//!
//! Written as `\u{f115}` and never as the character. Private-use codepoints
//! do not survive a copy and paste -- the list that started this file lost
//! every glyph on the way into the conversation.

use crate::entry::{Entry, Kind};

/// The glyph for a directory.
const DIRECTORY: char = '\u{f115}';
/// A symlink, whatever it points at.
const LINK: char = '\u{f482}';
/// Anything unrecognised.
const FILE: char = '\u{f016}';
/// A socket, a fifo, a device.
const SPECIAL: char = '\u{f0ae}';

/// Whole filenames, which beat an extension.
const NAMED: &[(&str, char)] = &[
    ("makefile", '\u{e615}'),
    ("gnumakefile", '\u{e615}'),
    ("cmakelists.txt", '\u{e615}'),
    ("dockerfile", '\u{f308}'),
    ("containerfile", '\u{f308}'),
    ("license", '\u{e60a}'),
    ("licence", '\u{e60a}'),
    ("copying", '\u{e60a}'),
    ("readme", '\u{e609}'),
    (".gitignore", '\u{f1d3}'),
    (".gitattributes", '\u{f1d3}'),
    (".gitmodules", '\u{f1d3}'),
    ("cargo.toml", '\u{e68b}'),
    ("cargo.lock", '\u{e68b}'),
];

/// Extensions, lowercased.
const BY_EXTENSION: &[(&str, char)] = &[
    // code
    ("rs", '\u{e68b}'),
    ("py", '\u{e606}'),
    ("rb", '\u{e21e}'),
    ("php", '\u{e608}'),
    ("go", '\u{e627}'),
    ("lua", '\u{e620}'),
    ("vim", '\u{e62b}'),
    ("java", '\u{e738}'),
    ("kt", '\u{e738}'),
    ("c", '\u{e61e}'),
    ("h", '\u{f0fd}'),
    ("hpp", '\u{f0fd}'),
    ("cpp", '\u{e61d}'),
    ("cc", '\u{e61d}'),
    ("cxx", '\u{e61d}'),
    ("js", '\u{e74e}'),
    ("mjs", '\u{e74e}'),
    ("ts", '\u{e628}'),
    ("tsx", '\u{e628}'),
    ("css", '\u{e749}'),
    ("scss", '\u{e749}'),
    ("html", '\u{f13b}'),
    ("htm", '\u{f13b}'),
    ("sh", '\u{f489}'),
    ("bash", '\u{f489}'),
    ("zsh", '\u{f489}'),
    ("fish", '\u{f489}'),
    // data and configuration
    ("json", '\u{e60b}'),
    ("toml", '\u{e60b}'),
    ("yaml", '\u{e60b}'),
    ("yml", '\u{e60b}'),
    ("conf", '\u{e615}'),
    ("ini", '\u{e615}'),
    ("cfg", '\u{e615}'),
    ("xml", '\u{f121}'),
    ("csv", '\u{f1c3}'),
    ("tsv", '\u{f1c3}'),
    ("sql", '\u{f1c0}'),
    ("db", '\u{f1c0}'),
    ("sqlite", '\u{f1c0}'),
    ("log", '\u{f18d}'),
    ("lock", '\u{f023}'),
    ("diff", '\u{e728}'),
    ("patch", '\u{e728}'),
    ("desktop", '\u{f108}'),
    // words
    ("md", '\u{e609}'),
    ("markdown", '\u{e609}'),
    ("txt", '\u{f15c}'),
    ("rst", '\u{f15c}'),
    ("org", '\u{f15c}'),
    ("pdf", '\u{f1c1}'),
    ("doc", '\u{f1c2}'),
    ("docx", '\u{f1c2}'),
    ("odt", '\u{f1c2}'),
    ("xls", '\u{f1c3}'),
    ("xlsx", '\u{f1c3}'),
    ("ods", '\u{f1c3}'),
    ("ppt", '\u{f1c4}'),
    ("pptx", '\u{f1c4}'),
    ("odp", '\u{f1c4}'),
    ("epub", '\u{f02d}'),
    // pictures, sound, moving pictures
    ("png", '\u{f1c5}'),
    ("jpg", '\u{f1c5}'),
    ("jpeg", '\u{f1c5}'),
    ("gif", '\u{f1c5}'),
    ("svg", '\u{f1c5}'),
    ("webp", '\u{f1c5}'),
    ("bmp", '\u{f1c5}'),
    ("ico", '\u{f1c5}'),
    ("avif", '\u{f1c5}'),
    ("mp3", '\u{f001}'),
    ("flac", '\u{f001}'),
    ("wav", '\u{f001}'),
    ("ogg", '\u{f001}'),
    ("opus", '\u{f001}'),
    ("m4a", '\u{f001}'),
    ("mp4", '\u{f008}'),
    ("mkv", '\u{f008}'),
    ("webm", '\u{f008}'),
    ("avi", '\u{f008}'),
    ("mov", '\u{f008}'),
    // parcels
    ("zip", '\u{f410}'),
    ("tar", '\u{f410}'),
    ("gz", '\u{f410}'),
    ("bz2", '\u{f410}'),
    ("xz", '\u{f410}'),
    ("zst", '\u{f410}'),
    ("7z", '\u{f410}'),
    ("rar", '\u{f410}'),
    ("deb", '\u{f187}'),
    ("rpm", '\u{f187}'),
    ("apk", '\u{f187}'),
    ("iso", '\u{f1c0}'),
    ("img", '\u{f1c0}'),
    // faces
    ("ttf", '\u{f031}'),
    ("otf", '\u{f031}'),
    ("woff", '\u{f031}'),
    ("woff2", '\u{f031}'),
];

/// The glyph for an entry.
///
/// A whole filename beats an extension, which beats the kind. `Makefile` is a
/// makefile whatever comes after any dot in it.
pub fn of(entry: &Entry) -> char {
    // A link is a link before it is anything else, broken or not. Checking
    // `is_directory` first drew a working link to a directory as a plain
    // folder and hid the one thing the name does not tell you. `lsd` shows
    // the link glyph too.
    match entry.kind {
        Kind::Link { .. } => return LINK,
        Kind::Directory => return DIRECTORY,
        Kind::Other => return SPECIAL,
        Kind::File => {}
    }

    let lower = entry.name.to_lowercase();

    if let Some((_, glyph)) = NAMED.iter().find(|(name, _)| *name == lower) {
        return *glyph;
    }

    // `readme.md` and `readme` are both a readme.
    if let Some(stem) = lower.split('.').next()
        && let Some((_, glyph)) = NAMED.iter().find(|(name, _)| *name == stem)
    {
        return *glyph;
    }

    let extension = entry.extension().to_lowercase();
    BY_EXTENSION
        .iter()
        .find(|(known, _)| *known == extension)
        .map_or(FILE, |(_, glyph)| *glyph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str, kind: Kind) -> Entry {
        Entry {
            name: name.to_owned(),
            path: PathBuf::from(name),
            kind,
            size: 0,
            modified: None,
            mode: 0o644,
            target: None,
            hidden: name.starts_with('.'),
        }
    }

    /// The four codepoints read out of `lsd` on this machine. If these move,
    /// somebody has guessed one.
    #[test]
    fn the_glyphs_are_the_ones_lsd_uses() {
        assert_eq!(of(&entry("src", Kind::Directory)), '\u{f115}');
        assert_eq!(of(&entry("main.rs", Kind::File)), '\u{e68b}');
        assert_eq!(of(&entry("notes.md", Kind::File)), '\u{e609}');
        assert_eq!(of(&entry("mystery", Kind::File)), '\u{f016}');
    }

    /// A whole filename beats an extension: `Cargo.toml` is Rust, not TOML,
    /// and `CMakeLists.txt` is a build file, not a text file.
    #[test]
    fn a_known_name_beats_its_extension() {
        assert_eq!(of(&entry("Cargo.toml", Kind::File)), '\u{e68b}');
        assert_ne!(
            of(&entry("Cargo.toml", Kind::File)),
            of(&entry("other.toml", Kind::File))
        );
        assert_eq!(of(&entry("CMakeLists.txt", Kind::File)), '\u{e615}');
    }

    /// A stem match catches `README.md` as well as `README`.
    #[test]
    fn a_readme_is_a_readme_either_way() {
        assert_eq!(of(&entry("README", Kind::File)), '\u{e609}');
        assert_eq!(of(&entry("README.md", Kind::File)), '\u{e609}');
        assert_eq!(of(&entry("readme.txt", Kind::File)), '\u{e609}');
    }

    /// A symlink is a symlink whether or not it leads anywhere, and a broken
    /// one must not be drawn as the directory it fails to point at.
    #[test]
    fn a_link_shows_as_a_link() {
        let whole = Kind::Link {
            directory: true,
            broken: false,
        };
        let broken = Kind::Link {
            directory: true,
            broken: true,
        };

        assert_eq!(of(&entry("to-a-directory", whole)), '\u{f482}');
        assert_eq!(of(&entry("broken", broken)), '\u{f482}');
    }

    /// Names arrive in any case, and an extension is not a statement about
    /// capitals.
    #[test]
    fn case_does_not_matter() {
        assert_eq!(
            of(&entry("PHOTO.PNG", Kind::File)),
            of(&entry("photo.png", Kind::File))
        );
        assert_eq!(of(&entry("Makefile", Kind::File)), '\u{e615}');
    }

    /// A dotfile has no extension, so it falls to the plain file glyph unless
    /// its whole name is known.
    #[test]
    fn dotfiles_are_handled() {
        assert_eq!(of(&entry(".gitignore", Kind::File)), '\u{f1d3}');
        assert_eq!(of(&entry(".bashrc", Kind::File)), '\u{f016}');
    }

    /// Every glyph in the tables is in a private-use area. One that is not is
    /// a character somebody typed by hand instead of reading from a font.
    #[test]
    fn every_glyph_is_a_nerd_font_codepoint() {
        let private = |glyph: char| {
            let point = glyph as u32;
            (0xE000..=0xF8FF).contains(&point) || (0xF0000..=0xFFFFD).contains(&point)
        };

        for (name, glyph) in NAMED.iter().chain(BY_EXTENSION) {
            assert!(private(*glyph), "{name} has U+{:04X}", *glyph as u32);
        }
        for glyph in [DIRECTORY, LINK, FILE, SPECIAL] {
            assert!(private(glyph), "U+{:04X}", glyph as u32);
        }
    }
}
