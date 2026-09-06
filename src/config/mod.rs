//! The config file: one TOML file, read once and watched afterwards.
//!
//! Stage 1 of M1 reads only what the window and the list need. Handlers, the
//! scan chain and the agent keys arrive with the milestones that use them, and
//! `deny_unknown_fields` means each has to be added here before a config may
//! mention it.

mod color;

use std::path::{Path, PathBuf};

pub use color::Rgba;
use serde::Deserialize;

/// Everything ricedir was told, plus how far it is trusted.
#[derive(Debug, Clone)]
pub struct Config {
    pub window: Window,
    pub theme: Theme,
    pub list: List,
    pub open: Open,
    /// Checked in order; the first that matches opens the file.
    pub handler: Vec<Handler>,
    /// Checked in order; the strongest verdict wins.
    pub scan: Vec<Scan>,

    /// Whether commands in this config may be run. See [`trustworthy`]. Not a
    /// config key; decided when the file is read.
    pub trusted: bool,
    /// Where this was read from, so the window can watch it for changes. Not a
    /// config key. `None` when there is nowhere to read one from at all.
    pub path: Option<PathBuf>,
    /// Why this is the built-in default rather than what the file says. Shown
    /// in the notice line, because a file manager started from a launcher has
    /// no terminal to print to.
    pub problem: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            window: Window::default(),
            theme: Theme::default(),
            list: List::default(),
            open: Open::default(),
            // Nothing to open anything with until a config says so, which is
            // what makes the no-handler dialogue the normal first experience.
            handler: Vec::new(),
            scan: Vec::new(),
            // Nothing to distrust: there is no file.
            trusted: true,
            path: None,
            problem: None,
        }
    }
}

/// What ricedir parses out of the file, before the fields above are decided.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Raw {
    window: Window,
    theme: Theme,
    list: List,
    open: Open,
    handler: Vec<Handler>,
    scan: Vec<Scan>,
}

/// How a file is opened when a handler has to be chosen.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Open {
    pub fallback: Fallback,
}

/// What to do about a file no handler matches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fallback {
    /// Name the type and offer the choices, including `xdg-open`. Taking
    /// "always" writes a real handler into the config.
    #[default]
    Ask,
    /// Hand it straight to `xdg-open`. Never silent for the types that would
    /// be run rather than opened -- see `open::REFUSED`.
    XdgOpen,
    /// Say there is no handler, and stop.
    None,
}

/// One program that may open files, and what it opens.
///
/// `run` is an argv vector and never a command line. There is no shell
/// anywhere in this program, so a file called `; rm -rf ~ #` is a file with a
/// silly name.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Handler {
    /// What to call this in a message. Defaults to the program's own name.
    pub name: Option<String>,
    /// A MIME type, with `*` allowed as the whole subtype: `video/*`.
    pub mime: Option<String>,
    /// A filename glob, for types the database does not know.
    pub glob: Option<String>,
    /// The argv to run. `{path}` becomes the file, as one element.
    pub run: Vec<String>,
    /// A flatpak application id, preferred over `run` when it is installed,
    /// because it brings a sandbox nobody here had to write.
    pub flatpak: Option<String>,
    /// Whether the file's directory becomes the working directory. Off by
    /// default: a handler should not be able to infer where it was opened
    /// from unless it needs to.
    pub in_directory: bool,
}

/// One check that runs before a file is opened.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Scan {
    /// Named so a refusal can say which rule refused. A rule with no name is
    /// a rule nobody can act on.
    pub name: String,
    pub kind: Kind,
    /// The pattern, for `glob` and `regex`.
    #[serde(rename = "match")]
    pub pattern: Option<String>,
    /// The limit in bytes, for `size`.
    pub over: Option<u64>,
    /// The argv to run, for `command`. `{path}` becomes the file.
    pub run: Vec<String>,
    /// How long `command` may take before it is killed and skipped.
    pub timeout: Option<u64>,
    /// Exit codes that mean block, and that mean warn. Anything else allows.
    pub block_on: Vec<i32>,
    pub warn_on: Vec<i32>,
    /// What a match means.
    pub verdict: Verdict,
}

impl Default for Scan {
    fn default() -> Self {
        Self {
            name: String::from("unnamed"),
            kind: Kind::Glob,
            pattern: None,
            over: None,
            run: Vec::new(),
            timeout: Some(5),
            block_on: Vec::new(),
            warn_on: Vec::new(),
            verdict: Verdict::Warn,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    #[default]
    Glob,
    Regex,
    /// The name says one type and the content says another.
    MagicMismatch,
    Size,
    /// An external scanner, judged by its exit code.
    Command,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Nothing to say; the next rule gets a turn.
    #[default]
    Allow,
    /// Open it, but say which rule was unhappy first.
    Warn,
    /// Refuse, and say which rule refused.
    Block,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Window {
    pub width: f32,
    pub height: f32,
    /// Family as fontconfig names it. List them with `fc-list : family`.
    pub font: Option<String>,
    pub font_size: f32,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            width: 1100.0,
            height: 700.0,
            font: None,
            font_size: 14.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Theme {
    pub background: Rgba,
    pub foreground: Rgba,
    /// Fill behind the row the cursor is on.
    pub accent: Rgba,
    /// Fill behind a selected row that is not the cursor.
    pub muted: Rgba,
    /// Text of something present but unimportant: a size column, a symlink
    /// target, the count in the status line.
    pub dim: Rgba,
    /// Wants attention: a broken symlink, a blocked file, a failed job.
    pub urgent: Rgba,
}

impl Default for Theme {
    fn default() -> Self {
        // Catppuccin Mocha, as ricebar ships.
        Self {
            background: Rgba::new(0x1e, 0x1e, 0x2e),
            foreground: Rgba::new(0xcd, 0xd6, 0xf4),
            accent: Rgba::new(0x89, 0xb4, 0xfa),
            muted: Rgba::new(0x45, 0x47, 0x5a),
            dim: Rgba::new(0x6c, 0x70, 0x86),
            urgent: Rgba::new(0xf3, 0x8b, 0xa8),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct List {
    /// Every row is this tall. Uniform height is what lets the list find its
    /// visible range by arithmetic rather than by measuring 100k rows.
    pub row_height: f32,
    pub show_hidden: bool,
    pub directories_first: bool,
    pub sort: Sort,
    pub sort_reversed: bool,
    /// Whether `Apple` and `apple` sort together.
    pub ignore_case: bool,
}

impl Default for List {
    fn default() -> Self {
        Self {
            row_height: 24.0,
            show_hidden: false,
            directories_first: true,
            sort: Sort::Name,
            sort_reversed: false,
            ignore_case: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    #[default]
    Name,
    Size,
    Modified,
    Extension,
}

/// Where the config lives when none was named on the command line.
fn default_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(dir).join("ricedir/config.toml"));
    }

    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/ricedir/config.toml"))
}

/// Read the config, falling back to defaults.
///
/// A broken config is reported and then ignored rather than fatal. Refusing to
/// start would leave someone who mistyped a colour with no way to see their
/// own files, and the notice line says what went wrong instead.
pub fn load(named: Option<PathBuf>) -> Config {
    // A path given on the command line wins over the usual location.
    let explicit = named.is_some();

    let Some(path) = named.or_else(default_path) else {
        eprintln!("ricedir: no HOME or XDG_CONFIG_HOME, using defaults");
        return Config::default();
    };

    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Writing a starter config belongs to stage 2, where probing what
            // is installed makes one worth writing.
            if explicit {
                eprintln!("ricedir: {} does not exist, using defaults", path.display());
            }
            return Config {
                path: Some(path),
                ..Config::default()
            };
        }
        Err(error) => {
            eprintln!("ricedir: cannot read {}: {error}", path.display());
            return Config::default();
        }
    };

    match parse(&path, &text) {
        Ok(config) => {
            eprintln!("ricedir: loaded {}", path.display());
            config
        }
        Err(problem) => {
            eprintln!("ricedir: {} is invalid, using defaults:", path.display());
            eprintln!("{problem}");

            // Still remember the path. It stays watched, so fixing the file is
            // enough to get the real config without restarting anything.
            Config {
                path: Some(path),
                problem: Some(problem),
                ..Config::default()
            }
        }
    }
}

/// Parse one config file, having already read it.
///
/// Split out from [`load`] so a running window can re-read the same path
/// without the fall-back-to-defaults behaviour that only makes sense at
/// startup.
pub fn parse(path: &Path, text: &str) -> Result<Config, String> {
    let raw: Raw = toml::from_str(text).map_err(|error| error.to_string())?;

    let trusted = trustworthy(path);

    Ok(Config {
        window: raw.window,
        theme: raw.theme,
        list: raw.list,
        open: raw.open,
        // A config others can write is a config that must not name programs
        // to run, so the tables are dropped rather than stored. The refusal
        // shows in the window, not only on stderr.
        handler: if trusted { raw.handler } else { Vec::new() },
        scan: if trusted { raw.scan } else { Vec::new() },
        trusted,
        path: Some(path.to_path_buf()),
        problem: None,
    })
}

/// Whether commands in this config may be run.
///
/// The config file *is* the trust boundary: it names the programs that open
/// files, so anyone who can write it can already run code as this user.
/// Validating the commands themselves would buy nothing — an attacker who can
/// edit the file can just as easily name an allowed path. What is worth
/// checking is whether anyone *else* can write it, which is the same check ssh
/// and sudo make of their own files.
fn trustworthy(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let writable_by_others = |path: &Path| match std::fs::metadata(path) {
        // 0o022 is the group-write and other-write bits.
        Ok(metadata) => metadata.permissions().mode() & 0o022 != 0,
        Err(_) => false,
    };

    if writable_by_others(path) {
        eprintln!(
            "ricedir: {} is writable by other users; refusing to run commands from it",
            path.display()
        );
        eprintln!("ricedir: fix with `chmod go-w {}`", path.display());
        return false;
    }

    // A writable directory means the file can simply be replaced -- unless it
    // is sticky, which is exactly the rule that says only an owner may rename
    // or unlink their own entries. Without this, `/tmp` and `/var/tmp` are
    // both 1777 and no config under either could name a program, which would
    // rule out the `-c /tmp/rig.toml` way of testing that `CLAUDE.md`
    // documents. ricebar's version of this check does not make the exception,
    // and has simply never had a config in a sticky directory.
    if let Some(parent) = path.parent()
        && writable_by_others(parent)
        && !sticky(parent)
    {
        eprintln!(
            "ricedir: {} is writable by other users; refusing to run commands from configs inside it",
            parent.display()
        );
        eprintln!("ricedir: fix with `chmod go-w {}`", parent.display());
        return false;
    }

    true
}

/// Whether a directory has the sticky bit, so only owners may remove entries.
fn sticky(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o1000 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse some config text as if it came from a file nobody else can write.
    ///
    /// A real file, mode 0600, rather than `/dev/null`: `/dev/null` is 0666,
    /// so `trustworthy` refuses it and every handler and scan rule would be
    /// dropped before a test could see them. ricebar uses `/dev/null` here and
    /// gets away with it only because nothing it parses is trust-gated.
    fn parse_text(text: &str) -> Result<Config, String> {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "ricedir-config-test-{}-{:?}.toml",
            std::process::id(),
            std::thread::current().id()
        ));

        std::fs::write(&path, text).expect("should write a test config");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("should set the mode");

        let parsed = parse(&path, text);
        let _ = std::fs::remove_file(&path);
        parsed
    }

    /// An empty config has to mean "all defaults" rather than an error, since
    /// that is what a first run and a deliberately minimal file both look like.
    #[test]
    fn empty_is_all_defaults() {
        let config = parse_text("").expect("empty config should parse");
        assert_eq!(config.list.row_height, List::default().row_height);
        assert_eq!(config.window.font_size, Window::default().font_size);
    }

    /// `deny_unknown_fields` is what turns a typo into a message instead of a
    /// setting that silently does nothing.
    #[test]
    fn rejects_an_unknown_key() {
        let problem = parse_text("[list]\nrow-hieght = 30\n").expect_err("typo should be rejected");
        assert!(
            problem.contains("row-hieght"),
            "unhelpful message: {problem}"
        );
    }

    /// Keys are kebab-case in the file and snake_case in Rust; nothing checks
    /// that mapping except a test that reads one.
    #[test]
    fn reads_kebab_case_keys() {
        let config = parse_text("[list]\nshow-hidden = true\ndirectories-first = false\n")
            .expect("kebab-case keys should parse");
        assert!(config.list.show_hidden);
        assert!(!config.list.directories_first);
    }

    /// A colour is the key most likely to be mistyped, and the message has to
    /// name the offending text rather than a serde field path alone.
    #[test]
    fn reports_a_bad_colour() {
        let problem =
            parse_text("[theme]\nbackground = \"1e1e2e\"\n").expect_err("no # should be rejected");
        assert!(problem.contains("1e1e2e"), "unhelpful message: {problem}");
    }

    /// The sort enum is lowercase in the file.
    #[test]
    fn reads_a_sort_key() {
        let config = parse_text("[list]\nsort = \"modified\"\n").expect("sort should parse");
        assert_eq!(config.list.sort, Sort::Modified);
    }
}
