//! The Elm loop: state, messages, update, view.
//!
//! Stage 1 of M1 is one window showing one buffer. Tiles, the places panel and
//! the jobs panel arrive in stage 3, so what is here is deliberately the least
//! that can exercise the list widget against a real directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use iced::widget::{column, container, row, text};
use iced::{Element, Length, Task, window};

use crate::action::{self, Action};
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
    /// Whether the filter box is on screen. Hidden until asked for, because a
    /// box that is always there is a box that is always in the way.
    filtering: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Opened(window::Id),
    Closed(window::Id),
    /// A chunk, or the end, of a listing. The generation says which listing,
    /// so a chunk from one that has been replaced can be dropped.
    Listed(usize, u64, buffer::Update),
    List(usize, list::Action),
    /// An action from anywhere that is not the list widget.
    Act(Action),
    /// The directory a buffer is showing changed on disk.
    Changed(usize),
    /// The filter box was typed into.
    Filter(usize, String),
    /// Show or hide the filter box.
    Filtering(bool),
    /// A breadcrumb, or back, forward, up.
    Go(usize, PathBuf),
    Back(usize),
    Forward(usize),
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
        filtering: false,
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

        // Every request becomes an `Action` and goes through the registry.
        // The widget says what happened; `action` says what it means.
        Message::List(index, found) => {
            let Some(action) = translate(index, found) else {
                return Task::none();
            };
            action::dispatch(app, action)
        }

        Message::Act(action) => action::dispatch(app, action),

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

        Message::Changed(index) => relist(app, index),

        Message::Filter(index, text) => action::dispatch(
            app,
            Action::Filter {
                buffer: index,
                text,
            },
        ),

        Message::Filtering(showing) => {
            app.filtering = showing;

            if showing {
                // A box that appears without focus is a box that swallows the
                // first thing typed into it -- `/` then `file1` put the box on
                // screen and the text nowhere. `focus` unfocuses everything
                // else in the same traversal, so nothing has to be told to let
                // go first.
                return iced::widget::operation::focus(filter_id());
            }

            if !showing {
                // Leaving the box behind with text in it would leave the
                // listing narrowed and nothing on screen saying why.
                let list = app.config.list.clone();
                for buffer in &mut app.buffers {
                    if !buffer.filter.is_empty() {
                        buffer.filter.clear();
                        buffer.rebuild(&list);
                    }
                }
            }
            Task::none()
        }

        Message::Go(index, into) => action::dispatch(
            app,
            Action::Go {
                buffer: index,
                path: into,
            },
        ),
        Message::Back(index) => action::dispatch(app, Action::Back { buffer: index }),
        Message::Forward(index) => action::dispatch(app, Action::Forward { buffer: index }),
    }
}

/// What a click or a key from the list widget means.
///
/// The widget reports what happened to it; this says which action that is.
/// Kept apart so the widget knows nothing about buffers or history.
fn translate(buffer: usize, found: list::Action) -> Option<Action> {
    Some(match found {
        list::Action::Select(row) => Action::Select { buffer, row },
        list::Action::Toggle(row) => Action::Toggle { buffer, row },
        list::Action::Extend(row) => Action::Extend { buffer, row },
        list::Action::Cursor(row) => Action::Select { buffer, row },
        list::Action::SelectAll => Action::SelectAll { buffer },
        list::Action::Activate(row) => Action::Activate { buffer, row },
        list::Action::Leave => Action::Leave { buffer },
        list::Action::Filter => Action::Filtering(true),
        list::Action::Escape => Action::Escape,
        // The menu is still to come. The click moves the cursor meanwhile, so
        // a right click does something rather than nothing.
        list::Action::Menu { row, .. } => Action::Select { buffer, row },
    })
}

/// Say no, and say why.
///
/// Every refusal goes through here, so a person always learns which rule
/// stopped them rather than watching nothing happen.
pub fn refuse(app: &mut App, reason: String) -> Task<Message> {
    app.notice = Some(reason);
    Task::none()
}

/// Do what an action says.
///
/// Reached only through `action::dispatch`, which checks the kind first.
pub fn carry_out(app: &mut App, action: Action) -> Task<Message> {
    let list = app.config.list.clone();

    match action {
        // Session actions answer a question. In the window there is nobody
        // asking, so they do nothing; on the socket they will be answered
        // before they ever reach here.
        Action::Selection | Action::Buffers | Action::Cursor => Task::none(),

        Action::Filtering(showing) => Task::done(Message::Filtering(showing)),

        Action::Escape => {
            // One key for "put away whatever is in front of me", in the order
            // things are stacked.
            if app.dialogue.is_some() {
                return Task::done(Message::Dialogue(Choice::Dismiss));
            }
            Task::done(Message::Filtering(false))
        }

        Action::Select { buffer, row } => {
            with(app, buffer, |found| found.select_only(row));
            Task::none()
        }
        Action::Toggle { buffer, row } => {
            with(app, buffer, |found| found.toggle(row));
            Task::none()
        }
        Action::Extend { buffer, row } => {
            with(app, buffer, |found| found.extend_to(row));
            Task::none()
        }
        Action::SelectAll { buffer } => {
            with(app, buffer, Buffer::select_all);
            Task::none()
        }

        Action::Filter { buffer, text } => {
            with(app, buffer, |found| {
                found.filter = text;
                found.rebuild(&list);
            });
            Task::none()
        }

        Action::Go { buffer, path } => {
            let Some(found) = app.buffers.get_mut(buffer) else {
                return Task::none();
            };
            if found.path == path {
                return Task::none();
            }

            let from = std::mem::replace(&mut found.path, path);
            found.history.push(from);
            found.future.clear();
            relist(app, buffer)
        }

        Action::Leave { buffer } => {
            let Some(found) = app.buffers.get_mut(buffer) else {
                return Task::none();
            };
            let Some(parent) = found.path.parent().map(PathBuf::from) else {
                return Task::none();
            };

            let from = std::mem::replace(&mut found.path, parent);
            found.history.push(from);
            found.future.clear();
            relist(app, buffer)
        }

        Action::Back { buffer } => {
            let Some(found) = app.buffers.get_mut(buffer) else {
                return Task::none();
            };
            let Some(back) = found.history.pop() else {
                return Task::none();
            };

            let from = std::mem::replace(&mut found.path, back);
            found.future.push(from);
            relist(app, buffer)
        }

        Action::Forward { buffer } => {
            let Some(found) = app.buffers.get_mut(buffer) else {
                return Task::none();
            };
            let Some(forward) = found.future.pop() else {
                return Task::none();
            };

            let from = std::mem::replace(&mut found.path, forward);
            found.history.push(from);
            relist(app, buffer)
        }

        Action::Activate { buffer, row } => {
            let Some(found) = app.buffers.get_mut(buffer) else {
                return Task::none();
            };

            // A directory is entered. A file goes to the scan chain first,
            // and there is no other route to opening one.
            let into = found
                .at(row)
                .filter(|entry| entry.kind.is_directory())
                .map(|entry| entry.path.clone());

            let Some(into) = into else {
                let Some(path) = found.at(row).map(|entry| entry.path.clone()) else {
                    return Task::none();
                };
                return scanned(app, path);
            };

            let from = std::mem::replace(&mut found.path, into);
            found.history.push(from);
            found.future.clear();
            relist(app, buffer)
        }
    }
}

/// Do something to one buffer, if it is still there.
fn with(app: &mut App, buffer: usize, change: impl FnOnce(&mut Buffer)) {
    if let Some(found) = app.buffers.get_mut(buffer) {
        change(found);
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
        // Not focused while a dialogue is up: otherwise Escape would close
        // the dialogue and clear the filter in the same keystroke, and arrows
        // would move a cursor nobody can see.
        app.dialogue.is_none(),
        move |action| Message::List(index, action),
    );

    let body = container(list).width(Length::Fill).height(Length::Fill);

    let mut page = column![path_bar(app, index, buffer)];

    if app.filtering {
        page = page.push(
            container(
                iced::widget::text_input("filter, or :glob", &buffer.filter)
                    .id(filter_id())
                    .on_input(move |text| Message::Filter(index, text))
                    .on_submit(Message::Filtering(false))
                    .size(14)
                    .padding(6),
            )
            .padding([0, 4]),
        );
    }

    let page = page
        .push(body)
        .push(status(app, buffer))
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

pub fn subscription(app: &App) -> iced::Subscription<Message> {
    // Only the buffers on screen are watched. inotify allows 128 instances
    // here, and a long session opens more directories than that.
    let watching = app
        .windows
        .values()
        .collect::<std::collections::HashSet<_>>();

    let watches = watching.into_iter().filter_map(|index| {
        let buffer = app.buffers.get(*index)?;
        if buffer.listing == Listing::Loading {
            // Watching a directory still being read would ask for a relist
            // before the first one had finished.
            return None;
        }

        // `.with` rather than a closure that captures the index: iced hashes
        // a subscription's closure to identify it, and rejects a capturing one
        // outright. It is the same rule that makes two buffers sharing a
        // recipe collapse into one stream without it.
        Some(
            crate::watch::directory(buffer.path.clone(), buffer.generation)
                .with(*index)
                .map(|(index, ())| Message::Changed(index)),
        )
    });

    iced::Subscription::batch(
        watches.chain(std::iter::once(window::close_events().map(Message::Closed))),
    )
}

/// Breadcrumbs, and the two arrows.
///
/// Each component is a button rather than one long string, because the thing
/// people want from a path bar is to jump three levels up without typing.
fn path_bar<'a>(app: &'a App, index: usize, buffer: &'a Buffer) -> Element<'a, Message> {
    use iced::widget::button;

    let dim = app.config.theme.dim.color();
    // Takes an owned `String`: an earlier version leaked a `&'static str` per
    // component with `Box::leak`, which in a function that runs every frame is
    // a leak that grows for as long as the window is open.
    let quiet = move |label: String, message: Option<Message>| {
        let mut made =
            button(text(label).size(14))
                .padding([2, 6])
                .style(move |_: &iced::Theme, status| {
                    let hovered = matches!(status, button::Status::Hovered);
                    button::Style {
                        background: None,
                        text_color: if hovered { iced::Color::WHITE } else { dim },
                        ..button::Style::default()
                    }
                });
        if let Some(message) = message {
            made = made.on_press(message);
        }
        made
    };

    let mut crumbs = row![
        quiet(
            String::from("<"),
            (!buffer.history.is_empty()).then_some(Message::Back(index)),
        ),
        quiet(
            String::from(">"),
            (!buffer.future.is_empty()).then_some(Message::Forward(index)),
        ),
    ]
    .spacing(2)
    .align_y(iced::Alignment::Center);

    // Built from the components rather than by splitting the string, so a
    // directory with a slash-looking name in it cannot fool the crumbs.
    let mut walked = PathBuf::new();
    for component in buffer.path.components() {
        walked.push(component.as_os_str());

        let label = match component {
            std::path::Component::RootDir => String::from("/"),
            other => other.as_os_str().to_string_lossy().into_owned(),
        };

        crumbs = crumbs.push(quiet(label, Some(Message::Go(index, walked.clone()))));
    }

    container(crumbs).padding([4, 4]).width(Length::Fill).into()
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

    // Saying a filter is on matters more than the count: a listing that is
    // narrowed and does not say so looks like a directory that lost files.
    let counts = if buffer.filter.is_empty() {
        counts
    } else {
        format!("{counts}  ·  filtered by `{}`", buffer.filter)
    };

    container(row![text(counts).size(13)].spacing(12))
        .padding(8)
        .width(Length::Fill)
        .into()
}

/// The filter box, so it can be focused when it appears.
///
/// A fixed id rather than one per buffer: there is one box, and it belongs to
/// whichever tile is in front.
fn filter_id() -> iced::widget::Id {
    iced::widget::Id::new("ricedir-filter")
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
        // A relist because the directory changed must not also clear what was
        // typed into the filter box, or watching a directory would fight
        // whoever is filtering it.
        fresh.filter = std::mem::take(&mut buffer.filter);
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
            filtering: false,
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
