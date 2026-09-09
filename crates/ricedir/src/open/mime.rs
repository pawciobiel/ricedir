//! What kind of file this is, by name and by content.
//!
//! Two independent answers, deliberately. The name is what a handler is
//! chosen by; the content is what the `magic-mismatch` scan rule compares it
//! against, and a file whose name and content disagree is the shape of the
//! problem this program exists to notice.
//!
//! The system database is parsed directly rather than linked against.
//! `/usr/share/mime/globs2` is three colon-separated fields and
//! `aliases` is two, which is less work than a dependency; and `libmagic` is
//! a C parser for hostile input, which is the one thing to keep out of this
//! process.

use std::collections::HashMap;
use std::path::Path;

/// Where the freedesktop database lives. `$XDG_DATA_DIRS` would be more
/// correct, but every distribution puts the compiled one here and a second
/// copy has never been seen in the wild.
const GLOBS: &str = "/usr/share/mime/globs2";
const ALIASES: &str = "/usr/share/mime/aliases";

/// How much of a file is read to sniff it.
///
/// Enough for every signature in [`sniff`] plus a tar header at 257, and
/// small enough that sniffing a directory of them costs one page each.
const SNIFF: usize = 1024;

/// A glob from the database.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Glob {
    /// `*.ext`, kept without the `*.`. Over 96% of the database on this
    /// machine, and the only shape worth a hash lookup.
    Suffix(String),
    /// A whole filename: `makefile`, `pom.xml`.
    Literal(String),
    /// Everything else: `*~`, `*.[1-9]`, `readme*`, `*.so.[0-9]*`.
    Pattern(Vec<Part>),
}

/// One piece of a pattern glob.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    /// `*`
    Any,
    /// `?`
    One,
    /// `[abc]`, `[a-z]`, `[!abc]`
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

#[derive(Debug, Clone)]
struct Rule {
    weight: u32,
    mime: String,
    glob: Glob,
    case_sensitive: bool,
    /// How specific the glob was, as written. The freedesktop rule is that
    /// the longer pattern wins before weight is consulted, so `*.tar.gz`
    /// beats `*.gz` however they are weighted.
    specificity: usize,
}

/// The name-to-type half of the database.
#[derive(Debug, Default)]
pub struct Database {
    /// Lowercased suffix to rule, for the case-insensitive majority.
    suffixes: HashMap<String, Rule>,
    /// Suffixes that the database marked case-sensitive, verbatim.
    suffixes_cased: HashMap<String, Rule>,
    literals: HashMap<String, Rule>,
    literals_cased: HashMap<String, Rule>,
    /// The forty-odd awkward ones, checked in turn.
    patterns: Vec<Rule>,
    /// `application/acrobat` -> `application/pdf`, so a config naming either
    /// gets the same handler.
    aliases: HashMap<String, String>,
}

impl Database {
    /// Read the system database, or come back empty.
    ///
    /// Empty is survivable: names still fall through to [`sniff`], and the
    /// handler table can match on globs of its own. A file manager that
    /// refused to start because `shared-mime-info` was missing would be
    /// worse than one that guesses less well.
    pub fn load() -> Self {
        let globs = std::fs::read_to_string(GLOBS).unwrap_or_default();
        let aliases = std::fs::read_to_string(ALIASES).unwrap_or_default();

        if globs.is_empty() {
            eprintln!("ricedir: no {GLOBS}; falling back to content sniffing alone");
        }

        Self::parse(&globs, &aliases)
    }

    fn parse(globs: &str, aliases: &str) -> Self {
        let mut database = Self::default();

        for line in globs.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }

            // `weight:mime:glob` with an optional `:cs`. The glob itself may
            // hold a colon, so the split is bounded rather than greedy.
            let mut fields = line.splitn(3, ':');
            let Some(weight) = fields.next().and_then(|text| text.parse::<u32>().ok()) else {
                continue;
            };
            let Some(mime) = fields.next() else { continue };
            let Some(rest) = fields.next() else { continue };

            let (glob, case_sensitive) = match rest.strip_suffix(":cs") {
                Some(glob) => (glob, true),
                None => (rest, false),
            };

            database.insert(Rule {
                weight,
                mime: mime.to_owned(),
                glob: classify(glob),
                case_sensitive,
                specificity: glob.len(),
            });
        }

        for line in aliases.lines() {
            if let Some((alias, canonical)) = line.split_once(' ') {
                database
                    .aliases
                    .insert(alias.to_owned(), canonical.to_owned());
            }
        }

        database
    }

    fn insert(&mut self, rule: Rule) {
        // The database is written highest weight first, so an entry already
        // present is the better one and this is a duplicate to ignore.
        let (map, key) = match (&rule.glob, rule.case_sensitive) {
            (Glob::Suffix(suffix), false) => (&mut self.suffixes, suffix.to_lowercase()),
            (Glob::Suffix(suffix), true) => (&mut self.suffixes_cased, suffix.clone()),
            (Glob::Literal(name), false) => (&mut self.literals, name.to_lowercase()),
            (Glob::Literal(name), true) => (&mut self.literals_cased, name.clone()),
            (Glob::Pattern(_), _) => {
                self.patterns.push(rule);
                return;
            }
        };

        map.entry(key).or_insert(rule);
    }

    /// Follow an alias to the type it stands for.
    pub fn canonical<'a>(&'a self, mime: &'a str) -> &'a str {
        self.aliases.get(mime).map_or(mime, String::as_str)
    }

    /// What this file's *name* says it is.
    ///
    /// The freedesktop rule, in order: a literal filename beats a glob, and
    /// among globs the longer pattern wins before weight is consulted, so
    /// `*.tar.gz` beats `*.gz`.
    pub fn by_name(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();

        if let Some(rule) = self
            .literals_cased
            .get(name)
            .or_else(|| self.literals.get(&lower))
        {
            return Some(&rule.mime);
        }

        let mut best: Option<&Rule> = None;

        // Every suffix from every dot, so `archive.tar.gz` offers `tar.gz`
        // and then `gz`. A leading dot is a hidden file, not an extension.
        for (at, _) in name.match_indices('.').skip_while(|(at, _)| *at == 0) {
            let suffix = &name[at + 1..];
            let found = self
                .suffixes_cased
                .get(suffix)
                .or_else(|| self.suffixes.get(&lower[at + 1..]));

            if let Some(rule) = found {
                best = Some(better(best, rule));
            }
        }

        for rule in &self.patterns {
            let Glob::Pattern(parts) = &rule.glob else {
                continue;
            };

            let subject = if rule.case_sensitive { name } else { &lower };
            if matches(parts, subject) {
                best = Some(better(best, rule));
            }
        }

        best.map(|rule| rule.mime.as_str())
    }
}

/// The database, read once.
///
/// A `OnceLock` rather than a field threaded through everything: it is a few
/// hundred kilobytes of read-only tables that every handler lookup and every
/// scan rule wants, and there is exactly one system database.
pub fn database() -> &'static Database {
    static DATABASE: std::sync::OnceLock<Database> = std::sync::OnceLock::new();
    DATABASE.get_or_init(Database::load)
}

/// Match one glob against one name, for the handler table and the scan chain.
///
/// The same syntax the MIME database uses, so a config only has one glob
/// language to learn.
pub fn glob_matches(pattern: &str, name: &str) -> bool {
    match classify(pattern) {
        Glob::Suffix(suffix) => name.rsplit_once('.').is_some_and(|(before, found)| {
            !before.is_empty() && found.eq_ignore_ascii_case(&suffix)
        }),
        Glob::Literal(literal) => name.eq_ignore_ascii_case(&literal),
        Glob::Pattern(parts) => matches(&parts, &name.to_lowercase()),
    }
}

/// Whether a MIME type from a name is honestly explained by what the content
/// sniffed as.
///
/// Most disagreements are not lies. A `.docx` really is a zip, a `.py` really
/// is text, an `.m4a` really is an ISO container. A `magic-mismatch` rule that
/// fired on those would be switched off within a minute of being switched on,
/// and then it would not be there for the executable called `invoice.pdf`.
///
/// So this is a list of the ways a container legitimately shows through, and
/// everything not on it counts as a disagreement.
pub fn plausible(claimed: &str, actual: &str) -> bool {
    /// Types that are text underneath, whatever they are called.
    const TEXTUAL: &[&str] = &[
        "application/json",
        "application/xml",
        "application/javascript",
        "application/x-shellscript",
        "application/x-perl",
        "application/x-python",
        "application/x-php",
        "application/x-ruby",
        "application/x-desktop",
        "application/toml",
        "application/x-yaml",
        "application/sql",
        "image/svg+xml",
    ];

    /// Formats that are a zip with a particular layout inside.
    const ZIPPED: &[&str] = &[
        "application/vnd.openxmlformats-officedocument",
        "application/vnd.oasis.opendocument",
        "application/vnd.ms-",
        "application/epub+zip",
        "application/java-archive",
        "application/vnd.android.package-archive",
        "application/x-xpinstall",
        "application/vnd.comicbook+zip",
        "application/x-krita",
    ];

    let textual =
        |mime: &str| mime.starts_with("text/") || mime.ends_with("+xml") || TEXTUAL.contains(&mime);

    match actual {
        "text/plain" => textual(claimed),
        // A shebang makes any script look like a shell script.
        "application/x-shellscript" => textual(claimed),
        "application/zip" => {
            claimed.ends_with("+zip") || ZIPPED.iter().any(|prefix| claimed.starts_with(prefix))
        }
        "application/gzip" => claimed.contains("gzip") || claimed.contains("compressed-tar"),
        // One ISO container carries audio, video and stills alike.
        "video/mp4" | "application/ogg" | "application/x-riff" => {
            claimed.starts_with("audio/")
                || claimed.starts_with("video/")
                || claimed.starts_with("image/")
        }
        // Every ELF looks the same at the first four bytes.
        "application/x-executable" => {
            claimed.starts_with("application/x-sharedlib")
                || claimed.starts_with("application/x-pie-executable")
                || claimed.starts_with("application/x-object")
                || claimed.starts_with("application/x-core")
        }
        _ => false,
    }
}

/// Which of two matching rules the spec prefers.
///
/// A candidate always exists, so the answer always does; only what is held so
/// far can be missing.
fn better<'a>(current: Option<&'a Rule>, candidate: &'a Rule) -> &'a Rule {
    match current {
        Some(held)
            if (held.specificity, held.weight) >= (candidate.specificity, candidate.weight) =>
        {
            held
        }
        _ => candidate,
    }
}

/// Decide which shape of glob this is, so most lookups can be a hash hit.
fn classify(glob: &str) -> Glob {
    let wild = |text: &str| text.contains(['*', '?', '[']);

    if let Some(suffix) = glob.strip_prefix("*.")
        && !wild(suffix)
    {
        return Glob::Suffix(suffix.to_owned());
    }

    if !wild(glob) {
        return Glob::Literal(glob.to_owned());
    }

    Glob::Pattern(compile(glob))
}

/// Break a glob into the pieces [`matches`] walks.
fn compile(glob: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut chars = glob.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            '*' | '?' | '[' if !text.is_empty() => {
                parts.push(Part::Text(std::mem::take(&mut text)));
                // Push the character back through by handling it next round:
                // simpler to re-dispatch than to duplicate each arm.
                match character {
                    '*' => parts.push(Part::Any),
                    '?' => parts.push(Part::One),
                    _ => parts.push(class(&mut chars)),
                }
            }
            '*' => parts.push(Part::Any),
            '?' => parts.push(Part::One),
            '[' => parts.push(class(&mut chars)),
            _ => text.push(character),
        }
    }

    if !text.is_empty() {
        parts.push(Part::Text(text));
    }

    parts
}

/// Read one `[...]` class, the opening bracket already taken.
fn class(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Part {
    let negated = chars.peek() == Some(&'!') || chars.peek() == Some(&'^');
    if negated {
        chars.next();
    }

    let mut ranges = Vec::new();

    while let Some(character) = chars.next() {
        if character == ']' {
            break;
        }

        // `a-z`, but a trailing `-` is a literal hyphen.
        if chars.peek() == Some(&'-') {
            chars.next();
            match chars.peek().copied() {
                Some(']') | None => ranges.push(('-', '-')),
                Some(end) => {
                    chars.next();
                    ranges.push((character, end));
                    continue;
                }
            }
        }

        ranges.push((character, character));
    }

    Part::Class { negated, ranges }
}

/// Whether a compiled glob matches a name.
///
/// Backtracking on `*` only, which is all these patterns need: the longest
/// in the database is `*.so.[0-9]*`.
fn matches(parts: &[Part], name: &str) -> bool {
    fn walk(parts: &[Part], rest: &str) -> bool {
        let Some((first, tail)) = parts.split_first() else {
            return rest.is_empty();
        };

        match first {
            Part::Text(text) => rest
                .strip_prefix(text.as_str())
                .is_some_and(|rest| walk(tail, rest)),

            Part::One => {
                let mut chars = rest.chars();
                chars.next().is_some() && walk(tail, chars.as_str())
            }

            Part::Class { negated, ranges } => {
                let mut chars = rest.chars();
                let Some(character) = chars.next() else {
                    return false;
                };
                let inside = ranges
                    .iter()
                    .any(|(from, to)| character >= *from && character <= *to);
                inside != *negated && walk(tail, chars.as_str())
            }

            // Try the shortest match first and grow, which is what makes
            // `readme*` match `readme` as well as `readme.md`.
            Part::Any => {
                if walk(tail, rest) {
                    return true;
                }
                let mut chars = rest.chars();
                while chars.next().is_some() {
                    if walk(tail, chars.as_str()) {
                        return true;
                    }
                }
                false
            }
        }
    }

    walk(parts, name)
}

/// What this file's *content* says it is.
///
/// A hand-written table rather than `magic.mgc`: the point is to notice a
/// name that lies, and two dozen signatures cover everything anyone actually
/// receives. Reading a compiled magic database would mean parsing an
/// attacker-influenced file with an attacker-influenced file, which is the
/// wrong shape entirely.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    /// A signature, and where it sits.
    const SIGNATURES: &[(usize, &[u8], &str)] = &[
        (0, b"\x89PNG\r\n\x1a\n", "image/png"),
        (0, b"\xff\xd8\xff", "image/jpeg"),
        (0, b"GIF87a", "image/gif"),
        (0, b"GIF89a", "image/gif"),
        (0, b"BM", "image/bmp"),
        (0, b"\x00\x00\x01\x00", "image/vnd.microsoft.icon"),
        (0, b"%PDF-", "application/pdf"),
        (0, b"PK\x03\x04", "application/zip"),
        (0, b"PK\x05\x06", "application/zip"),
        (0, b"\x1f\x8b", "application/gzip"),
        (0, b"BZh", "application/x-bzip2"),
        (0, b"\xfd7zXZ\x00", "application/x-xz"),
        (0, b"\x28\xb5\x2f\xfd", "application/zstd"),
        (0, b"7z\xbc\xaf\x27\x1c", "application/x-7z-compressed"),
        (0, b"Rar!\x1a\x07", "application/vnd.rar"),
        (0, b"\x7fELF", "application/x-executable"),
        (0, b"!<arch>", "application/x-archive"),
        (0, b"\xca\xfe\xba\xbe", "application/x-java-applet"),
        (0, b"\x00asm", "application/wasm"),
        (0, b"OggS", "application/ogg"),
        (0, b"fLaC", "audio/flac"),
        (0, b"ID3", "audio/mpeg"),
        (0, b"\x1a\x45\xdf\xa3", "video/x-matroska"),
        (0, b"SQLite format 3\x00", "application/vnd.sqlite3"),
        (0, b"\x25\x21PS", "application/postscript"),
        (0, b"MZ", "application/x-ms-dos-executable"),
        // A tar header carries its magic well into the file.
        (257, b"ustar", "application/x-tar"),
    ];

    for (offset, signature, mime) in SIGNATURES {
        if bytes.len() >= offset + signature.len()
            && &bytes[*offset..offset + signature.len()] == *signature
        {
            return Some(mime);
        }
    }

    // The RIFF and ISO families put the interesting word after a length.
    if bytes.len() >= 12 {
        if &bytes[0..4] == b"RIFF" {
            return match &bytes[8..12] {
                b"WEBP" => Some("image/webp"),
                b"WAVE" => Some("audio/x-wav"),
                b"AVI " => Some("video/x-msvideo"),
                _ => Some("application/x-riff"),
            };
        }

        if &bytes[4..8] == b"ftyp" {
            return match &bytes[8..12] {
                b"avif" | b"avis" => Some("image/avif"),
                b"heic" | b"heix" => Some("image/heif"),
                b"jxl " => Some("image/jxl"),
                _ => Some("video/mp4"),
            };
        }
    }

    if bytes.starts_with(b"#!") {
        return Some("application/x-shellscript");
    }

    // Anything left that reads as text is text. Checked last, so a script
    // and an XML file keep their more useful answers.
    text(bytes).then_some("text/plain")
}

/// Whether these bytes look like text rather than data.
///
/// A NUL settles it, and so does a run of control characters that no text
/// file has. A truncated multi-byte character at the end of the buffer is
/// expected rather than a failure, since the buffer is a fixed prefix.
fn text(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.contains(&0) {
        return false;
    }

    let printable = |byte: &u8| matches!(byte, 0x09 | 0x0a | 0x0d | 0x20..=0x7e) || *byte >= 0x80;

    bytes.iter().filter(|byte| !printable(byte)).count() * 32 < bytes.len()
}

/// Read enough of a file to sniff it.
pub fn head(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut bytes = vec![0; SNIFF];
    let read = file.read(&mut bytes)?;
    bytes.truncate(read);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Database {
        Database::parse(
            "\
#comment
80:text/html:*.html
50:application/gzip:*.gz
90:application/x-compressed-tar:*.tar.gz
50:text/x-csrc:*.c:cs
50:text/x-c++src:*.C:cs
50:text/x-makefile:makefile
50:text/x-cmake:cmakelists.txt
50:application/x-trash:*~
50:text/x-readme:readme*
50:text/x-manpage:*.[1-9]
50:application/x-sharedlib:*.so.[0-9]*
",
            "application/acrobat application/pdf\n",
        )
    }

    /// A longer glob wins before weight is looked at, so an archive is an
    /// archive rather than merely gzipped.
    #[test]
    fn the_longer_suffix_wins() {
        let db = database();
        assert_eq!(
            db.by_name("archive.tar.gz"),
            Some("application/x-compressed-tar")
        );
        assert_eq!(db.by_name("notes.gz"), Some("application/gzip"));
    }

    /// Most of the database is case-insensitive, and a few entries are not.
    /// `*.c` and `*.C` are different languages and both are marked `cs`.
    #[test]
    fn case_is_respected_where_the_database_asks() {
        let db = database();
        assert_eq!(db.by_name("main.c"), Some("text/x-csrc"));
        assert_eq!(db.by_name("main.C"), Some("text/x-c++src"));
        assert_eq!(db.by_name("PAGE.HTML"), Some("text/html"));
    }

    /// A whole filename beats a glob.
    #[test]
    fn a_literal_name_wins() {
        let db = database();
        assert_eq!(db.by_name("makefile"), Some("text/x-makefile"));
        assert_eq!(db.by_name("Makefile"), Some("text/x-makefile"));
        assert_eq!(db.by_name("CMakeLists.txt"), Some("text/x-cmake"));
    }

    /// The awkward forty: trailing wildcards and character classes.
    #[test]
    fn patterns_match() {
        let db = database();
        assert_eq!(db.by_name("notes.txt~"), Some("application/x-trash"));
        assert_eq!(db.by_name("readme"), Some("text/x-readme"));
        assert_eq!(db.by_name("readme.md"), Some("text/x-readme"));
        assert_eq!(db.by_name("ricedir.1"), Some("text/x-manpage"));
        assert_eq!(db.by_name("libc.so.6"), Some("application/x-sharedlib"));
        assert_eq!(db.by_name("ricedir.0"), None);
    }

    /// A leading dot makes a file hidden rather than giving it a suffix, so
    /// `.gz` is not a gzip archive.
    #[test]
    fn a_dotfile_has_no_suffix() {
        assert_eq!(database().by_name(".gz"), None);
    }

    /// An unknown name says so rather than guessing, and the caller falls
    /// through to sniffing.
    #[test]
    fn an_unknown_name_is_none() {
        assert_eq!(database().by_name("mystery"), None);
    }

    /// Aliases exist so a config naming either spelling gets one handler.
    #[test]
    fn aliases_resolve() {
        let db = database();
        assert_eq!(db.canonical("application/acrobat"), "application/pdf");
        assert_eq!(db.canonical("application/pdf"), "application/pdf");
    }

    /// A missing database must not be fatal: names stop resolving, and
    /// content sniffing carries on.
    #[test]
    fn an_empty_database_answers_nothing_and_does_not_panic() {
        let db = Database::parse("", "");
        assert_eq!(db.by_name("notes.md"), None);
    }

    /// The signatures the scan chain leans on for a name that lies.
    #[test]
    fn content_is_sniffed() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(sniff(b"%PDF-1.7"), Some("application/pdf"));
        assert_eq!(sniff(b"PK\x03\x04...."), Some("application/zip"));
        assert_eq!(
            sniff(b"\x7fELF\x02\x01\x01"),
            Some("application/x-executable")
        );
        assert_eq!(
            sniff(b"#!/bin/sh\necho hi\n"),
            Some("application/x-shellscript")
        );
        assert_eq!(sniff(b"RIFF\x00\x00\x00\x00WEBP"), Some("image/webp"));
        assert_eq!(sniff(b"\x00\x00\x00\x18ftypavif"), Some("image/avif"));
    }

    /// Plain text is the last answer, not the first, so a script keeps the
    /// more useful one.
    #[test]
    fn text_is_the_fallback() {
        assert_eq!(sniff(b"just some words\n"), Some("text/plain"));
        assert_eq!(sniff(b"\x00\x01\x02\x03"), None);
        assert_eq!(sniff(b""), None);
    }

    /// A tar header sits at 257, which is why a whole kilobyte is read.
    #[test]
    fn a_tar_header_is_deep_in_the_file() {
        let mut bytes = vec![0x20; 300];
        bytes[257..262].copy_from_slice(b"ustar");
        assert_eq!(sniff(&bytes), Some("application/x-tar"));
    }

    /// The fixture above is a dozen lines chosen to exercise each shape. The
    /// real database is 1250, and a parser that only ever met the fixture
    /// would not have met a colon inside a glob or a weight it did not
    /// expect. Skipped where `shared-mime-info` is not installed, since that
    /// is a machine ricedir still has to run on.
    #[test]
    fn the_real_database_answers_sensibly() {
        let db = Database::load();
        if db.suffixes.is_empty() {
            eprintln!("no {GLOBS} on this machine; skipped");
            return;
        }

        for (name, expected) in [
            ("photo.png", "image/png"),
            ("notes.md", "text/markdown"),
            ("archive.tar.gz", "application/x-compressed-tar"),
            ("archive.gz", "application/gzip"),
            ("thing.desktop", "application/x-desktop"),
            ("script.sh", "application/x-shellscript"),
            ("page.HTML", "text/html"),
            ("makefile", "text/x-makefile"),
            // A character class, and the type is not the `text/troff` a
            // guess would reach for.
            ("ricedir.1", "application/x-troff-man"),
        ] {
            assert_eq!(db.by_name(name), Some(expected), "for {name}");
        }

        // Nothing in 1250 lines should claim a name with no extension at all.
        assert_eq!(db.by_name("no-such-thing-anywhere"), None);
    }

    /// An executable renamed to look like a document is exactly what the
    /// `magic-mismatch` rule is for, so the two halves must disagree.
    #[test]
    fn a_name_that_lies_is_visible() {
        let db = database();
        let bytes = b"\x7fELF\x02\x01\x01\x00";
        assert_eq!(
            db.by_name("invoice.tar.gz"),
            Some("application/x-compressed-tar")
        );
        assert_eq!(sniff(bytes), Some("application/x-executable"));
    }
}
