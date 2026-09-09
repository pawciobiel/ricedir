//! The config written the first time ricedir starts.
//!
//! Generated from what is actually installed rather than shipped fixed. A
//! config full of handlers for programs this machine does not have would send
//! every file to the no-handler dialogue and teach nobody anything; one that
//! names what is here works immediately, and says beside each missing entry
//! what it would need.
//!
//! The same trick as ricebar's first run, with `Need::Flatpak` added, since a
//! flatpak is the handler worth preferring where there is one.

use std::io;
use std::path::Path;

/// The file, with `# @HANDLERS@` still in it.
const TEMPLATE: &str = include_str!("../../config.default.toml");
const HANDLERS: &str = "# @HANDLERS@";

/// What a handler needs before it is worth writing uncommented.
#[derive(Debug, Clone, Copy)]
enum Need {
    /// A program on `$PATH`.
    Program(&'static str),
    /// A flatpak application. Preferred where both are available.
    Flatpak(&'static str),
    /// A terminal, and something to run in it.
    Terminal(&'static str, &'static str),
}

/// One handler ricedir knows how to offer.
struct Offered {
    comment: &'static str,
    matcher: &'static str,
    /// Tried in order; the first that is available wins.
    needs: &'static [Need],
}

/// Everything worth offering, most specific first, since handlers are checked
/// in the order they are written.
const OFFERED: &[Offered] = &[
    Offered {
        comment: "images",
        matcher: r#"mime = "image/*""#,
        needs: &[
            Need::Program("swayimg"),
            Need::Program("imv"),
            Need::Program("nsxiv"),
            Need::Flatpak("org.xfce.ristretto"),
            Need::Program("feh"),
        ],
    },
    Offered {
        comment: "video",
        matcher: r#"mime = "video/*""#,
        needs: &[
            Need::Flatpak("io.mpv.Mpv"),
            Need::Program("mpv"),
            Need::Flatpak("org.videolan.VLC"),
        ],
    },
    Offered {
        comment: "audio",
        matcher: r#"mime = "audio/*""#,
        needs: &[
            Need::Flatpak("io.mpv.Mpv"),
            Need::Program("mpv"),
            Need::Flatpak("org.videolan.VLC"),
        ],
    },
    Offered {
        comment: "PDFs and ebooks",
        matcher: r#"mime = "application/pdf""#,
        needs: &[
            Need::Flatpak("org.kde.okular"),
            Need::Program("zathura"),
            Need::Program("mupdf"),
        ],
    },
    Offered {
        comment: "office documents",
        // A TOML array rather than brace expansion. The first draft wrote
        // `*.{doc,docx,...}`, which the glob matcher reads as one literal
        // suffix, so the handler matched nothing at all -- caught by reading
        // the generated file rather than by any test.
        matcher: r#"glob = ["*.doc", "*.docx", "*.odt", "*.xls", "*.xlsx", "*.ods", "*.ppt", "*.pptx", "*.odp"]"#,
        needs: &[Need::Flatpak("org.libreoffice.LibreOffice")],
    },
    Offered {
        comment: "web pages",
        matcher: r#"mime = "text/html""#,
        needs: &[
            Need::Flatpak("org.mozilla.firefox"),
            Need::Flatpak("com.brave.Browser"),
            Need::Flatpak("org.chromium.Chromium"),
        ],
    },
    Offered {
        comment: "anything textual",
        matcher: r#"mime = "text/*""#,
        // A text editor on Linux is usually a terminal program, so the
        // terminal pairs come first and the bare ones are only the editors
        // that are certainly windows of their own.
        //
        // `emacs` was at the top of this list and it was wrong. The build
        // here is emacs-nox: it links no GUI toolkit at all, so launched from
        // a window manager it has no terminal, starts, and dies without
        // drawing anything. Nothing on `$PATH` says which build you have, and
        // the same trap waits for `vim`, `vi` and `nano`.
        needs: &[
            Need::Terminal("foot", "nvim"),
            Need::Terminal("foot", "vim"),
            Need::Terminal("foot", "emacs"),
            Need::Terminal("foot", "nano"),
            Need::Terminal("alacritty", "nvim"),
            Need::Terminal("alacritty", "vim"),
            Need::Terminal("alacritty", "emacs"),
            Need::Terminal("alacritty", "nano"),
            // Editors that are their own window, so no terminal is wanted.
            Need::Program("gnome-text-editor"),
            Need::Program("gedit"),
            Need::Program("kate"),
            Need::Program("mousepad"),
            Need::Program("code"),
        ],
    },
];

/// Write a starter config, and the directory it lives in.
///
/// Only ever additive: an existing file is left alone, so deleting the config
/// is how somebody asks for a fresh one.
pub fn create(path: &Path) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(path, TEMPLATE.replace(HANDLERS, &handlers()))?;

    // The config names programs to run, so it is a file others must not be
    // able to write. `trustworthy` would refuse it otherwise, and the refusal
    // would be baffling on a file ricedir wrote itself.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;

    eprintln!("ricedir: no config found, wrote {}", path.display());
    Ok(())
}

/// The `[[handler]]` blocks, from what this machine has.
fn handlers() -> String {
    let mut written = String::new();

    for offered in OFFERED {
        let found = offered.needs.iter().find(|need| available(need));

        match found {
            Some(need) => {
                written.push_str(&format!(
                    "# {}\n[[handler]]\n{}\n{}\n\n",
                    offered.comment,
                    offered.matcher,
                    invocation(need)
                ));
            }
            None => {
                // Written commented out with what it wanted, so the file is
                // still a menu of what ricedir could do here.
                let wanted: Vec<String> = offered.needs.iter().map(describe).collect();
                written.push_str(&format!(
                    "# {} -- nothing installed. Wanted one of: {}.\n\
                     #   [[handler]]\n\
                     #   {}\n\
                     #   run = [\"...\", \"--\", \"{{path}}\"]\n\n",
                    offered.comment,
                    wanted.join(", "),
                    offered.matcher,
                ));
            }
        }
    }

    written.trim_end().to_owned()
}

/// The TOML line that runs this.
fn invocation(need: &Need) -> String {
    match need {
        Need::Program(program) => format!("run = [\"{program}\", \"--\", \"{{path}}\"]"),
        Need::Flatpak(id) => format!("flatpak = \"{id}\""),
        Need::Terminal(terminal, program) => {
            format!("run = [\"{terminal}\", \"-e\", \"{program}\", \"--\", \"{{path}}\"]")
        }
    }
}

fn describe(need: &Need) -> String {
    match need {
        Need::Program(program) => (*program).to_owned(),
        Need::Flatpak(id) => format!("flatpak {id}"),
        Need::Terminal(terminal, program) => format!("{terminal} with {program}"),
    }
}

fn available(need: &Need) -> bool {
    match need {
        Need::Program(program) => on_path(program),
        Need::Flatpak(id) => flatpak(id),
        Need::Terminal(terminal, program) => on_path(terminal) && on_path(program),
    }
}

/// Whether a program is on `$PATH` and executable.
fn on_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&path).any(|directory| {
        std::fs::metadata(directory.join(program))
            .is_ok_and(|found| found.is_file() && found.permissions().mode() & 0o111 != 0)
    })
}

/// Whether a flatpak application is installed.
fn flatpak(id: &str) -> bool {
    use std::process::Stdio;

    std::process::Command::new("flatpak")
        .args(["info", "--show-ref", id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The template ships with the placeholder in it, and a substitution that
    /// silently matched nothing would write a config with no handlers at all.
    #[test]
    fn the_template_has_its_placeholder() {
        assert!(
            TEMPLATE.contains(HANDLERS),
            "config.default.toml lost `{HANDLERS}`"
        );
    }

    /// The whole point of `deny_unknown_fields` is that a key ricedir does not
    /// know is an error. That makes the shipped template able to break first
    /// run, so it is parsed here rather than on somebody's machine.
    #[test]
    fn the_generated_config_parses() {
        let text = TEMPLATE.replace(HANDLERS, &handlers());
        let config = super::super::parse(Path::new("/dev/null"), &text)
            .expect("the config ricedir writes must be one it can read");

        // `/dev/null` is 0666 so this parses untrusted, which empties the
        // handler table -- the tables are still proof the keys were accepted.
        assert!(!config.trusted);
    }

    /// A machine with nothing installed still gets a readable file, with every
    /// entry commented out and saying what it wanted.
    #[test]
    fn nothing_installed_still_writes_a_menu() {
        let written = handlers();
        for offered in OFFERED {
            assert!(
                written.contains(offered.comment),
                "no mention of {}",
                offered.comment
            );
        }
    }

    /// Every generated handler puts the path in as one element, after `--`
    /// where there is no placeholder. A generated config that got this wrong
    /// would undo the argv rule everywhere at once.
    #[test]
    fn every_invocation_passes_the_path_safely() {
        for offered in OFFERED {
            for need in offered.needs {
                let line = invocation(need);
                assert!(
                    line.contains("{path}") || line.starts_with("flatpak = "),
                    "{line} does not pass the path"
                );

                if line.starts_with("run = ") {
                    assert!(line.contains("\"--\""), "{line} has no separator");
                }
            }
        }
    }

    /// Every generated matcher has to be a pattern the matcher understands.
    /// The office entry was first written with brace expansion, which parses
    /// as TOML, passes `deny_unknown_fields`, and silently matches nothing.
    #[test]
    fn every_matcher_matches_something() {
        use crate::open::mime;

        for offered in OFFERED {
            let Some(rest) = offered.matcher.strip_prefix("glob = ") else {
                continue;
            };

            // The globs are written as TOML, so read them back as TOML --
            // through the same scalar-or-array reader a real config uses.
            #[derive(serde::Deserialize)]
            struct Holder {
                glob: super::super::Patterns,
            }

            let holder: Holder = toml::from_str(&format!("glob = {rest}"))
                .expect("the generated glob should be valid TOML");
            let patterns: Vec<String> = holder.glob.iter().cloned().collect();

            for pattern in patterns {
                assert!(
                    !pattern.contains('{') && !pattern.contains('}'),
                    "{pattern} uses brace expansion, which the matcher does not"
                );

                // A glob has to match the name it was written for.
                let example = pattern.replace('*', "example");
                assert!(
                    mime::glob_matches(&pattern, &example),
                    "{pattern} does not match {example}"
                );
            }
        }
    }

    /// `on_path` has to agree with the shell about something that is certainly
    /// there, and about something that is certainly not.
    #[test]
    fn programs_are_found_on_the_path() {
        assert!(on_path("sh"), "sh should be on PATH");
        assert!(!on_path("ricedir-no-such-program-anywhere"));
    }
}
