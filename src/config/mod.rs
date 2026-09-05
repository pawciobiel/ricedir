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

    Ok(Config {
        window: raw.window,
        theme: raw.theme,
        list: raw.list,
        trusted: trustworthy(path),
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

    // A writable directory means the file can simply be replaced.
    if let Some(parent) = path.parent()
        && writable_by_others(parent)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `/dev/null` stands in for a file that exists and is not group- or
    /// other-writable, so `parse` can be exercised without touching a disk.
    fn parse_text(text: &str) -> Result<Config, String> {
        parse(Path::new("/dev/null"), text)
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
