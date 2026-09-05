//! ricedir: a Wayland file manager.

// The entry model and half the config are written before the window that draws
// them, so most of this crate is briefly unreachable. Goes when `app` lands and
// stage 1 of M1 is finished; it is not a licence to leave dead code behind.
#![allow(dead_code)]

mod config;
mod entry;

use std::path::PathBuf;

/// What the command line asked for, if anything.
enum Arguments {
    Run {
        config: Option<PathBuf>,
        start: Option<PathBuf>,
    },
    /// Printed and exited, rather than opening a window.
    Handled,
}

fn main() {
    let Arguments::Run { config, start } = arguments() else {
        return;
    };

    let config = config::load(config);
    let start = start.unwrap_or_else(home);

    // Stands in for `app::run` until there is a window to open.
    eprintln!("ricedir: nothing to draw yet; see TODO.md");
    if let Some(path) = &config.path {
        eprintln!("ricedir: config {}", path.display());
    }
    eprintln!("ricedir: would open {}", start.display());
}

fn arguments() -> Arguments {
    let mut config = None;
    let mut start = None;
    let mut arguments = std::env::args().skip(1);

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "-c" | "--config" => match arguments.next() {
                Some(path) => config = Some(PathBuf::from(path)),
                None => {
                    eprintln!("ricedir: {argument} needs a path");
                    return Arguments::Handled;
                }
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return Arguments::Handled;
            }
            "-V" | "--version" => {
                println!("ricedir {}", env!("CARGO_PKG_VERSION"));
                return Arguments::Handled;
            }
            // A path is how a file manager is usually started, so anything
            // that is not a flag is one. A second is a mistake worth saying
            // out loud rather than quietly dropping.
            path if !path.starts_with('-') => {
                if start.is_some() {
                    eprintln!("ricedir: more than one directory named");
                    return Arguments::Handled;
                }
                start = Some(PathBuf::from(path));
            }
            other => {
                eprintln!("ricedir: unknown argument `{other}`");
                eprintln!("{USAGE}");
                return Arguments::Handled;
            }
        }
    }

    Arguments::Run { config, start }
}

/// Where a window opens when nothing named a directory.
fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

const USAGE: &str = "\
Usage: ricedir [options] [directory]

Options:
  -c, --config <path>  read this config instead of the default
  -h, --help           print this and exit
  -V, --version        print the version and exit

The default config is $XDG_CONFIG_HOME/ricedir/config.toml, or
~/.config/ricedir/config.toml.";

#[cfg(test)]
mod tests {
    use super::*;

    /// A window has to open somewhere even where HOME is unset, so this must
    /// never hand back an empty or relative path.
    #[test]
    fn home_is_always_absolute() {
        let found = home();
        assert!(found.is_absolute(), "{} is not absolute", found.display());
    }

    /// The usage text is what a mistyped flag prints, so it has to name every
    /// flag the parser accepts.
    #[test]
    fn usage_names_every_flag() {
        for flag in ["--config", "--help", "--version"] {
            assert!(USAGE.contains(flag), "usage does not mention {flag}");
        }
    }
}
