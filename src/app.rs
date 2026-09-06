//! The Elm loop: state, messages, update, view.
//!
//! Stage 1 of M1 is one window showing one buffer. Tiles, the places panel and
//! the jobs panel arrive in stage 3, so what is here is deliberately the least
//! that can exercise the list widget against a real directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use iced::widget::{column, container, row, text};
use iced::{Element, Length, Task, window};

use crate::buffer::{self, Buffer, Listing};
use crate::config::Config;
use crate::dialogue::{self, Choice, Dialogue};
use crate::open::{self, Plan, scan};
use crate::widget::list::{self, FileList};

pub struct App {
    config: Config,
    /// Every open directory. Tiles will hold indices into this, exactly as
    /// ricebar's bars hold indices into one module list, so a directory open
    /// twice is listed once and watched once.
    buffers: Vec<Buffer>,
    /// Which buffer each window is showing. One entry until tiles land.
    windows: HashMap<window::Id, usize>,
    /// Anything the person needs to be told, since a file manager started from
    /// a launcher has no terminal to print to.
    notice: Option<String>,
    /// The one modal, when there is one.
    dialogue: Option<Dialogue>,
    /// What has been typed into the dialogue's program box.
    typed: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    Opened(window::Id),
    Closed(window::Id),
    /// A chunk, or the end, of a listing. The generation says which listing,
    /// so a chunk from one that has been replaced can be dropped.
    Listed(usize, u64, buffer::Update),
    List(usize, list::Action),
    /// The scan chain finished on a file somebody asked to open.
    Scanned(PathBuf, scan::Report),
    /// A handler was started, or would not start.
    Spawned(Result<(), String>),
    Dialogue(Choice),
}

pub fn new(config: Config, start: PathBuf) -> (App, Task<Message>) {
    let (id, opened) = window::open(window::Settings {
        size: iced::Size::new(config.window.width, config.window.height),
        min_size: Some(iced::Size::new(480.0, 320.0)),
        ..window::Settings::default()
    });

    let notice = config.problem.clone();
    let mut app = App {
        config,
        buffers: vec![Buffer::new(start.clone())],
        windows: HashMap::new(),
        notice,
        dialogue: None,
        typed: String::new(),
    };

    // The id comes back before the window exists, so the buffer can be tied to
    // it now rather than in a later message.
    app.windows.insert(id, 0);

    let listing = relist(&mut app, 0);
    (app, Task::batch([opened.map(Message::Opened), listing]))
}

pub fn title(app: &App, window: window::Id) -> String {
    match buffer_of(app, window) {
        Some(buffer) => format!("{} — ricedir", buffer.path.display()),
        None => String::from("ricedir"),
    }
}

pub fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::Opened(_) => Task::none(),

        Message::Closed(id) => {
            app.windows.remove(&id);
            if app.windows.is_empty() {
                // A daemon does not stop when the last window closes.
                return iced::exit();
            }
            Task::none()
        }

        Message::Listed(index, generation, update) => {
            let list = app.config.list.clone();
            let Some(buffer) = app.buffers.get_mut(index) else {
                return Task::none();
            };

            // A chunk from a listing that has since been replaced would put
            // the old directory's entries into the new one.
            if generation != buffer.generation {
                return Task::none();
            }

            match update {
                buffer::Update::Entries(entries) => buffer.extend(entries, &list),
                buffer::Update::Done => buffer.finish(&list),
                buffer::Update::Failed(problem) => {
                    app.notice = Some(format!("{}: {problem}", buffer.path.display()));
                    buffer.fail(problem);
                }
            }

            Task::none()
        }

        Message::List(index, action) => {
            let list = app.config.list.clone();
            let Some(buffer) = app.buffers.get_mut(index) else {
                return Task::none();
            };

            match action {
                list::Action::Select(row) => buffer.select_only(row),
                list::Action::Toggle(row) => buffer.toggle(row),
                list::Action::Extend(row) => buffer.extend_to(row),
                list::Action::Cursor(row) => buffer.move_to(row),
                list::Action::SelectAll => buffer.select_all(),
                // The menu is stage 3; the click still moves the cursor, so
                // right-clicking does something rather than nothing.
                list::Action::Menu { row, .. } => buffer.move_to(row),

                list::Action::Activate(row) => {
                    // Opening a file is stage 2. Entering a directory is the
                    // half that stage 1 needs.
                    let into = buffer
                        .at(row)
                        .filter(|entry| entry.kind.is_directory())
                        .map(|entry| entry.path.clone());

                    let Some(into) = into else {
                        // A file, not a directory. The scan chain gets its say
                        // before anything is chosen to run it with.
                        let Some(path) = buffer.at(row).map(|entry| entry.path.clone()) else {
                            return Task::none();
                        };
                        return scanned(app, path);
                    };

                    let from = std::mem::replace(&mut buffer.path, into);
                    buffer.history.push(from);
                    buffer.future.clear();
                    return relist(app, index);
                }

                list::Action::Leave => {
                    let Some(parent) = buffer.path.parent().map(PathBuf::from) else {
                        return Task::none();
                    };

                    let from = std::mem::replace(&mut buffer.path, parent);
                    buffer.history.push(from);
                    buffer.future.clear();
                    return relist(app, index);
                }
            }

            let _ = list;
            Task::none()
        }

        Message::Scanned(path, report) => {
            use crate::config::Verdict;

            match report.verdict {
                Verdict::Block => {
                    app.dialogue = Some(Dialogue::Blocked {
                        path,
                        rule: report.rule.unwrap_or_else(|| String::from("a scan rule")),
                        detail: report.detail,
                    });
                    Task::none()
                }

                Verdict::Warn => match open::plan(&app.config, &path) {
                    // A warning about a file nothing opens anyway is one
                    // dialogue too many: the handler question is the useful
                    // one, and the warning rides along in it.
                    plan @ Plan::Run { .. } => {
                        app.dialogue = Some(Dialogue::Warned {
                            path,
                            plan: Box::new(plan),
                            rule: report.rule.unwrap_or_else(|| String::from("a scan rule")),
                            detail: report.detail,
                        });
                        Task::none()
                    }
                    other => {
                        app.dialogue = Some(from_plan(path, other));
                        Task::none()
                    }
                },

                Verdict::Allow => match open::plan(&app.config, &path) {
                    plan @ Plan::Run { .. } => start(plan),
                    other => {
                        app.dialogue = Some(from_plan(path, other));
                        Task::none()
                    }
                },
            }
        }

        Message::Spawned(result) => {
            if let Err(problem) = result {
                app.notice = Some(problem);
            }
            Task::none()
        }

        Message::Dialogue(choice) => chose(app, choice),
    }
}

/// Run the scan chain over a file, then come back with what it said.
fn scanned(app: &App, path: PathBuf) -> Task<Message> {
    let rules = app.config.scan.clone();

    Task::perform(
        async move {
            let report = scan::run(&rules, &path).await;
            (path, report)
        },
        |(path, report)| Message::Scanned(path, report),
    )
}

/// Start a handler, and report only a failure.
fn start(plan: Plan) -> Task<Message> {
    Task::perform(async move { open::spawn(&plan).await }, Message::Spawned)
}

/// Turn a plan that cannot run into the dialogue that asks about it.
fn from_plan(path: PathBuf, plan: Plan) -> Dialogue {
    match plan {
        Plan::Refused { mime } => Dialogue::Refused { path, mime },
        _ => Dialogue::NoHandler {
            path: path.clone(),
            mime: open::kind(&path),
        },
    }
}

/// What the dialogue's buttons do.
fn chose(app: &mut App, choice: Choice) -> Task<Message> {
    let Some(dialogue) = app.dialogue.take() else {
        return Task::none();
    };

    match (choice, dialogue) {
        (Choice::Typing(typed), held) => {
            // Still open; only the box changed.
            app.dialogue = Some(held);
            app.typed = typed;
            Task::none()
        }

        (Choice::Dismiss, _) => {
            app.typed.clear();
            Task::none()
        }

        (Choice::Anyway, Dialogue::Warned { plan, .. }) => start(*plan),

        (Choice::Once, Dialogue::NoHandler { path, .. }) => start(xdg_open(&path)),

        (Choice::Always, Dialogue::NoHandler { path, mime }) => {
            // Remember it before running it, so the answer holds even if the
            // handler itself then fails.
            remember(app, &path, mime.as_deref(), &["xdg-open".to_owned()]);
            start(xdg_open(&path))
        }

        (Choice::Named, Dialogue::NoHandler { path, mime }) => {
            let typed = std::mem::take(&mut app.typed);
            let program = typed.trim();

            if program.is_empty() {
                app.dialogue = Some(Dialogue::NoHandler { path, mime });
                return Task::none();
            }

            // Split on spaces so `foot -e nvim` works, and each word stays its
            // own argv element. There is no shell, so nothing else happens to
            // what was typed.
            let run: Vec<String> = program.split_whitespace().map(str::to_owned).collect();
            remember(app, &path, mime.as_deref(), &run);

            let Some((program, rest)) = run.split_first() else {
                return Task::none();
            };
            let mut arguments: Vec<std::ffi::OsString> =
                rest.iter().map(std::ffi::OsString::from).collect();
            arguments.push(std::ffi::OsString::from("--"));
            arguments.push(path.as_os_str().to_owned());

            start(Plan::Run {
                name: program.clone(),
                program: std::ffi::OsString::from(program),
                arguments,
                directory: None,
            })
        }

        // Every other pairing is a button the dialogue did not draw.
        (_, held) => {
            app.dialogue = Some(held);
            Task::none()
        }
    }
}

fn xdg_open(path: &Path) -> Plan {
    Plan::Run {
        name: String::from("xdg-open"),
        program: std::ffi::OsString::from("xdg-open"),
        arguments: vec![std::ffi::OsString::from("--"), path.as_os_str().to_owned()],
        directory: None,
    }
}

/// Write a chosen handler into the config, and say so.
///
/// Keyed on the MIME type where there is one and the extension otherwise, so
/// "always" means what it says even for a type the database has never heard
/// of. A file with neither cannot be remembered, and the notice says so
/// rather than letting the promise quietly fail.
fn remember(app: &mut App, path: &Path, mime: Option<&str>, run: &[String]) {
    use crate::config::Matcher;

    let Some(config) = app.config.path.clone() else {
        return;
    };

    let Some(matcher) = Matcher::of(path, mime) else {
        app.notice = Some(String::from(
            "opened it, but there is no type or extension to remember it by",
        ));
        return;
    };

    match crate::config::append_handler(&config, &matcher, run) {
        Ok(()) => {
            app.notice = Some(format!(
                "{} now opens with `{}`",
                matcher.describe(),
                run.join(" ")
            ));
        }
        Err(error) => {
            app.notice = Some(format!("could not write {}: {error}", config.display()));
        }
    }
}

pub fn view(app: &App, window: window::Id) -> Element<'_, Message> {
    let Some(index) = app.windows.get(&window).copied() else {
        return text("no buffer").into();
    };
    let Some(buffer) = app.buffers.get(index) else {
        return text("no buffer").into();
    };

    let list = FileList::new(
        buffer,
        &app.config.theme,
        &app.config.list,
        app.config.window.font_size,
        true,
        move |action| Message::List(index, action),
    );

    let body = container(list).width(Length::Fill).height(Length::Fill);

    let page = column![path_bar(buffer), body, status(app, buffer)]
        .width(Length::Fill)
        .height(Length::Fill);

    let Some(dialogue) = &app.dialogue else {
        return page.into();
    };

    // A `Stack` rather than a second surface: iced's own popups take a grab,
    // and a modal that starves the window of pointer events is worse than no
    // modal at all -- a lesson ricebar paid for with its tooltips.
    iced::widget::stack![
        page,
        dialogue::view(dialogue, &app.config.theme, &app.typed).map(Message::Dialogue),
    ]
    .into()
}

pub fn theme(app: &App, _window: window::Id) -> iced::Theme {
    // A palette of our own, rather than one of iced's: the config names six
    // semantic colours and every widget here takes them directly.
    iced::Theme::custom(
        String::from("ricedir"),
        iced::theme::Palette {
            background: app.config.theme.background.color(),
            text: app.config.theme.foreground.color(),
            primary: app.config.theme.accent.color(),
            success: app.config.theme.accent.color(),
            warning: app.config.theme.urgent.color(),
            danger: app.config.theme.urgent.color(),
        },
    )
}

pub fn subscription(_app: &App) -> iced::Subscription<Message> {
    window::close_events().map(Message::Closed)
}

fn path_bar(buffer: &Buffer) -> Element<'_, Message> {
    container(text(buffer.path.display().to_string()).size(14))
        .padding(8)
        .width(Length::Fill)
        .into()
}

fn status<'a>(app: &'a App, buffer: &'a Buffer) -> Element<'a, Message> {
    // A notice outranks the counts: it is there because something went wrong
    // and there is no terminal to say so in.
    if let Some(notice) = &app.notice {
        return container(text(notice.clone()).size(13))
            .padding(8)
            .width(Length::Fill)
            .into();
    }

    let selected = buffer.selection.len();
    let counts = match buffer.listing {
        Listing::Loading => format!("{} items, reading…", buffer.rows()),
        Listing::Failed(_) => String::from("unreadable"),
        Listing::Ready if selected > 0 => {
            format!("{} items, {selected} selected", buffer.rows())
        }
        Listing::Ready => format!("{} items", buffer.rows()),
    };

    container(row![text(counts).size(13)].spacing(12))
        .padding(8)
        .width(Length::Fill)
        .into()
}

fn buffer_of(app: &App, window: window::Id) -> Option<&Buffer> {
    app.windows
        .get(&window)
        .and_then(|index| app.buffers.get(*index))
}

/// Start reading a buffer's directory again, abandoning whatever was running.
fn relist(app: &mut App, index: usize) -> Task<Message> {
    let Some(buffer) = app.buffers.get_mut(index) else {
        return Task::none();
    };

    buffer.generation = buffer.generation.wrapping_add(1);
    let generation = buffer.generation;

    *buffer = {
        let mut fresh = Buffer::new(buffer.path.clone());
        fresh.generation = generation;
        fresh.history = std::mem::take(&mut buffer.history);
        fresh.future = std::mem::take(&mut buffer.future);
        fresh
    };

    let path = buffer.path.clone();
    buffer::list(path).map(move |update| Message::Listed(index, generation, update))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{Entry, Kind};

    /// Drive `update` and drop the task: what these are about is the state it
    /// leaves behind, not the work it asks for.
    fn tell(app: &mut App, message: Message) {
        let _ = update(app, message);
    }

    fn app() -> App {
        App {
            config: Config::default(),
            buffers: vec![Buffer::new(PathBuf::from("/tmp/one"))],
            windows: HashMap::new(),
            notice: None,
            dialogue: None,
            typed: String::new(),
        }
    }

    fn entry(name: &str, kind: Kind) -> Entry {
        Entry {
            name: name.to_owned(),
            path: PathBuf::from("/tmp/one").join(name),
            kind,
            size: 0,
            modified: None,
            mode: 0o644,
            target: None,
            hidden: false,
        }
    }

    /// A chunk from a listing that has been replaced would otherwise put the
    /// old directory's entries into the new one.
    #[test]
    fn a_stale_chunk_is_dropped() {
        let mut app = app();
        app.buffers[0].generation = 7;

        tell(
            &mut app,
            Message::Listed(
                0,
                3,
                buffer::Update::Entries(vec![entry("old", Kind::File)]),
            ),
        );
        assert_eq!(app.buffers[0].rows(), 0);

        tell(
            &mut app,
            Message::Listed(
                0,
                7,
                buffer::Update::Entries(vec![entry("new", Kind::File)]),
            ),
        );
        assert_eq!(app.buffers[0].rows(), 1);
    }

    /// Backspace goes up, and remembers where it came from.
    #[test]
    fn leaving_a_directory_goes_to_the_parent() {
        let mut app = app();
        tell(&mut app, Message::List(0, list::Action::Leave));

        assert_eq!(app.buffers[0].path, PathBuf::from("/tmp"));
        assert_eq!(app.buffers[0].history, [PathBuf::from("/tmp/one")]);
    }

    /// There is nowhere above `/`, and asking must not panic or clear history.
    #[test]
    fn leaving_the_root_does_nothing() {
        let mut app = app();
        app.buffers[0].path = PathBuf::from("/");

        tell(&mut app, Message::List(0, list::Action::Leave));
        assert_eq!(app.buffers[0].path, PathBuf::from("/"));
        assert!(app.buffers[0].history.is_empty());
    }

    /// Activating a file must not move the buffer anywhere. Where it goes
    /// next is the scan chain's business, which happens in a task.
    #[test]
    fn activating_a_file_does_not_navigate() {
        let mut app = app();
        let list = Config::default().list;
        app.buffers[0].extend(vec![entry("notes.md", Kind::File)], &list);
        app.buffers[0].finish(&list);

        tell(&mut app, Message::List(0, list::Action::Activate(0)));
        assert_eq!(app.buffers[0].path, PathBuf::from("/tmp/one"));
        assert!(app.dialogue.is_none(), "the dialogue waits for the scan");
    }

    /// A blocked file raises a dialogue that names the rule, because a person
    /// told only "blocked" has nothing to go and edit.
    #[test]
    fn a_blocked_file_names_the_rule_that_refused() {
        use crate::config::Verdict;

        let mut app = app();
        tell(
            &mut app,
            Message::Scanned(
                PathBuf::from("/tmp/one/invoice.pdf.exe"),
                scan::Report {
                    verdict: Verdict::Block,
                    rule: Some(String::from("double extension")),
                    detail: Some(String::from("it ends .pdf.exe")),
                },
            ),
        );

        let Some(Dialogue::Blocked { rule, .. }) = &app.dialogue else {
            panic!("expected a blocked dialogue, got {:?}", app.dialogue);
        };
        assert_eq!(rule, "double extension");
    }

    /// Nothing in the config opens it, so the question is which program --
    /// and the answer must not be silence.
    #[test]
    fn an_unopenable_file_asks() {
        use crate::config::Verdict;

        let mut app = app();
        tell(
            &mut app,
            Message::Scanned(
                PathBuf::from("/tmp/one/mystery.zip"),
                scan::Report::allowed(),
            ),
        );

        assert!(
            matches!(app.dialogue, Some(Dialogue::NoHandler { .. })),
            "got {:?}",
            app.dialogue
        );
        let _ = Verdict::Allow;
    }

    /// Typing in the dialogue must not close it, or the box would lose a
    /// character at a time.
    #[test]
    fn typing_leaves_the_dialogue_open() {
        let mut app = app();
        app.dialogue = Some(Dialogue::NoHandler {
            path: PathBuf::from("/tmp/one/mystery"),
            mime: None,
        });

        tell(
            &mut app,
            Message::Dialogue(Choice::Typing(String::from("em"))),
        );
        assert!(app.dialogue.is_some());
        assert_eq!(app.typed, "em");
    }

    /// Cancelling closes it and forgets what was typed, so the next file does
    /// not inherit half a program name.
    #[test]
    fn cancelling_clears_the_box() {
        let mut app = app();
        app.typed = String::from("emacs");
        app.dialogue = Some(Dialogue::NoHandler {
            path: PathBuf::from("/tmp/one/mystery"),
            mime: None,
        });

        tell(&mut app, Message::Dialogue(Choice::Dismiss));
        assert!(app.dialogue.is_none());
        assert!(app.typed.is_empty());
    }

    /// An empty box is not a program. Pressing the button must leave the
    /// dialogue up rather than running nothing.
    #[test]
    fn an_empty_program_box_does_nothing() {
        let mut app = app();
        app.dialogue = Some(Dialogue::NoHandler {
            path: PathBuf::from("/tmp/one/mystery"),
            mime: None,
        });

        tell(&mut app, Message::Dialogue(Choice::Named));
        assert!(app.dialogue.is_some(), "the dialogue should stay up");
    }

    /// A message naming a buffer that is gone must not panic.
    #[test]
    fn a_message_for_a_missing_buffer_is_ignored() {
        let mut app = app();
        tell(&mut app, Message::List(9, list::Action::SelectAll));
        tell(&mut app, Message::Listed(9, 0, buffer::Update::Done));
    }
}
