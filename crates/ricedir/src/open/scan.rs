//! The scan chain: what a machine-specific opinion gets to say about a file
//! before it is opened.
//!
//! Every rule comes from the config, so none of this is compiled in. That is
//! the point: the useful check differs per machine, and a scanner nobody can
//! turn off or replace is a scanner people work around.
//!
//! The chain runs before *every* open, whichever handler wins, including the
//! `xdg-open` fallback. There is no path around it, because `open::resolve`
//! is the only door.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use regex_lite::Regex;

use crate::config::{Kind, Scan, Verdict};
use crate::open::mime;

/// The strongest thing the chain had to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub verdict: Verdict,
    /// Which rule reached that verdict, so a refusal can name it. A person
    /// told only "blocked" has nothing to edit.
    pub rule: Option<String>,
    /// What the rule found, in a sentence.
    pub detail: Option<String>,
}

impl Report {
    pub const fn allowed() -> Self {
        Self {
            verdict: Verdict::Allow,
            rule: None,
            detail: None,
        }
    }

    pub fn blocked(&self) -> bool {
        self.verdict == Verdict::Block
    }
}

/// Run the chain over one file.
///
/// The strongest verdict wins rather than the first: a rule that only warns
/// must not hide a later one that blocks, and the order rules happen to sit
/// in a config file is not a judgement about severity. A `block` short-cuts
/// the rest, since nothing can outrank it.
///
/// Async because `Kind::Command` spawns a scanner. Everything else is a
/// string comparison and a `stat`.
pub async fn run(rules: &[Scan], path: &Path) -> Report {
    let name = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());

    let mut report = Report::allowed();

    for rule in rules {
        let found = match rule.kind {
            Kind::Glob => glob(rule, &name).map(|()| rule.verdict),
            Kind::Regex => regex(rule, &name),
            Kind::MagicMismatch => mismatch(rule, path),
            Kind::Size => size(rule, path).map(|()| rule.verdict),
            Kind::Command => command(rule, path).await,
        };

        let Some(verdict) = found else { continue };
        if verdict == Verdict::Allow {
            continue;
        }

        if verdict > report.verdict {
            report = Report {
                verdict,
                rule: Some(rule.name.clone()),
                detail: detail(rule, &name),
            };
        }

        if report.blocked() {
            break;
        }
    }

    report
}

/// A sentence for the dialogue, saying what the rule was looking for.
fn detail(rule: &Scan, name: &str) -> Option<String> {
    match rule.kind {
        Kind::Glob | Kind::Regex => rule
            .pattern
            .as_ref()
            .map(|pattern| format!("`{name}` matches `{pattern}`")),
        Kind::MagicMismatch => Some(String::from(
            "the name and the content disagree about what this is",
        )),
        Kind::Size => rule.over.map(|over| format!("larger than {over} bytes")),
        Kind::Command => Some(format!("`{}` said so", rule.run.first()?)),
    }
}

fn glob(rule: &Scan, name: &str) -> Option<()> {
    let pattern = rule.pattern.as_ref()?;
    // The same matcher the MIME database uses, so one glob syntax is learnt
    // rather than two.
    mime::glob_matches(pattern, name).then_some(())
}

fn regex(rule: &Scan, name: &str) -> Option<Verdict> {
    let pattern = rule.pattern.as_ref()?;

    // Compiled per file rather than once. A chain is a handful of rules and a
    // file is opened when somebody clicks, so this is nanoseconds in a place
    // that already waits for a process to start. Caching it would mean
    // threading a cache through, or a `Mutex` in a hot-looking path, for no
    // measurable gain.
    let Ok(regex) = Regex::new(pattern) else {
        // A pattern that does not compile is a mistake in the config, and
        // failing open on it is wrong: the rule was written to catch
        // something. Warn, name the rule, and let the person fix it.
        eprintln!("ricedir: scan rule `{}` has an invalid pattern", rule.name);
        return Some(Verdict::Warn);
    };

    regex.is_match(name).then_some(rule.verdict)
}

/// Whether the name and the content disagree.
///
/// The shape of the problem this program exists to notice: something arrives
/// called `invoice.pdf` and is an executable. Only a disagreement counts --
/// a file the database cannot name, or cannot sniff, says nothing either way,
/// because half the files on a disk are unrecognised and none of that is
/// suspicious.
fn mismatch(rule: &Scan, path: &Path) -> Option<Verdict> {
    let database = mime::database();
    let name = path.file_name()?.to_str()?;

    let claimed = database.by_name(name)?;
    let bytes = mime::head(path).ok()?;
    let actual = mime::sniff(&bytes)?;

    let claimed = database.canonical(claimed);
    let actual = database.canonical(actual);

    if claimed == actual {
        return None;
    }

    // Plenty of honest files sniff as something broader than their name says.
    // A `.docx` really is a zip; a `.py` really is text. Treating those as a
    // lie would make the rule useless within a minute of being switched on.
    if mime::plausible(claimed, actual) {
        return None;
    }

    Some(rule.verdict)
}

fn size(rule: &Scan, path: &Path) -> Option<()> {
    let over = rule.over?;
    let metadata = std::fs::symlink_metadata(path).ok()?;
    (metadata.len() > over).then_some(())
}

/// Ask an external scanner, and judge it by its exit code.
async fn command(rule: &Scan, path: &Path) -> Option<Verdict> {
    let (program, arguments) = rule.run.split_first()?;

    let deadline = Duration::from_secs(rule.timeout.unwrap_or(5));
    let arguments: Vec<std::ffi::OsString> = arguments
        .iter()
        .map(|argument| super::substitute(argument, path))
        .collect();

    let run = tokio::process::Command::new(program)
        .args(&arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        // The timeout below drops this future, which on its own leaves the
        // process running: a scanner that hangs would otherwise leave one
        // behind every time a file was opened.
        .kill_on_drop(true)
        .output();

    let output = match tokio::time::timeout(deadline, run).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            // A scanner that will not start is a broken config, not a verdict
            // about the file. Saying so beats silently allowing everything,
            // which is what a missing scanner would otherwise mean.
            eprintln!("ricedir: scan rule `{}` could not run: {error}", rule.name);
            return Some(Verdict::Warn);
        }
        Err(_) => {
            eprintln!(
                "ricedir: scan rule `{}` timed out after {deadline:?}",
                rule.name
            );
            return Some(Verdict::Warn);
        }
    };

    // A killed scanner has no code. It did not finish, so it did not clear
    // the file either.
    let Some(code) = output.status.code() else {
        return Some(Verdict::Warn);
    };

    if rule.block_on.contains(&code) {
        return Some(Verdict::Block);
    }
    if rule.warn_on.contains(&code) {
        return Some(Verdict::Warn);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Scan;

    fn rule(kind: Kind, pattern: &str, verdict: Verdict) -> Scan {
        Scan {
            name: String::from("test"),
            kind,
            pattern: Some(pattern.to_owned()),
            verdict,
            ..Scan::default()
        }
    }

    fn verdict(rules: &[Scan], path: &str) -> Report {
        // No `command` rule in these, so nothing here actually awaits.
        futures_lite_block(run(rules, Path::new(path)))
    }

    /// A tiny executor, so these tests need no runtime. Every rule under test
    /// is synchronous; only `Kind::Command` would suspend.
    fn futures_lite_block<T>(future: impl std::future::Future<Output = T>) -> T {
        use std::task::{Context, Poll, Waker};
        let mut future = Box::pin(future);
        let mut context = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
                return value;
            }
        }
    }

    /// The double extension that arrives in mail, which is the whole reason
    /// the chain exists.
    #[test]
    fn a_double_extension_is_caught() {
        let rules = [rule(
            Kind::Regex,
            r"\.(pdf|jpe?g|docx?)\.(exe|scr|js|sh)$",
            Verdict::Block,
        )];

        assert!(verdict(&rules, "/tmp/invoice.pdf.exe").blocked());
        assert!(!verdict(&rules, "/tmp/invoice.pdf").blocked());
        assert!(!verdict(&rules, "/tmp/notes.txt").blocked());
    }

    /// An empty chain allows everything, which is what an unconfigured
    /// ricedir has to do.
    #[test]
    fn no_rules_allows() {
        let report = verdict(&[], "/tmp/anything");
        assert_eq!(report.verdict, Verdict::Allow);
        assert!(report.rule.is_none());
    }

    /// The strongest verdict wins rather than the first, so a warning early
    /// in the file cannot hide a block later in it.
    #[test]
    fn a_block_outranks_an_earlier_warning() {
        let rules = [
            rule(Kind::Glob, "*.exe", Verdict::Warn),
            rule(Kind::Regex, r"\.exe$", Verdict::Block),
        ];

        let report = verdict(&rules, "/tmp/thing.exe");
        assert_eq!(report.verdict, Verdict::Block);
    }

    /// And the order does not matter, which is the point of taking the
    /// strongest rather than the first.
    #[test]
    fn a_block_wins_from_either_position() {
        let rules = [
            rule(Kind::Regex, r"\.exe$", Verdict::Block),
            rule(Kind::Glob, "*.exe", Verdict::Warn),
        ];

        assert_eq!(verdict(&rules, "/tmp/thing.exe").verdict, Verdict::Block);
    }

    /// A refusal has to name the rule, or there is nothing to go and edit.
    #[test]
    fn a_verdict_names_its_rule() {
        let mut only = rule(Kind::Glob, "*.scr", Verdict::Block);
        only.name = String::from("screensavers are not documents");

        let report = verdict(&[only], "/tmp/holiday.scr");
        assert_eq!(
            report.rule.as_deref(),
            Some("screensavers are not documents")
        );
        assert!(report.detail.is_some());
    }

    /// A pattern that does not compile is a mistake in the config. Failing
    /// open would silently drop a rule somebody wrote on purpose.
    #[test]
    fn a_broken_pattern_warns_rather_than_allowing() {
        let rules = [rule(Kind::Regex, "(unclosed", Verdict::Block)];
        assert_eq!(verdict(&rules, "/tmp/anything").verdict, Verdict::Warn);
    }

    /// A rule whose verdict is `allow` is a rule that says nothing, and must
    /// not stop the chain or claim credit for the outcome.
    #[test]
    fn an_allow_rule_says_nothing() {
        let rules = [
            rule(Kind::Glob, "*.exe", Verdict::Allow),
            rule(Kind::Glob, "*.exe", Verdict::Block),
        ];

        let report = verdict(&rules, "/tmp/thing.exe");
        assert_eq!(report.verdict, Verdict::Block);
    }

    /// A `glob` rule with no pattern cannot match anything, and must not
    /// match everything.
    #[test]
    fn a_rule_with_no_pattern_matches_nothing() {
        let bare = Scan {
            name: String::from("bare"),
            kind: Kind::Glob,
            verdict: Verdict::Block,
            ..Scan::default()
        };

        assert_eq!(verdict(&[bare], "/tmp/anything").verdict, Verdict::Allow);
    }

    /// A size rule reads the file it is given, so a path that is not there
    /// says nothing rather than blocking everything.
    #[test]
    fn a_size_rule_on_a_missing_file_says_nothing() {
        let big = Scan {
            name: String::from("huge"),
            kind: Kind::Size,
            over: Some(10),
            verdict: Verdict::Block,
            ..Scan::default()
        };

        let report = verdict(&[big], "/tmp/ricedir-no-such-file-at-all");
        assert_eq!(report.verdict, Verdict::Allow);
    }
}
