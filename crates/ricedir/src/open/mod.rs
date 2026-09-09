//! Deciding what a file is, whether it may be opened, and with what.
//!
//! This module is the only door. A handler is chosen here, the scan chain
//! runs here, and the process is spawned here, so there is nowhere else for a
//! click, a key or an agent to reach that skips any of it.
//!
//! Read `## The opener` in `TODO.md` before changing any of the rules below.
//! Each one is there because the alternative is the way file managers get
//! people hurt.

pub mod mime;
pub mod scan;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::config::{Config, Fallback, Handler};

/// Types that are *run* rather than opened.
///
/// The fallback is refused for these outright, at any `fallback` setting.
/// Handing one of these to the desktop's own resolution is not a convenience,
/// it is the exploit: the whole chain ricedir replaces exists to turn a file
/// like this into a running program. A config may still name a handler for
/// one deliberately -- opening a `.desktop` file in an editor is reasonable.
/// What is refused is *guessing*.
const REFUSED: &[&str] = &[
    "application/x-desktop",
    "application/x-executable",
    "application/x-sharedlib",
    "application/x-pie-executable",
    "application/x-shellscript",
    "application/x-ms-dos-executable",
    "application/x-msdownload",
    "application/vnd.microsoft.portable-executable",
];

/// What opening this file would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Run this, once the scan chain and the person agree.
    Run {
        /// What to call it in a message.
        name: String,
        program: OsString,
        arguments: Vec<OsString>,
        directory: Option<PathBuf>,
    },
    /// Nothing matched. Name the type and offer the choices.
    NoHandler { mime: Option<String> },
    /// Nothing matched, and the fallback is not allowed to guess here.
    Refused { mime: String },
}

/// What this file is, by name and then by content.
///
/// The name decides, because that is what a handler is chosen by and what a
/// person sees. Content is only consulted when the name says nothing, so a
/// file with no extension still opens sensibly.
pub fn kind(path: &Path) -> Option<String> {
    let database = mime::database();

    if let Some(name) = path.file_name().and_then(OsStr::to_str)
        && let Some(found) = database.by_name(name)
    {
        return Some(database.canonical(found).to_owned());
    }

    let bytes = mime::head(path).ok()?;
    mime::sniff(&bytes).map(|found| database.canonical(found).to_owned())
}

/// Choose how to open a file, without opening it.
///
/// Separate from [`spawn`] so the scan chain, the dialogue and the tests can
/// all see the decision before anything runs.
pub fn plan(config: &Config, path: &Path) -> Plan {
    let mime = kind(path);
    let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();

    if let Some(handler) = config
        .handler
        .iter()
        .find(|handler| suits(handler, mime.as_deref(), name))
    {
        return build(handler, path);
    }

    match (config.open.fallback, &mime) {
        (Fallback::None, _) => Plan::NoHandler { mime: mime.clone() },

        // The refusal holds whatever the setting says. `ask` still offers the
        // dialogue, which is where a person may name a handler deliberately;
        // what neither setting may do is hand this to `xdg-open`.
        (_, Some(found)) if REFUSED.contains(&found.as_str()) => Plan::Refused {
            mime: found.clone(),
        },

        (Fallback::XdgOpen, _) => Plan::Run {
            name: String::from("xdg-open"),
            program: OsString::from("xdg-open"),
            arguments: vec![OsString::from("--"), path.as_os_str().to_owned()],
            directory: None,
        },

        (Fallback::Ask, _) => Plan::NoHandler { mime: mime.clone() },
    }
}

/// Whether a handler covers this file.
fn suits(handler: &Handler, mime: Option<&str>, name: &str) -> bool {
    if handler.run.is_empty() && handler.flatpak.is_none() {
        return false;
    }

    if let Some(mime) = mime
        && handler
            .mime
            .iter()
            .any(|pattern| mime_matches(pattern, mime))
    {
        return true;
    }

    handler
        .glob
        .iter()
        .any(|glob| mime::glob_matches(glob, name))
}

/// `video/*` against `video/mp4`. Only the whole subtype may be a star:
/// `vid*/mp4` is a typo, not a pattern.
fn mime_matches(pattern: &str, mime: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        return mime
            .split_once('/')
            .is_some_and(|(top, _)| top.eq_ignore_ascii_case(prefix));
    }

    pattern.eq_ignore_ascii_case(mime)
}

/// Turn a matching handler into the argv that will run.
fn build(handler: &Handler, path: &Path) -> Plan {
    // A flatpak wins where the app is installed, because it brings a sandbox
    // nobody here had to write. Where it is not, `run` is what is left.
    let flatpak = handler
        .flatpak
        .as_ref()
        .filter(|id| flatpak_installed(id))
        .map(|id| {
            (
                OsString::from("flatpak"),
                vec![
                    OsString::from("run"),
                    OsString::from(id),
                    OsString::from("--"),
                    path.as_os_str().to_owned(),
                ],
            )
        });

    let (program, arguments) = match flatpak {
        Some(pair) => pair,
        None => {
            let Some((program, rest)) = handler.run.split_first() else {
                return Plan::NoHandler { mime: None };
            };

            let mut arguments: Vec<OsString> =
                rest.iter().map(|text| substitute(text, path)).collect();

            // A handler that does not say where the path goes gets it last,
            // after `--`, which is the shape that cannot be mistaken for an
            // option however the file is named.
            if !rest.iter().any(|text| text.contains("{path}")) {
                arguments.push(OsString::from("--"));
                arguments.push(path.as_os_str().to_owned());
            }

            (OsString::from(program), arguments)
        }
    };

    let name = handler
        .name
        .clone()
        .unwrap_or_else(|| program.to_string_lossy().into_owned());

    Plan::Run {
        name,
        program,
        arguments,
        directory: handler
            .in_directory
            .then(|| path.parent().map(Path::to_path_buf))
            .flatten(),
    }
}

/// Put the path into one argv element.
///
/// An `OsString`, so a filename that is not UTF-8 survives: Linux names are
/// bytes, and a handler must receive exactly the bytes that name the file
/// rather than a lossy rendering of them.
pub fn substitute(argument: &str, path: &Path) -> OsString {
    let Some((before, after)) = argument.split_once("{path}") else {
        return OsString::from(argument);
    };

    let mut built = OsString::from(before);
    built.push(path.as_os_str());
    built.push(after);
    built
}

/// Whether a flatpak application is installed.
///
/// Asked once per application and remembered: `flatpak info` is a process,
/// and a directory of videos would otherwise start one per row.
fn flatpak_installed(id: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static SEEN: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashMap::new()));

    if let Ok(seen) = seen.lock()
        && let Some(known) = seen.get(id)
    {
        return *known;
    }

    let found = std::process::Command::new("flatpak")
        .args(["info", "--show-ref", id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());

    if let Ok(mut seen) = seen.lock() {
        seen.insert(id.to_owned(), found);
    }

    found
}

/// Start a handler, and leave it to get on with it.
///
/// Detached on purpose: these launch editors and video players, and waiting
/// on one would hold the window for as long as somebody watched a film.
/// stderr is piped so a handler that fails immediately can say why in the
/// notice line rather than into a terminal nobody is looking at.
///
/// Not `async`: there is nothing to wait for. It still has to be called from
/// inside the tokio runtime, because `tokio::process::Command` registers the
/// child with the reactor.
pub fn spawn(plan: &Plan) -> Result<(), String> {
    let Plan::Run {
        name,
        program,
        arguments,
        directory,
    } = plan
    else {
        return Err(String::from("nothing to run"));
    };

    let mut command = tokio::process::Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    if let Some(directory) = directory {
        command.current_dir(directory);
    }

    // Its own process group, so a handler that spawns children does not leave
    // them attached to ricedir's group. `kill_on_drop` reaches a child but
    // never its grandchildren, which ricebar measured the hard way.
    command.process_group(0);

    match command.spawn() {
        Ok(_) => Ok(()),
        Err(error) => Err(format!("`{name}` would not start: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Handler};

    fn config(handlers: Vec<Handler>, fallback: Fallback) -> Config {
        Config {
            handler: handlers,
            open: crate::config::Open { fallback },
            ..Config::default()
        }
    }

    fn handler(mime: Option<&str>, glob: Option<&str>, run: &[&str]) -> Handler {
        use std::fmt::Write;

        // Built through the parser, so the tests exercise the same
        // scalar-or-array deserialiser a real config goes through.
        let mut text = String::from("[[handler]]\n");
        if let Some(mime) = mime {
            let _ = writeln!(text, "mime = \"{mime}\"");
        }
        if let Some(glob) = glob {
            let _ = writeln!(text, "glob = \"{glob}\"");
        }
        let quoted: Vec<String> = run.iter().map(|text| format!("\"{text}\"")).collect();
        let _ = writeln!(text, "run = [{}]", quoted.join(", "));

        let raw: crate::config::Raw = toml::from_str(&text).expect("test handler should parse");
        raw.handler.into_iter().next().expect("one handler")
    }

    /// The path goes in as one argv element after `--`, so a file named like
    /// an option is a file and not an option.
    #[test]
    fn a_path_is_one_argument_after_a_separator() {
        let config = config(
            vec![handler(Some("text/*"), None, &["nvim"])],
            Fallback::Ask,
        );
        let plan = plan(&config, Path::new("/tmp/--config=evil.txt"));

        let Plan::Run {
            program, arguments, ..
        } = plan
        else {
            panic!("expected a handler, got {plan:?}");
        };

        assert_eq!(program, OsString::from("nvim"));
        assert_eq!(
            arguments,
            [
                OsString::from("--"),
                OsString::from("/tmp/--config=evil.txt")
            ]
        );
    }

    /// A filename full of shell metacharacters is a filename. There is no
    /// shell anywhere, so it arrives at the handler exactly as it is.
    #[test]
    fn a_hostile_name_is_just_a_name() {
        let config = config(
            vec![handler(Some("text/*"), None, &["nvim"])],
            Fallback::Ask,
        );
        let nasty = "/tmp/; rm -rf ~ #.txt";

        let Plan::Run { arguments, .. } = plan(&config, Path::new(nasty)) else {
            panic!("expected a handler");
        };

        assert_eq!(arguments.last(), Some(&OsString::from(nasty)));
    }

    /// `{path}` puts the file where the handler wants it, still as one
    /// element even when it is glued to other text.
    #[test]
    fn a_placeholder_stays_one_argument() {
        let config = config(
            vec![handler(
                Some("text/*"),
                None,
                &["editor", "--file={path}", "-q"],
            )],
            Fallback::Ask,
        );

        let Plan::Run { arguments, .. } = plan(&config, Path::new("/tmp/a b.txt")) else {
            panic!("expected a handler");
        };

        assert_eq!(
            arguments,
            [OsString::from("--file=/tmp/a b.txt"), OsString::from("-q")]
        );
    }

    /// `video/*` covers a subtype; a partial star is a typo and matches
    /// nothing rather than everything.
    #[test]
    fn mime_stars_cover_a_subtype_only() {
        assert!(mime_matches("video/*", "video/mp4"));
        assert!(mime_matches("VIDEO/*", "video/mp4"));
        assert!(!mime_matches("video/*", "audio/mpeg"));
        assert!(mime_matches("text/plain", "text/plain"));
        assert!(!mime_matches("vid*/mp4", "video/mp4"));
    }

    /// The first matching handler wins, so a config can put a specific rule
    /// above a general one.
    #[test]
    fn the_first_matching_handler_wins() {
        let config = config(
            vec![
                handler(None, Some("*.md"), &["glow"]),
                handler(Some("text/*"), None, &["nvim"]),
            ],
            Fallback::Ask,
        );

        let Plan::Run { program, .. } = plan(&config, Path::new("/tmp/notes.md")) else {
            panic!("expected a handler");
        };
        assert_eq!(program, OsString::from("glow"));
    }

    /// A handler with no program cannot be chosen, however well its pattern
    /// matches, or it would swallow the file and do nothing.
    #[test]
    fn a_handler_with_no_program_is_not_a_handler() {
        let config = config(vec![handler(Some("text/*"), None, &[])], Fallback::Ask);
        assert!(matches!(
            plan(&config, Path::new("/tmp/notes.txt")),
            Plan::NoHandler { .. }
        ));
    }

    /// Nothing matches and the answer is a dialogue naming the type, not
    /// silence.
    #[test]
    fn no_handler_names_the_type() {
        let config = config(Vec::new(), Fallback::Ask);
        let Plan::NoHandler { mime } = plan(&config, Path::new("/tmp/photo.png")) else {
            panic!("expected the dialogue");
        };
        assert_eq!(mime.as_deref(), Some("image/png"));
    }

    /// The load-bearing one. `xdg-open` resolves through `.desktop` files, so
    /// letting it near a `.desktop` file or a script is handing over the
    /// exact chain this program replaces -- at any fallback setting.
    #[test]
    fn the_fallback_never_guesses_at_something_that_would_run() {
        for setting in [Fallback::XdgOpen, Fallback::Ask] {
            let config = config(Vec::new(), setting);

            for name in ["thing.desktop", "script.sh"] {
                let plan = plan(&config, Path::new("/tmp").join(name).as_path());
                assert!(
                    matches!(plan, Plan::Refused { .. }),
                    "{name} at {setting:?} gave {plan:?}"
                );
            }
        }
    }

    /// A config may still name a handler for one deliberately: opening a
    /// `.desktop` file in an editor is a reasonable thing to want. What is
    /// refused is guessing, not choosing.
    #[test]
    fn a_named_handler_may_still_open_a_desktop_file() {
        let config = config(
            vec![handler(Some("application/x-desktop"), None, &["nvim"])],
            Fallback::Ask,
        );

        let Plan::Run { program, .. } = plan(&config, Path::new("/tmp/thing.desktop")) else {
            panic!("a named handler should be honoured");
        };
        assert_eq!(program, OsString::from("nvim"));
    }

    /// `fallback = "none"` means no dialogue and no guess.
    #[test]
    fn the_fallback_can_be_turned_off() {
        let config = config(Vec::new(), Fallback::None);
        assert!(matches!(
            plan(&config, Path::new("/tmp/photo.png")),
            Plan::NoHandler { .. }
        ));
    }

    /// With the fallback on, an ordinary unknown file reaches `xdg-open` --
    /// with `--` before the path, like every other handler.
    #[test]
    fn the_fallback_reaches_xdg_open_for_ordinary_files() {
        let config = config(Vec::new(), Fallback::XdgOpen);
        let Plan::Run {
            program, arguments, ..
        } = plan(&config, Path::new("/tmp/photo.png"))
        else {
            panic!("expected xdg-open");
        };

        assert_eq!(program, OsString::from("xdg-open"));
        assert_eq!(arguments.first(), Some(&OsString::from("--")));
    }

    /// A config others can write names no programs at all, so every file
    /// falls through to the dialogue rather than to whatever it said.
    ///
    /// `/dev/null` looks like the obvious stand-in for "a file that exists"
    /// and is exactly wrong here: it is mode 0666, so the trust check refuses
    /// it and the handlers are gone before the test can look. A real file at
    /// 0600 is the only way to see the trusted path.
    #[test]
    fn an_untrusted_config_has_no_handlers() {
        use std::os::unix::fs::PermissionsExt;

        let text = "[[handler]]\nmime = \"text/*\"\nrun = [\"evil\"]\n";
        let path = std::env::temp_dir().join(format!("ricedir-trust-{}.toml", std::process::id()));

        std::fs::write(&path, text).expect("should write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("should set the mode");
        let trusted = crate::config::parse(&path, text).expect("should parse");
        assert_eq!(
            trusted.handler.len(),
            1,
            "a private config keeps its handlers"
        );

        // Now the same file, writable by anybody.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))
            .expect("should set the mode");
        let untrusted = crate::config::parse(&path, text).expect("should parse");
        let _ = std::fs::remove_file(&path);

        assert!(!untrusted.trusted);
        assert!(untrusted.handler.is_empty(), "a shared config keeps none");
        assert!(matches!(
            plan(&untrusted, Path::new("/tmp/notes.txt")),
            Plan::NoHandler { .. }
        ));
    }
}
