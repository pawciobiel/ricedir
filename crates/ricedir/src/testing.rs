//! Where a test is allowed to write.
//!
//! Every test that makes a file makes it under the repository's `tmp/`, and
//! [`jobs::carry_out`](crate::jobs) refuses to change anything outside it.
//!
//! This project has twice had a test write into the real home: `State::save`
//! left a `state.toml` in `~/.local/state`, and a config test wrote
//! `~/.config/ricedir`. Both only left a file behind. A copy or a delete that
//! names the wrong path takes one away instead, so the rule stops being about
//! tidiness at M2.
//!
//! The root comes from `CARGO_MANIFEST_DIR`, which cargo fills in at compile
//! time. A test therefore lands in the same place whatever directory it runs
//! from, and there is no variable to remember to set.

use std::path::{Component, Path, PathBuf};

/// The repository's `tmp/`.
///
/// Nothing is made here. [`scratch`] makes what it hands out.
pub fn root() -> PathBuf {
    // `<repo>/crates/ricedir` is where the manifest is, so the repository is
    // two above it.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate sits two deep in the repository")
        .join("tmp")
}

/// An empty directory of this test's own.
///
/// The process id is part of the name, so two `cargo test` runs at once do
/// not share one. It is cleared first, so a test that failed half way through
/// does not poison the next run.
pub fn scratch(name: &str) -> PathBuf {
    let path = root().join(format!("{name}-{}", std::process::id()));

    // `remove_dir_all` below is the one place this module destroys anything,
    // so it must never be handed the root. An empty name, or one that climbs,
    // would do exactly that and take every other test's directory with it.
    assert!(
        path.parent() == Some(root().as_path()) && inside(&path),
        "a scratch directory is one name under {}: {}",
        root().display(),
        path.display()
    );

    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("should make the scratch directory");
    path
}

/// Whether a test may change this path.
///
/// `..` is refused rather than resolved. A test has no reason to name one,
/// and resolving it would mean canonicalising a path that does not exist yet
/// -- which is every destination a copy has.
pub fn inside(path: &Path) -> bool {
    !path.components().any(|part| part == Component::ParentDir) && path.starts_with(root())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_is_the_repositorys_own_tmp() {
        let root = root();
        assert!(root.is_absolute(), "{} is not absolute", root.display());
        assert!(root.ends_with("tmp"));
        assert!(
            root.parent()
                .expect("tmp has a parent")
                .join("Cargo.toml")
                .is_file(),
            "{} is not beside the workspace manifest",
            root.display()
        );
    }

    /// The root holds every test's directory, so nothing may name it.
    #[test]
    #[should_panic(expected = "a scratch directory is one name under")]
    fn a_scratch_directory_cannot_be_the_root() {
        let _ = scratch("../rig-home");
    }

    #[test]
    fn only_tmp_is_allowed() {
        assert!(inside(&scratch("guard").join("a/b.txt")));
        assert!(!inside(Path::new("/home")));
        assert!(!inside(Path::new("/tmp/ricedir-test")));
        assert!(
            !inside(&root().join("../../etc/passwd")),
            "a `..` climbs out and must be refused"
        );
    }
}
