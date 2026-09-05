//! The Elm loop: state, messages, update, view.
//!
//! Stage 1 of M1 is one window showing one buffer. Tiles, the places panel and
//! the jobs panel arrive in stage 3, so what is here is deliberately the least
//! that can exercise the list widget against a real directory.

use std::collections::HashMap;
use std::path::PathBuf;

use iced::widget::{column, container, row, text};
use iced::{Element, Length, Task, window};

use crate::buffer::{self, Buffer, Listing};
use crate::config::Config;
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
}

#[derive(Debug, Clone)]
pub enum Message {
    Opened(window::Id),
    Closed(window::Id),
    /// A chunk, or the end, of a listing. The generation says which listing,
    /// so a chunk from one that has been replaced can be dropped.
    Listed(usize, u64, buffer::Update),
    List(usize, list::Action),
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
                        app.notice = Some(String::from("opening files lands in M1 stage 2"));
                        return Task::none();
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

    column![path_bar(buffer), body, status(app, buffer)]
        .width(Length::Fill)
        .height(Length::Fill)
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

    /// Activating a file cannot open it until stage 2, and the person has to
    /// be told rather than left wondering why nothing happened.
    #[test]
    fn activating_a_file_says_so_rather_than_nothing() {
        let mut app = app();
        let list = Config::default().list;
        app.buffers[0].extend(vec![entry("notes.md", Kind::File)], &list);
        app.buffers[0].finish(&list);

        tell(&mut app, Message::List(0, list::Action::Activate(0)));
        assert!(app.notice.is_some(), "no notice after opening a file");
        assert_eq!(app.buffers[0].path, PathBuf::from("/tmp/one"));
    }

    /// A message naming a buffer that is gone must not panic.
    #[test]
    fn a_message_for_a_missing_buffer_is_ignored() {
        let mut app = app();
        tell(&mut app, Message::List(9, list::Action::SelectAll));
        tell(&mut app, Message::Listed(9, 0, buffer::Update::Done));
    }
}
