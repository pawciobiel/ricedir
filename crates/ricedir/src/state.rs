//! What ricedir remembers between runs.
//!
//! Kept apart from the config, in `$XDG_STATE_HOME/ricedir/state.toml`. The
//! config is a file a person writes; this is a file the program writes.
//! Mixing them means ricedir rewrites something somebody hand-edited, and
//! every comment in it is lost the first time it does.
//!
//! Nothing in here is needed to run. A missing or broken state file is not
//! reported: the defaults apply and the next change writes a good one.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::Layout;

/// Where the program's own memory lives.
#[cfg(not(test))]
fn path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME") {
        return Some(PathBuf::from(dir).join("ricedir/state.toml"));
    }

    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local/state/ricedir/state.toml"))
}

/// Under test, there is no fallback to `$HOME`.
///
/// A test that switches the layout calls [`State::save`], and the first run
/// of one wrote `~/.local/state/ricedir/state.toml` into the real home. The
/// same thing happened once with the config. Leaving it out of the build is
/// stronger than remembering to set a variable.
#[cfg(test)]
fn path() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME").map(|dir| PathBuf::from(dir).join("ricedir/state.toml"))
}

/// What was last chosen, for the next run to open with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct State {
    /// The layout a new buffer starts in, when the config does not insist.
    ///
    /// One value, not one per directory. Remembering a view per folder is
    /// what Windows Explorer does, and it is the reason people complain that
    /// their folders "forget" or "change by themselves": the state grows
    /// without bound and nothing on screen says which rule won.
    pub layout: Option<Layout>,
}

impl State {
    /// Read what was remembered. Anything wrong means remember nothing.
    pub fn load() -> Self {
        let Some(path) = path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };

        toml::from_str(&text).unwrap_or_default()
    }

    /// Write it back, quietly.
    ///
    /// A failure here is not worth a notice: nothing is lost except the
    /// memory of a preference, and saying so would interrupt the thing the
    /// person was actually doing.
    pub fn save(self) {
        let Some(path) = path() else { return };
        let Ok(text) = toml::to_string(&self) else {
            return;
        };

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A missing or broken file is not an error. The defaults apply.
    #[test]
    fn rubbish_is_survivable() {
        let nothing: State = toml::from_str("").expect("empty is valid");
        assert_eq!(nothing.layout, None);

        assert!(
            toml::from_str::<State>("layout = \"nonsense\"").is_err(),
            "and a bad value is caught, so `load` falls back"
        );
    }

    /// What is written must read back as the same thing.
    #[test]
    fn a_remembered_layout_round_trips() {
        let saved = State {
            layout: Some(Layout::Detail),
        };
        let text = toml::to_string(&saved).expect("should write");
        let read: State = toml::from_str(&text).expect("should read");
        assert_eq!(read, saved);
    }
}
