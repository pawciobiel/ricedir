//! The Elm loop: state, messages, update, view.
//!
//! Stage 1 of M1 is one window showing one buffer. Tiles, the places panel and
//! the jobs panel arrive in stage 3, so what is here is deliberately the least
//! that can exercise the list widget against a real directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use iced::widget::{column, container, pane_grid, row, text};
use iced::{Element, Length, Task, window};

use crate::action::{self, Action};
use crate::buffer::{self, Buffer, Listing};
use crate::config::Config;
use crate::dialogue::{self, Choice, Dialogue};
use crate::open::{self, Plan, scan};
use crate::places::{self, Place};
use crate::widget::list::{self, FileList};

/// The tiles in one window, and which of them has the keyboard.
///
/// A tile holds a `usize` into `App.buffers`, exactly as ricebar's bars hold
/// indices into one module list. Two tiles pointed at the same buffer share
/// its listing, its watcher and its selection -- which is the emacs move, and
/// the reason buffers and tiles are separate things.
pub struct Tiles {
    pub panes: pane_grid::State<usize>,
    pub focus: pane_grid::Pane,
}

impl Tiles {
    fn new(buffer: usize) -> Self {
        let (panes, first) = pane_grid::State::new(buffer);
        Self {
            panes,
            focus: first,
        }
    }

    /// The buffer the focused tile is showing.
    fn buffer(&self) -> usize {
        self.panes.get(self.focus).copied().unwrap_or(0)
    }
}

/// A menu, and where it was asked for.
#[derive(Debug, Clone)]
pub struct Menu {
    pub kind: MenuKind,
    pub buffer: usize,
    /// Window coordinates. The list widget knows where the click landed and a
    /// toolbar button knows where it is, so nothing has to track the cursor
    /// separately -- which is the usual answer when iced tells `update` no
    /// geometry.
    pub at: (f32, f32),
}

/// Which menu, and so what is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    /// A right click on a row, or past the last one.
    ///
    /// Empty space is not "no menu": paste, a new folder and the hidden-files
    /// switch are about the directory, and empty space is where people look
    /// for them. Offering the file items there instead, greyed out or acting
    /// on whatever happened to be selected, is the thing that makes a context
    /// menu untrustworthy.
    Context { row: Option<usize> },
    /// A right click on one of the places in the sidebar.
    Place { index: usize },
    /// Every open directory, to point this tile at one of them.
    Buffers,
    /// The toolbar's sort button.
    Sort,
    /// The toolbar's last button: everything the other buttons do, in words.
    Toolbar,
}

/// A box divided left and right, and a box divided top and bottom.
///
/// Codicons. Checked against the installed font and then rendered to see
/// which way round they go, because the names are `split-horizontal` and
/// `split-vertical` and neither says whether that is the divider or the
/// direction the panes sit in.
/// How wide the places and jobs panel is.
///
/// Named because the menus need it too: anything opened for a tile has to
/// start to the right of this or it covers the panel it is not about.
const SIDEBAR: f32 = 180.0;

const SPLIT_RIGHT: char = '\u{eb56}';
const SPLIT_DOWN: char = '\u{eb57}';

pub struct App {
    config: Config,
    /// Every open directory. Tiles will hold indices into this, exactly as
    /// ricebar's bars hold indices into one module list, so a directory open
    /// twice is listed once and watched once.
    buffers: Vec<Buffer>,
    /// The tiles in each window.
    windows: HashMap<window::Id, Tiles>,
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
    /// The path being typed, when the path bar is showing its text face.
    ///
    /// The draft rather than the buffer's path: what is typed has to survive
    /// being wrong -- a half-finished path names nothing, and replacing the
    /// buffer's own path with it would relist on every keystroke. It belongs
    /// to the focused tile, the way the filter box does.
    typing_path: Option<String>,
    /// The face the glyphs come from, resolved once at startup.
    ///
    /// `Font::with_name` wants a `&'static str` and the family comes from a
    /// config file, so the name is leaked -- once, not once per frame.
    icon_font: Option<iced::Font>,
    /// The context menu, when one is open.
    menu: Option<Menu>,
    /// Where the pointer was last seen, in window coordinates.
    ///
    /// The list widget is handed the cursor and can say where a right click
    /// landed, but a toolbar button cannot: iced tells `update` no geometry,
    /// so a button has no idea where on screen it is. Tracking the pointer is
    /// the documented answer, and a menu opened from a button belongs under
    /// the pointer that opened it anyway.
    pointer: (f32, f32),
    /// How big the window is now, so a menu can be kept inside it.
    ///
    /// `config.window` is the size it *opened* at. Anything that has to fit on
    /// screen needs the size it is, which only `resize_events` reports.
    size: iced::Size,
    /// Which step the reading bar is on. Moves only while a read is out.
    tick: usize,
    /// Home, the user directories, the mounts and the bookmarks.
    ///
    /// Read once at startup and after a bookmark is added. A disk appearing is
    /// worth a refresh too, which is M3's business once the socket exists to
    /// ask for one.
    places: Vec<Place>,
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
    Changed(usize, crate::watch::Change),
    /// The paths a change named, read on a thread and back with the answer.
    Examined(usize, u64, Vec<(PathBuf, Option<crate::entry::Entry>)>),
    /// The clock that moves the reading bar. Only runs while a read is out.
    Tick,
    /// The filter box was typed into.
    Filter(usize, String),
    /// Show or hide the filter box.
    Filtering(bool),
    /// The path bar's text face: what has been typed, and going there or back.
    PathTyped(String),
    TypingPath(usize, bool),
    /// The typed path was submitted.
    PathSubmitted(usize),
    /// Escape, from the subscription rather than the list. Only the path
    /// bar's text face uses it; everything else Escape does still comes
    /// through the list widget.
    EscapedPath,
    /// Tab, likewise: complete the path being typed.
    CompletePath,
    /// A breadcrumb, or back, forward, up.
    Go(usize, PathBuf),
    Back(usize),
    Forward(usize),
    /// The scan chain finished on a file somebody asked to open.
    Scanned(PathBuf, scan::Report),
    /// A handler was started, or would not start.
    Spawned(Result<(), String>),
    Dialogue(Choice),
    /// A tile was clicked, dragged onto another, or its divider moved.
    TileClicked(pane_grid::Pane),
    TileDragged(pane_grid::DragEvent),
    TileResized(pane_grid::ResizeEvent),
    /// The pointer moved. Only recorded, never acted on.
    Pointer(f32, f32),
    /// The window changed size.
    Resized(iced::Size),
}

pub fn new(config: Config, start: PathBuf) -> (App, Task<Message>) {
    let (id, opened) = window::open(window::Settings {
        size: iced::Size::new(config.window.width, config.window.height),
        min_size: Some(iced::Size::new(480.0, 320.0)),
        ..window::Settings::default()
    });

    let notice = config.problem.clone();
    let config_font = config.list.icon_font.clone();
    let (config_width, config_height) = (config.window.width, config.window.height);
    let config_view = config.list.view();
    let mut app = App {
        config,
        buffers: vec![Buffer::new(start, config_view)],
        windows: HashMap::new(),
        notice,
        dialogue: None,
        typed: String::new(),
        filtering: false,
        typing_path: None,
        icon_font: config_font.as_deref().map(icon_font),
        menu: None,
        pointer: (0.0, 0.0),
        size: iced::Size::new(config_width, config_height),
        places: places::list(),
        tick: 0,
    };

    // The id comes back before the window exists, so the tiles can be tied to
    // it now rather than in a later message.
    app.windows.insert(id, Tiles::new(0));

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
                buffer::Update::Done => {
                    buffer.finish(&list);

                    // Something changed while this listing was being read, so
                    // what just landed was already out of date when it
                    // arrived. Read it once more. Without this the change is
                    // lost until somebody presses F5.
                    if buffer.take_stale() {
                        return relist(app, index);
                    }
                }
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
            // Anything done to a tile focuses it first. The list widget
            // captures its own presses, so `pane_grid`'s `on_click` never
            // sees them and the keyboard would stay on whichever tile had it
            // -- you could select a row in one tile and then find the arrows
            // moving a cursor in the other.
            focus_showing(app, index);

            let showing = app
                .buffers
                .get(index)
                .map_or(app.config.list.layout, |found| found.view.layout);

            action::dispatch(app, translate(index, found, showing, app.pointer))
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

        Message::Pointer(x, y) => {
            app.pointer = (x, y);
            Task::none()
        }

        Message::Tick => {
            app.tick = app.tick.wrapping_add(1);
            Task::none()
        }

        Message::Resized(size) => {
            app.size = size;
            Task::none()
        }

        Message::TileClicked(pane) => {
            focus_tile(app, pane);
            Task::none()
        }

        Message::TileDragged(pane_grid::DragEvent::Dropped { pane, target }) => {
            for tiles in app.windows.values_mut() {
                tiles.panes.drop(pane, target);
            }
            Task::none()
        }
        Message::TileDragged(_) => Task::none(),

        Message::TileResized(pane_grid::ResizeEvent { split, ratio }) => {
            for tiles in app.windows.values_mut() {
                tiles.panes.resize(split, ratio);
            }
            Task::none()
        }

        Message::Changed(index, change) => {
            // One file saved used to cost a whole `read_dir`, one
            // `symlink_metadata` per entry and a full sort -- 114 CPU ticks
            // on a directory of 100,000, every time somebody pressed save.
            // Now it costs one `stat` per path the events named.
            let touched = match change {
                crate::watch::Change::Rescan => return relist(app, index),
                crate::watch::Change::Touched(paths) => paths,
            };

            // The reading is a blocking syscall per path, so it goes to a
            // thread and comes back as a message. On a local disk that is
            // microseconds and the round trip is the expensive half; over
            // sshfs it is the only thing keeping the window answering.
            let generation = app.buffers.get(index).map_or(0, |found| found.generation);
            buffer::examine(touched).map(move |read| Message::Examined(index, generation, read))
        }

        Message::Examined(index, generation, read) => {
            let list = app.config.list.clone();
            let Some(buffer) = app.buffers.get_mut(index) else {
                return Task::none();
            };

            // The buffer moved on while the paths were being read, so what
            // came back describes a directory it is no longer showing.
            if generation != buffer.generation {
                return Task::none();
            }

            // A `false` back means a replacement listing is already on its
            // way and will say what is there, so there is nothing to do.
            buffer.reconcile(read, &list);
            Task::none()
        }

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

        Message::TypingPath(index, typing) => {
            if !typing {
                app.typing_path = None;
                return Task::none();
            }

            // Starts as the path it is showing, so the common thing -- take a
            // piece of this path -- needs no typing at all.
            let Some(buffer) = app.buffers.get(index) else {
                return Task::none();
            };
            app.typing_path = Some(buffer.path.to_string_lossy().into_owned());

            // Focused *and* selected: turning the bar over to copy half a
            // path should not need a drag from one end first.
            iced::widget::operation::focus(path_id())
                .chain(iced::widget::operation::select_all(path_id()))
        }

        Message::PathTyped(text) => {
            app.typing_path = Some(text);
            Task::none()
        }

        Message::EscapedPath => {
            // Only this. Every other thing Escape puts away still comes
            // through the list widget's own `Action::Escape`.
            app.typing_path = None;
            Task::none()
        }

        Message::CompletePath => {
            let Some(typed) = &app.typing_path else {
                return Task::none();
            };
            let Some(longer) = complete(typed) else {
                return Task::none();
            };

            app.typing_path = Some(longer);
            // The caret goes to the end, or the next keystroke lands in the
            // middle of what was just filled in.
            iced::widget::operation::move_cursor_to_end(path_id())
        }

        Message::PathSubmitted(index) => {
            let Some(text) = app.typing_path.take() else {
                return Task::none();
            };

            let path = expand(&text);

            // A path that is not a directory says so and stays on screen with
            // what was typed still in it. Going nowhere and clearing the box
            // would look like the keystroke was lost.
            if !path.is_dir() {
                app.typing_path = Some(text);
                return refuse(app, format!("{} is not a directory", path.display()));
            }

            action::dispatch(
                app,
                Action::Go {
                    buffer: index,
                    path,
                },
            )
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
///
/// `pointer` is where the hand is. A menu opened by a key has no click to sit
/// under, and the last known pointer position is a better guess than a corner.
const fn translate(
    buffer: usize,
    found: list::Action,
    current: crate::config::Layout,
    pointer: (f32, f32),
) -> Action {
    match found {
        list::Action::Select(row) => Action::Select { buffer, row },
        list::Action::Toggle(row) => Action::Toggle { buffer, row },
        list::Action::Extend(row) => Action::Extend { buffer, row },
        list::Action::Cursor(row) => Action::Select { buffer, row },
        list::Action::SelectAll => Action::SelectAll { buffer },
        list::Action::Invert => Action::Invert { buffer },
        list::Action::Band {
            rows,
            columns,
            across,
            add,
        } => Action::Band {
            buffer,
            rows,
            columns,
            across,
            add,
        },
        list::Action::Activate(row) => Action::Activate { buffer, row },
        list::Action::Leave => Action::Leave { buffer },
        list::Action::Filter => Action::Filtering(true),
        list::Action::Layout(None) => Action::Layout {
            buffer,
            layout: current.next(),
        },
        list::Action::Layout(Some(layout)) => Action::Layout { buffer, layout },
        list::Action::SplitRight => Action::Split(pane_grid::Axis::Vertical),
        list::Action::SplitDown => Action::Split(pane_grid::Axis::Horizontal),
        list::Action::CloseTile => Action::CloseTile,
        list::Action::NextTile => Action::NextTile,
        list::Action::PreviousTile => Action::PreviousTile,
        list::Action::Escape => Action::Escape,
        list::Action::Bookmark => Action::Bookmark { buffer, path: None },
        list::Action::Buffers => Action::Menu {
            kind: MenuKind::Buffers,
            buffer,
            at: pointer,
        },
        list::Action::TypePath => Action::TypingPath {
            buffer,
            typing: true,
        },
        list::Action::Relist => Action::Relist { buffer },
        list::Action::Menu { row, at } => Action::Menu {
            kind: MenuKind::Context { row },
            buffer,
            at: (at.x, at.y),
        },
    }
}

/// A path short enough for a menu line, with `$HOME` written as `~`.
///
/// The buffer list is a column of paths and most of them start with the same
/// twelve characters, which is exactly the part that carries no information.
fn short(path: &Path) -> String {
    let full = path.to_string_lossy();

    let Some(home) = std::env::var_os("HOME") else {
        return full.into_owned();
    };
    let home = home.to_string_lossy();

    match full.strip_prefix(home.as_ref()) {
        Some("") => String::from("~"),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => full.into_owned(),
    }
}

/// Resolve the icon family, once.
///
/// `Font::with_name` wants a `&'static str` while the family comes from the
/// config, so the string is leaked. Once, at start-up, which is why this is
/// not called from `view`.
fn icon_font(family: &str) -> iced::Font {
    iced::Font::with_name(String::leak(family.to_owned()))
}

/// Give one tile the keyboard.
///
/// Every window is asked, because a `Pane` belongs to exactly one of them and
/// there is no cheaper way to say which from a click alone.
fn focus_tile(app: &mut App, pane: pane_grid::Pane) {
    for tiles in app.windows.values_mut() {
        if tiles.panes.get(pane).is_some() {
            tiles.focus = pane;
        }
    }
}

/// Give the keyboard to whichever tile is showing this buffer.
///
/// The first one found: two tiles can show one buffer, and then either will
/// do -- they are the same listing and the same cursor.
fn focus_showing(app: &mut App, buffer: usize) {
    for tiles in app.windows.values_mut() {
        let found = tiles
            .panes
            .iter()
            .find(|(_, index)| **index == buffer)
            .map(|(pane, _)| *pane);

        if let Some(pane) = found {
            tiles.focus = pane;
            return;
        }
    }
}

/// Put away the context menu, if one is open.
pub const fn close_menu(app: &mut App) {
    app.menu = None;
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

        Action::TypingPath { buffer, typing } => Task::done(Message::TypingPath(buffer, typing)),

        Action::Menu { kind, buffer, at } => {
            // A right click already moved the cursor. The menu acts on
            // whatever is selected, so what it will do is on screen before it
            // opens. A toolbar menu changes no selection at all.
            if let MenuKind::Context { row: Some(row) } = kind {
                with(app, buffer, |found| {
                    if found.is_selected(row) {
                        found.move_to(row);
                    } else {
                        found.select_only(row);
                    }
                });
            }

            app.menu = Some(Menu { kind, buffer, at });
            Task::none()
        }

        Action::SortBy(field) => {
            // Picking the field that is already sorted turns the order round,
            // which is what a column heading does everywhere else.
            if app.config.list.sort == field {
                app.config.list.sort_reversed = !app.config.list.sort_reversed;
            } else {
                app.config.list.sort = field;
                app.config.list.sort_reversed = false;
            }
            resort(app);
            Task::none()
        }

        Action::ReverseSort => {
            app.config.list.sort_reversed = !app.config.list.sort_reversed;
            resort(app);
            Task::none()
        }

        Action::CopyPath { buffer } => {
            let Some(found) = app.buffers.get(buffer) else {
                return Task::none();
            };

            // Every selected path, one per line, so pasting several into a
            // terminal or an editor gives a list rather than a run-on.
            let mut paths: Vec<String> = found
                .selected()
                .map(|entry| entry.path.display().to_string())
                .collect();

            // Nothing selected means the directory itself, which is what
            // somebody asking for "the path" from an empty patch of window
            // means.
            if paths.is_empty() {
                paths.push(found.path.display().to_string());
            }

            app.notice = Some(match paths.len() {
                1 => format!("copied {}", paths[0]),
                many => format!("copied {many} paths"),
            });
            iced::clipboard::write(paths.join("\n"))
        }

        Action::Bookmark { buffer, path } => {
            // `None` means the directory this buffer is showing. Resolved
            // here rather than at the caller, so a key press does not have to
            // carry a path the widget never saw.
            let Some(path) = path.or_else(|| app.buffers.get(buffer).map(|f| f.path.clone()))
            else {
                return Task::none();
            };

            match crate::places::bookmark(&path) {
                Ok(()) => {
                    app.places = places::list();
                    app.notice = Some(format!("{} is in your places", path.display()));
                }
                Err(error) => {
                    app.notice = Some(format!("could not write the bookmark: {error}"));
                }
            }
            Task::none()
        }

        Action::Unbookmark { path } => {
            match crate::places::unbookmark(&path) {
                Ok(()) => {
                    app.places = places::list();
                    app.notice = Some(format!("{} is no longer a place", path.display()));
                }
                Err(error) => {
                    app.notice = Some(format!("could not remove the bookmark: {error}"));
                }
            }
            Task::none()
        }

        Action::ShowHidden { buffer, showing } => {
            // Only this buffer, and only this buffer relists. The dotfiles
            // are part of the view, and a tile opened on a `.config` is no
            // reason for the other one to fill up with `.git` and `.cache`.
            let list = app.config.list.clone();
            with(app, buffer, |found| {
                found.view.show_hidden = showing;
                found.rebuild(&list);
            });
            Task::none()
        }

        Action::Layout { buffer, layout } => {
            with(app, buffer, |found| found.view.layout = layout);
            Task::none()
        }

        Action::ShowBuffer { buffer } => {
            if app.buffers.get(buffer).is_none() {
                return Task::none();
            }

            let Some(window) = app.windows.values_mut().next() else {
                return Task::none();
            };
            let focus = window.focus;
            if window.panes.get(focus).copied() == Some(buffer) {
                // Already showing it. Relisting would be a surprise.
                return Task::none();
            }
            if let Some(showing) = window.panes.get_mut(focus) {
                *showing = buffer;
            }

            // Only visible buffers are watched -- inotify allows 128
            // instances here -- so one that has been out of sight is showing
            // whatever the directory held when it was last on screen. This is
            // the only way a buffer becomes visible again, so it is the only
            // place that has to notice.
            relist(app, buffer)
        }

        Action::CloseBuffer { buffer } => {
            let showing = |app: &App, index: usize| {
                app.windows
                    .values()
                    .any(|tiles| tiles.panes.iter().any(|(_, at)| *at == index))
            };

            if app.buffers.len() <= 1 {
                return refuse(app, String::from("that is the only buffer"));
            }
            if showing(app, buffer) {
                return refuse(app, String::from("a tile is showing that one"));
            }
            if app.buffers.get(buffer).is_none() {
                return Task::none();
            }

            // `swap_remove`, so every other index but one stays put. The
            // buffer that was last now sits in the hole, and any tile that
            // pointed at the last index has to be told.
            let moved = app.buffers.len() - 1;
            app.buffers.swap_remove(buffer);

            if moved != buffer {
                for tiles in app.windows.values_mut() {
                    for (_, at) in tiles.panes.iter_mut() {
                        if *at == moved {
                            *at = buffer;
                        }
                    }
                }

                // The moved buffer's listing task, if one is still running,
                // is addressed to the index it used to have and its chunks
                // would be dropped. Relisting is the honest fix: closing a
                // buffer is a deliberate, occasional act, so one extra
                // directory read costs nothing anybody will feel.
                return relist(app, buffer);
            }

            Task::none()
        }

        Action::Split(axis) => split(app, axis, None),

        Action::OpenBeside { path } => split(app, pane_grid::Axis::Vertical, Some(path)),

        Action::CloseTile => {
            let Some(window) = app.windows.values_mut().next() else {
                return Task::none();
            };

            // The last tile stays. A window with none in it shows nothing and
            // gives nobody a way back.
            if window.panes.len() <= 1 {
                return refuse(app, String::from("that is the only tile"));
            }

            if let Some((_, sibling)) = window.panes.close(window.focus) {
                window.focus = sibling;
            }

            // The buffer stays open. That is the point of buffers: closing a
            // tile costs nothing and reopening the directory is instant.
            Task::none()
        }

        Action::NextTile | Action::PreviousTile => {
            let backwards = matches!(action, Action::PreviousTile);
            let Some(window) = app.windows.values_mut().next() else {
                return Task::none();
            };

            let order: Vec<_> = window.panes.iter().map(|(pane, _)| *pane).collect();
            let Some(at) = order.iter().position(|pane| *pane == window.focus) else {
                return Task::none();
            };

            let next = if backwards {
                (at + order.len() - 1) % order.len()
            } else {
                (at + 1) % order.len()
            };
            window.focus = order[next];
            Task::none()
        }

        Action::Relist { buffer } => relist(app, buffer),

        Action::Escape => {
            // One key for "put away whatever is in front of me", in the order
            // things are stacked.
            if app.menu.take().is_some() {
                return Task::none();
            }
            if app.dialogue.is_some() {
                return Task::done(Message::Dialogue(Choice::Dismiss));
            }
            if app.typing_path.is_some() {
                app.typing_path = None;
                return Task::none();
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

        Action::Invert { buffer } => {
            with(app, buffer, Buffer::invert);
            Task::none()
        }

        Action::Band {
            buffer,
            rows,
            columns,
            across,
            add,
        } => {
            with(app, buffer, |found| {
                found.select_band(&rows, columns.as_ref(), across, add);
            });
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

            // Arriving somewhere clears whatever the notice line was
            // complaining about. "`/s` is not a directory" left up after a
            // successful move reads as though this directory were the
            // problem.
            app.notice = None;
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

/// Rebuild every buffer after a setting that changes the order.
///
/// Every one, not only the focused tile: the sort is a window-wide setting,
/// and a second tile left in the old order would look like a bug.
fn resort(app: &mut App) {
    let list = app.config.list.clone();
    for buffer in &mut app.buffers {
        buffer.rebuild(&list);
    }
}

/// Do something to one buffer, if it is still there.
/// Split the focused tile, and give the new half a buffer of its own.
///
/// `where_to` is the directory for the new tile; `None` means the same one the
/// old tile shows. Either way it is a *separate* buffer, so the two are
/// independent. Pointing a tile at a buffer another one already shows is the
/// other move, and that is what shares a listing -- see `Tiles`.
fn split(app: &mut App, axis: pane_grid::Axis, where_to: Option<PathBuf>) -> Task<Message> {
    let Some(window) = app.windows.values().next() else {
        return Task::none();
    };
    let Some(showing) = window.panes.get(window.focus).copied() else {
        return Task::none();
    };
    let Some((path, view)) = app
        .buffers
        .get(showing)
        .map(|found| (found.path.clone(), found.view))
    else {
        return Task::none();
    };

    let fresh = app.buffers.len();
    // The new tile copies the view of the one it split from, which is what
    // somebody splitting a grid to compare two directories expects.
    app.buffers
        .push(Buffer::new(where_to.unwrap_or(path), view));

    let Some(window) = app.windows.values_mut().next() else {
        return Task::none();
    };
    let Some((pane, _)) = window.panes.split(axis, window.focus, fresh) else {
        // `pane_grid` refuses a split it cannot fit. Saying so beats a
        // keystroke that does nothing.
        app.buffers.pop();
        return refuse(app, String::from("no room to split"));
    };
    window.focus = pane;

    relist(app, fresh)
}

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
    Task::perform(async move { open::spawn(&plan) }, Message::Spawned)
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
    let Some(tiles) = app.windows.get(&window) else {
        return text("no window").into();
    };

    let focused = tiles.buffer();

    // One tile per pane, each showing whichever buffer it points at. The
    // divider between two panes is draggable, which `pane_grid` gives free.
    let grid = pane_grid(&tiles.panes, |pane, index, _maximised| {
        let Some(buffer) = app.buffers.get(*index) else {
            return pane_grid::Content::new(text("no buffer"));
        };

        pane_grid::Content::new(tile(app, *index, buffer, pane == tiles.focus))
    })
    .width(Length::Fill)
    .height(Length::Fill)
    // Wide enough to see and to grab. `pane_grid` leaves the gap empty and
    // shows whatever is behind, so this only reads as a divider because the
    // container below paints `muted` there -- at 1px against a background the
    // same colour as the tiles, two tiles looked like one.
    .spacing(4)
    .on_click(Message::TileClicked)
    .on_drag(Message::TileDragged)
    .on_resize(6, Message::TileResized)
    .style(move |_: &iced::Theme| pane_grid::Style {
        hovered_region: pane_grid::Highlight {
            background: app.config.theme.accent.color().into(),
            border: iced::Border::default(),
        },
        picked_split: pane_grid::Line {
            color: app.config.theme.accent.color(),
            width: 2.0,
        },
        hovered_split: pane_grid::Line {
            color: app.config.theme.accent.color(),
            width: 2.0,
        },
    });

    // Places above, jobs below, the tiles in the rest. Fixed furniture: a file
    // manager whose panels move around is one nobody can be shown how to use.
    let page = row![
        sidebar(app, focused),
        // One pixel of `muted`, full height. A `Space` with no height makes
        // the container collapse to nothing, which is a divider you cannot
        // see -- it was written that way once.
        container(iced::widget::Space::new())
            .width(1)
            .height(Length::Fill)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(app.config.theme.muted.color().into()),
                ..container::Style::default()
            }),
        // `muted` behind the grid, so the gaps `pane_grid` leaves between
        // tiles are lines rather than nothing.
        container(grid)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(app.config.theme.muted.color().into()),
                ..container::Style::default()
            }),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    // A menu sits over the page, and under a dialogue.
    let page: Element<'_, Message> = match &app.menu {
        Some(menu) => iced::widget::stack![page, context_menu(app, menu)].into(),
        None => page.into(),
    };

    let Some(dialogue) = &app.dialogue else {
        return page;
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

/// One tile: a path bar, the list, and the counts underneath.
fn tile<'a>(app: &'a App, index: usize, buffer: &'a Buffer, focused: bool) -> Element<'a, Message> {
    let list = FileList::new(
        buffer,
        &app.config.theme,
        &app.config.list,
        app.config.window.font_size,
        app.icon_font,
        // Only the focused tile takes the keyboard, and not while a dialogue
        // is up: otherwise Escape would close the dialogue and clear the
        // filter in one keystroke, and arrows would move an unseen cursor.
        //
        // Nor while the path bar is showing its text face. That is different
        // from the filter box, which deliberately leaves the list live --
        // `text_input` captures neither the vertical arrows nor Tab, so
        // filtering and navigating at once is free. A path being typed wants
        // those keys: Tab is completion, and with the list live it switched
        // tiles instead. Escape then has to come from the subscription, since
        // the list is no longer there to report it.
        focused && app.dialogue.is_none() && app.typing_path.is_none(),
        move |action| Message::List(index, action),
    );

    let mut page = column![path_bar(app, index, buffer)];

    if app.filtering && focused {
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

    page = page
        .push(container(list).width(Length::Fill).height(Length::Fill))
        .push(status(app, buffer));

    // The focused tile is edged in the accent. With two tiles and no mark,
    // nothing on screen says which one the keyboard will reach.
    let accent = app.config.theme.accent.color();
    let muted = app.config.theme.muted.color();

    let background = app.config.theme.background.color();

    container(page.width(Length::Fill).height(Length::Fill))
        .style(move |_: &iced::Theme| container::Style {
            // Its own background, now that `muted` sits behind the grid to
            // make the gaps visible. Without this the divider colour shows
            // through every tile.
            background: Some(background.into()),
            border: iced::Border {
                color: if focused { accent } else { muted },
                width: if focused { 1.0 } else { 0.0 },
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
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
    // here, and a long session opens more directories than that. A buffer in
    // two tiles is watched once, which is what the set is for.
    // A `BTreeSet`, not a `HashSet`. Both remove the duplicate when two tiles
    // show one buffer, but a `HashSet` iterates in an order that changes from
    // one call to the next, so the batch of subscriptions handed to iced would
    // be shuffled after every update. Sorted, it is the same list every time.
    let watching: std::collections::BTreeSet<usize> = app
        .windows
        .values()
        .flat_map(|tiles| tiles.panes.iter().map(|(_, index)| *index))
        .collect();

    let watches = watching.into_iter().filter_map(|index| {
        let buffer = app.buffers.get(index)?;
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
                .with(index)
                .map(|(index, change)| Message::Changed(index, change)),
        )
    });

    // A bare `fn`, not a closure: iced hashes the function to identify the
    // subscription and rejects one that captures anything.
    fn moved(
        event: iced::Event,
        _status: iced::event::Status,
        _window: window::Id,
    ) -> Option<Message> {
        match event {
            iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                Some(Message::Pointer(position.x, position.y))
            }
            // The path bar's text face takes the keyboard away from the list,
            // so nothing else is left to report Escape while it is open.
            // `update` ignores this unless the box is actually up.
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                ..
            }) => Some(Message::EscapedPath),
            // Tab, for the same reason. `text_input` does not capture it
            // either, so without this it would reach nothing at all.
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
                ..
            }) => Some(Message::CompletePath),
            _ => None,
        }
    }

    // A clock, but only while something is being read.
    //
    // The count in the status line moves when a chunk lands, which on a local
    // disk is often enough to look alive. On sshfs a chunk can take seconds,
    // and on a hung NFS mount `readdir` never returns at all -- so exactly
    // when it matters most, a count driven by arrivals stops moving and the
    // window looks frozen. A clock keeps the bar sliding whatever the
    // filesystem is doing, which is the one thing it has to say: ricedir is
    // fine, the mount is slow.
    //
    // It is subscribed only while a read is outstanding, so an idle window
    // wakes for nothing.
    let reading = app
        .buffers
        .iter()
        .any(|found| found.refreshing() || found.listing == Listing::Loading);

    let ticking = reading
        .then(|| iced::time::every(std::time::Duration::from_millis(80)).map(|_| Message::Tick));

    iced::Subscription::batch(
        watches.chain(
            [
                window::close_events().map(Message::Closed),
                window::resize_events().map(|(_, size)| Message::Resized(size)),
                iced::event::listen_with(moved),
            ]
            .into_iter()
            .chain(ticking),
        ),
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
        // Leftmost, then back and forward, then the path. A browser's order,
        // which is the one most hands already know.
        tool(
            app,
            '\u{f021}',
            String::from("Relist  (F5)"),
            false,
            Message::Act(Action::Relist { buffer: index }),
        ),
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

    // The path bar has two faces, and this is where it turns over. Text when
    // somebody asked for text -- a row of buttons gives nothing to drag
    // across, and taking `~/workspace/rust` out of a longer path to
    // paste into a terminal is a thing people do constantly.
    let focused = app
        .windows
        .values()
        .any(|window| window.panes.get(window.focus).copied() == Some(index));

    let middle: Element<'a, Message> = match &app.typing_path {
        Some(typed) if focused => iced::widget::text_input("path", typed)
            .id(path_id())
            .on_input(Message::PathTyped)
            .on_submit(Message::PathSubmitted(index))
            .size(14)
            .padding([2, 6])
            .width(Length::Fill)
            .into(),

        _ => {
            // Built from the components rather than by splitting the string,
            // so a directory with a slash-looking name in it cannot fool the
            // crumbs.
            let mut walked = PathBuf::new();
            for component in buffer.path.components() {
                walked.push(component.as_os_str());

                let label = match component {
                    std::path::Component::RootDir => String::from("/"),
                    other => other.as_os_str().to_string_lossy().into_owned(),
                };

                crumbs = crumbs.push(quiet(label, Some(Message::Go(index, walked.clone()))));
            }

            // The space after the last crumb turns the bar over. A click on a
            // crumb already means "go there" and has to keep meaning it, so
            // the two are told apart by where the click lands -- which makes
            // the gap the only unambiguous target, and on a short path it is
            // most of the bar. The `\u{f044}` button beside it is the target
            // that is never ambiguous.
            iced::widget::mouse_area(container(crumbs).width(Length::Fill))
                .on_press(Message::Act(Action::TypingPath {
                    buffer: index,
                    typing: true,
                }))
                .into()
        }
    };

    // The crumbs take what is left, so the toolbar stays pinned to the right
    // however long the path is.
    row![
        container(middle).width(Length::Fill),
        tool(
            app,
            '\u{f044}',
            String::from("Edit the path  (Ctrl+L)"),
            app.typing_path.is_some() && focused,
            Message::Act(Action::TypingPath {
                buffer: index,
                typing: app.typing_path.is_none(),
            }),
        ),
        toolbar(app, index, buffer.view)
    ]
    .align_y(iced::Alignment::Center)
    .padding([4, 4])
    .width(Length::Fill)
    .into()
}

/// The buttons at the right of a tile's path bar.
///
/// Per tile rather than one bar across the window. A single bar would have to
/// answer "which tile does this act on", and the honest answer is whichever
/// has the keyboard -- which is one more thing to know before pressing a
/// button. Beside the path it is already unambiguous.
fn toolbar(app: &App, index: usize, view: crate::config::View) -> Element<'_, Message> {
    let list = &app.config.list;
    let layout = view.layout;

    // Every codepoint checked against the installed font before use, the way
    // `icon.rs` was built. A guessed one draws an empty box.
    //
    // The bars glyph is not here: it belongs to the menu at the far right,
    // where a browser puts it, and two buttons wearing it would each look
    // like the other one's job.
    let layout_glyph = match layout {
        crate::config::Layout::List => '\u{f03a}',
        crate::config::Layout::Detail => '\u{f0ce}',
        crate::config::Layout::Icons => '\u{f009}',
    };

    let sort_glyph = if list.sort_reversed {
        '\u{f0de}'
    } else {
        '\u{f0dd}'
    };

    let hidden_glyph = if view.show_hidden {
        '\u{f06e}'
    } else {
        '\u{f070}'
    };

    row![
        tool(
            app,
            layout_glyph,
            format!("View: {} \u{2192} {}", layout.name(), layout.next().name()),
            false,
            Message::Act(Action::Layout {
                buffer: index,
                layout: layout.next(),
            }),
        ),
        tool(
            app,
            sort_glyph,
            format!("Sort: {:?}", list.sort),
            false,
            // Under the pointer that clicked it. A button cannot say where
            // it is, so the menu goes where the hand already was.
            Message::Act(Action::Menu {
                kind: MenuKind::Sort,
                buffer: index,
                at: app.pointer,
            }),
        ),
        tool(
            app,
            hidden_glyph,
            String::from("Hidden files"),
            view.show_hidden,
            Message::Act(Action::ShowHidden {
                buffer: index,
                showing: !view.show_hidden,
            }),
        ),
        tool(
            app,
            SPLIT_RIGHT,
            String::from("Split right  (Ctrl+\\)"),
            false,
            Message::Act(Action::Split(pane_grid::Axis::Vertical)),
        ),
        tool(
            app,
            SPLIT_DOWN,
            String::from("Split down  (Ctrl+-)"),
            false,
            Message::Act(Action::Split(pane_grid::Axis::Horizontal)),
        ),
        tool(
            app,
            '\u{f00d}',
            String::from("Close this tile  (Ctrl+W)"),
            false,
            Message::Act(Action::CloseTile),
        ),
        // Everything the buttons do, spelled out. A tile narrow enough to
        // clip the buttons still has this, and a person who cannot tell one
        // glyph from another can read the words.
        //
        // Far right, wearing the bars a browser wears. It is the one button
        // here that people already know where to look for.
        tool(
            app,
            '\u{f0c9}',
            String::from("Everything else"),
            false,
            Message::Act(Action::Menu {
                kind: MenuKind::Toolbar,
                buffer: index,
                at: app.pointer,
            }),
        ),
    ]
    .spacing(2)
    .align_y(iced::Alignment::Center)
    .into()
}

/// One toolbar button: a glyph, and a tooltip saying what it does.
fn tool(app: &App, glyph: char, says: String, lit: bool, message: Message) -> Element<'_, Message> {
    use iced::widget::{button, tooltip};

    let theme = &app.config.theme;
    let dim = theme.dim.color();
    let accent = theme.accent.color();
    let muted = theme.muted.color();

    let face = app.icon_font.unwrap_or_default();

    let pressed = button(text(glyph.to_string()).font(face).size(14))
        .padding([3, 7])
        .style(move |_: &iced::Theme, status| button::Style {
            // Lit means the thing is on -- hidden files showing -- which a
            // toolbar has to say without being asked.
            background: (lit || matches!(status, button::Status::Hovered))
                .then(|| if lit { accent.into() } else { muted.into() }),
            text_color: if lit { theme.background.color() } else { dim },
            border: iced::Border {
                radius: 4.0.into(),
                ..iced::Border::default()
            },
            ..button::Style::default()
        })
        .on_press(message);

    tooltip(
        pressed,
        container(text(says).size(12))
            .padding([2, 6])
            .style(move |_: &iced::Theme| container::Style {
                background: Some(theme.background.color().into()),
                border: iced::Border {
                    color: muted,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            }),
        tooltip::Position::Bottom,
    )
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

    let reading = buffer.refreshing() || buffer.listing == Listing::Loading;
    let selected = buffer.selection.len();
    let counts = match buffer.listing {
        Listing::Loading => format!("{} items, reading\u{2026}", buffer.rows()),
        Listing::Failed(_) => String::from("unreadable"),
        Listing::Ready if selected > 0 => {
            format!("{} items, {selected} selected", buffer.rows())
        }
        Listing::Ready => format!("{} items", buffer.rows()),
    };

    // A relist keeps the old listing up and stays `Ready`, so without this
    // there is nothing on screen to say a refresh is running. The count of
    // what is showing is still true, so it stays, and the second number is
    // how much of the replacement has arrived.
    //
    // The number and the bar say different things and both are wanted. The
    // number says how big the directory is turning out to be, but it only
    // moves when a chunk lands -- rarely on sshfs, never on a hung mount.
    // The bar runs off a clock and says the one thing left to say there:
    // ricedir is alive and the filesystem is slow.
    let counts = if buffer.refreshing() {
        format!(
            "{counts}  \u{b7}  refreshing, {} read\u{2026}",
            buffer.read_so_far()
        )
    } else {
        counts
    };

    // Saying a filter is on matters more than the count: a listing that is
    // narrowed and does not say so looks like a directory that lost files.
    let counts = if buffer.filter.is_empty() {
        counts
    } else {
        format!("{counts}  ·  filtered by `{}`", buffer.filter)
    };

    let mut line = row![text(counts).size(13)].spacing(12);

    if reading {
        line = line.push(reading_bar(app));
    }

    container(line.align_y(iced::Alignment::Center))
        .padding(8)
        .width(Length::Fill)
        .into()
}

/// A lit block sliding along a track, while a directory is being read.
///
/// Quads and nothing else -- no glyph, so no font to check, and no rotation,
/// which iced has no transform for anyway. What it has to say is only that
/// ricedir is alive: on a stalled sshfs or a hung NFS mount the count beside
/// it stops moving, and a still window is indistinguishable from a crashed
/// one.
fn reading_bar(app: &App) -> Element<'_, Message> {
    /// How many steps the block takes to cross and come back.
    const STEPS: usize = 12;

    let theme = &app.config.theme;
    let accent = theme.accent.color();
    let muted = theme.muted.color();

    // A triangle wave, so it slides back rather than jumping to the start.
    let step = app.tick % (STEPS * 2);
    let at = if step < STEPS { step } else { STEPS * 2 - step };

    let block = |lit: bool| {
        container(iced::widget::Space::new().width(6).height(4)).style(move |_: &iced::Theme| {
            container::Style {
                background: Some(if lit { accent.into() } else { muted.into() }),
                border: iced::Border {
                    radius: 2.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            }
        })
    };

    let mut track = row![].spacing(3);
    for square in 0..STEPS {
        track = track.push(block(square == at));
    }
    track.into()
}

/// The menu a right click opens, where the right click happened.
///
/// Placed with `pin`, because the list widget already knew where the pointer
/// was: iced tells `update` no geometry, but a `Widget` is handed the cursor,
/// so the position travels with the action instead of being tracked apart.
fn context_menu<'a>(app: &'a App, menu: &'a Menu) -> Element<'a, Message> {
    use iced::widget::{button, mouse_area, pin};

    let theme = &app.config.theme;
    let foreground = theme.foreground.color();
    let background = theme.background.color();
    let accent = theme.accent.color();

    let buffer = app.buffers.get(menu.buffer);
    let row = match menu.kind {
        MenuKind::Context { row } => row,
        MenuKind::Place { .. } | MenuKind::Buffers | MenuKind::Sort | MenuKind::Toolbar => None,
    };
    let entry = row.and_then(|row| buffer.and_then(|found| found.at(row)));
    let directory = entry.filter(|entry| entry.kind.is_directory());

    let item = move |label: String, message: Message| {
        button(text(label).size(13))
            .width(Length::Fill)
            .padding([4, 12])
            .style(move |_: &iced::Theme, status| button::Style {
                background: matches!(status, button::Status::Hovered).then(|| accent.into()),
                text_color: if matches!(status, button::Status::Hovered) {
                    background
                } else {
                    foreground
                },
                ..button::Style::default()
            })
            .on_press(message)
    };

    let mut items = column![].width(Length::Fill);

    // The sort menu is a short list of fields. The view-wide switches -- the
    // layout, hidden files, relist, split, close -- moved to the toolbar,
    // where a button can also *show* whether a thing is on. A context menu
    // that carried them made every right click a list of settings rather than
    // a list of things to do to the file under the pointer.
    if menu.kind == MenuKind::Toolbar {
        let list = &app.config.list;

        // The buffer the menu was opened over, not the config: the view is a
        // property of one tile, and the menu has to offer the same next step
        // the button beside it does.
        let view = buffer.map_or_else(|| list.view(), |found| found.view);

        items = items.push(item(
            format!("View: {}", view.layout.next().name()),
            Message::Act(Action::Layout {
                buffer: menu.buffer,
                layout: view.layout.next(),
            }),
        ));
        items = items.push(item(
            String::from(if view.show_hidden {
                "Hide hidden files"
            } else {
                "Show hidden files"
            }),
            Message::Act(Action::ShowHidden {
                buffer: menu.buffer,
                showing: !view.show_hidden,
            }),
        ));
        items = items.push(item(
            String::from("Relist"),
            Message::Act(Action::Relist {
                buffer: menu.buffer,
            }),
        ));
        items = items.push(item(
            String::from("Split right"),
            Message::Act(Action::Split(pane_grid::Axis::Vertical)),
        ));
        items = items.push(item(
            String::from("Split down"),
            Message::Act(Action::Split(pane_grid::Axis::Horizontal)),
        ));
        items = items.push(item(
            String::from("Close this tile"),
            Message::Act(Action::CloseTile),
        ));
        items = items.push(item(
            String::from("Filter\u{2026}"),
            Message::Act(Action::Filtering(true)),
        ));
        items = items.push(item(
            String::from("Open directories\u{2026}  (Ctrl+B)"),
            Message::Act(Action::Menu {
                kind: MenuKind::Buffers,
                buffer: menu.buffer,
                at: app.pointer,
            }),
        ));
    } else if menu.kind == MenuKind::Sort {
        use crate::config::Sort;

        for (field, label) in [
            (Sort::Name, "Name"),
            (Sort::Size, "Size"),
            (Sort::Modified, "Modified"),
            (Sort::Extension, "Type"),
        ] {
            let chosen = app.config.list.sort == field;
            let arrow = if !chosen {
                ""
            } else if app.config.list.sort_reversed {
                " \u{2191}"
            } else {
                " \u{2193}"
            };

            items = items.push(item(
                format!("{label}{arrow}"),
                Message::Act(Action::SortBy(field)),
            ));
        }

        items = items.push(item(
            String::from("Reverse the order"),
            Message::Act(Action::ReverseSort),
        ));
    } else if menu.kind == MenuKind::Buffers {
        // Every open directory, and how many tiles already show it. The count
        // is the thing worth saying: picking one that is already beside you
        // is how two tiles come to share a listing, and picking one nothing
        // shows is how a buffer comes back from being closed.
        for (at, found) in app.buffers.iter().enumerate() {
            let tiles = app
                .windows
                .values()
                .flat_map(|window| window.panes.iter())
                .filter(|(_, index)| **index == at)
                .count();

            let mark = if at == menu.buffer { " \u{2022}" } else { "" };
            let seen = match tiles {
                0 => String::from("  (no tile)"),
                1 => String::new(),
                many => format!("  ({many} tiles)"),
            };

            items = items.push(item(
                format!("{at}  {}{mark}{seen}", short(&found.path)),
                Message::Act(Action::ShowBuffer { buffer: at }),
            ));
        }

        // Closing is offered for this tile's own buffer only when something
        // else is showing it too, which is never -- so it is offered for the
        // ones nothing shows, where it is the whole point.
        let idle: Vec<usize> = app
            .buffers
            .iter()
            .enumerate()
            .map(|(at, _)| at)
            .filter(|at| {
                !app.windows
                    .values()
                    .any(|window| window.panes.iter().any(|(_, index)| index == at))
            })
            .collect();

        for at in idle {
            let Some(found) = app.buffers.get(at) else {
                continue;
            };
            items = items.push(item(
                format!("Close {}", short(&found.path)),
                Message::Act(Action::CloseBuffer { buffer: at }),
            ));
        }
    } else if let MenuKind::Place { index } = menu.kind {
        // On a place in the sidebar. Three items, and the third only for a
        // bookmark: the home directory and a mounted disk are not ours to
        // take out of the list.
        let Some(place) = app.places.get(index) else {
            return iced::widget::Space::new().into();
        };

        items = items.push(item(
            format!("Open {}", place.label),
            Message::Act(Action::Go {
                buffer: menu.buffer,
                path: place.path.clone(),
            }),
        ));
        items = items.push(item(
            String::from("Open in a new tile"),
            Message::Act(Action::OpenBeside {
                path: place.path.clone(),
            }),
        ));

        if place.kind == places::Kind::Bookmark {
            items = items.push(item(
                String::from("Remove from places"),
                Message::Act(Action::Unbookmark {
                    path: place.path.clone(),
                }),
            ));
        }
    } else if let Some(row) = row {
        // On an entry. What can be done to the file under the pointer, and
        // nothing about the window: a right click that opened a list of
        // settings was what pushed those onto the toolbar.
        if entry.is_some() {
            items = items.push(item(
                String::from("Open"),
                Message::Act(Action::Activate {
                    buffer: menu.buffer,
                    row,
                }),
            ));
        }

        if let Some(entry) = directory {
            items = items.push(item(
                String::from("Open in a new tile"),
                Message::Act(Action::OpenBeside {
                    path: entry.path.clone(),
                }),
            ));
        }

        items = items.push(item(
            String::from("Copy path"),
            Message::Act(Action::CopyPath {
                buffer: menu.buffer,
            }),
        ));

        if let Some(entry) = directory {
            items = items.push(item(
                format!("Add {} to places", entry.name),
                Message::Act(Action::Bookmark {
                    buffer: menu.buffer,
                    path: Some(entry.path.clone()),
                }),
            ));
        }
    } else {
        // On empty space. About the directory rather than about a file --
        // which is why the click had to be told apart from one on a row, and
        // why "paste" and "new folder" will land here in M2 rather than in
        // the menu above.
        let Some(found) = buffer else {
            return iced::widget::Space::new().into();
        };

        items = items.push(item(
            String::from(if found.view.show_hidden {
                "Hide hidden files"
            } else {
                "Show hidden files"
            }),
            Message::Act(Action::ShowHidden {
                buffer: menu.buffer,
                showing: !found.view.show_hidden,
            }),
        ));
        items = items.push(item(
            String::from("Relist"),
            Message::Act(Action::Relist {
                buffer: menu.buffer,
            }),
        ));
        items = items.push(item(
            String::from("Copy path"),
            Message::Act(Action::CopyPath {
                buffer: menu.buffer,
            }),
        ));
        items = items.push(item(
            String::from("Add this directory to places  (Ctrl+D)"),
            Message::Act(Action::Bookmark {
                buffer: menu.buffer,
                path: None,
            }),
        ));
    }

    // Kept inside the window. A menu opened near the right edge would
    // otherwise be clipped, and its longest line would wrap instead of the
    // menu simply moving left -- which is what a sort menu on the toolbar did,
    // since the toolbar lives at the right-hand end of the bar.
    //
    // The buffer list is wider than the rest because its lines are paths.
    // `short` takes `$HOME` off the front and the rest is as long as it is;
    // at the other menus' width every second line wrapped.
    let wide: f32 = if menu.kind == MenuKind::Buffers {
        360.0
    } else {
        230.0
    };
    const TALL: f32 = 200.0;

    // A menu opened by a key has no click to sit under, and the pointer may
    // never have moved -- `app.pointer` is then still the window's corner,
    // which is where the buffer list first appeared, half of it off the
    // screen. So it is placed rather than followed: below the toolbar and
    // clear of the sidebar, which is inside the tile it acts on.
    let asked_at = if menu.kind == MenuKind::Buffers {
        (SIDEBAR + 12.0, 40.0)
    } else {
        menu.at
    };

    let at = (
        asked_at.0.min((app.size.width - wide).max(0.0)),
        asked_at.1.min((app.size.height - TALL).max(0.0)),
    );

    let panel = container(items)
        .width(Length::Fixed(wide))
        .padding(4)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(background.into()),
            border: iced::Border {
                color: theme.muted.color(),
                width: 1.0,
                radius: 6.0.into(),
            },
            shadow: iced::Shadow {
                color: iced::Color {
                    a: 0.4,
                    ..iced::Color::BLACK
                },
                offset: iced::Vector::new(0.0, 2.0),
                blur_radius: 8.0,
            },
            ..container::Style::default()
        });

    // A click anywhere else puts the menu away, which is what every menu on
    // every desktop does. The catcher fills the window under the panel.
    mouse_area(
        container(pin(panel).x(at.0).y(at.1))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::Act(Action::Escape))
    .on_right_press(Message::Act(Action::Escape))
    .into()
}

/// The panel down the left: places at the top, jobs at the bottom.
fn sidebar<'a>(app: &'a App, index: usize) -> Element<'a, Message> {
    use iced::widget::{button, scrollable};

    let theme = &app.config.theme;
    let dim = theme.dim.color();
    let foreground = theme.foreground.color();

    let heading = |what: &'a str| {
        container(text(what).size(11).color(dim))
            .padding([8, 10])
            .width(Length::Fill)
    };

    let mut list = column![].width(Length::Fill);
    let mut previous = None;

    for (at, place) in app.places.iter().enumerate() {
        // A rule between the four groups, so the panel reads as a short list
        // of short lists rather than one long one.
        if previous.is_some_and(|kind| kind != place.kind) {
            list = list.push(iced::widget::Space::new().height(6));
        }
        previous = Some(place.kind);

        let entry = button(text(place.label.clone()).size(13))
            .width(Length::Fill)
            .padding([3, 10])
            .style(move |_: &iced::Theme, status| button::Style {
                background: None,
                text_color: if matches!(status, button::Status::Hovered) {
                    iced::Color::WHITE
                } else {
                    foreground
                },
                ..button::Style::default()
            })
            .on_press(Message::Act(Action::Go {
                buffer: index,
                path: place.path.clone(),
            }));

        // `button` has no right press, and `mouse_area`'s does not say where
        // it happened. The tracked pointer answers that: it is the same
        // position, one event earlier.
        list = list.push(iced::widget::mouse_area(entry).on_right_press(Message::Act(
            Action::Menu {
                kind: MenuKind::Place { index: at },
                buffer: index,
                at: app.pointer,
            },
        )));
    }

    let jobs = column![
        heading("JOBS"),
        container(text("nothing running").size(12).color(dim)).padding([0, 10]),
    ]
    .width(Length::Fill);

    container(
        column![
            heading("PLACES"),
            container(scrollable(list)).height(Length::Fill),
            jobs,
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fixed(SIDEBAR))
    .height(Length::Fill)
    .style(move |_: &iced::Theme| container::Style {
        background: Some(theme.background.color().into()),
        ..container::Style::default()
    })
    .into()
}

/// The filter box, so it can be focused when it appears.
///
/// A fixed id rather than one per buffer: there is one box, and it belongs to
/// whichever tile is in front.
const fn filter_id() -> iced::widget::Id {
    iced::widget::Id::new("ricedir-filter")
}

/// The path bar's text face, for the same reason.
const fn path_id() -> iced::widget::Id {
    iced::widget::Id::new("ricedir-path")
}

/// The longest path every directory matching what has been typed agrees on.
///
/// `None` when nothing matches, so a Tab that completes nothing changes
/// nothing rather than emptying the box. Directories only: the path bar goes
/// to a directory, and offering a file would complete to something that
/// cannot be submitted.
///
/// The common prefix rather than the first match, which is what a shell does
/// and what stops Tab guessing: two directories starting `wo` complete to
/// `wo` and wait for another letter.
fn complete(typed: &str) -> Option<String> {
    let full = expand(typed);

    // Trailing slash means "inside this", not "finish this name".
    let (directory, start) = if typed.ends_with('/') {
        (full.as_path(), String::new())
    } else {
        (
            full.parent()?,
            full.file_name()?.to_string_lossy().into_owned(),
        )
    };

    let mut matches = Vec::new();
    for entry in std::fs::read_dir(directory).ok()? {
        let Ok(entry) = entry else { continue };
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&start) {
            matches.push(name);
        }
    }

    let (first, rest) = matches.split_first()?;
    let mut shared = first.clone();
    for name in rest {
        let keep = shared
            .char_indices()
            .zip(name.chars())
            .take_while(|((_, want), got)| want == got)
            .count();
        // By character, then cut on a boundary the indices give us.
        shared = shared.chars().take(keep).collect();
    }

    if shared.len() <= start.len() {
        return None;
    }

    let mut done = directory.join(shared).to_string_lossy().into_owned();
    // A completed directory gets its slash, so the next Tab looks inside it.
    if matches.len() == 1 {
        done.push('/');
    }
    Some(done)
}

/// What a typed path means: `~` is home, and a relative one is relative to
/// nothing in particular, so it is left as typed and will simply not exist.
///
/// Deliberately *not* a shell: no globbing, no `$VAR`, no command
/// substitution. A path bar that ran a shell would be a shell prompt with a
/// file manager attached to it.
fn expand(text: &str) -> PathBuf {
    under(text, std::env::var_os("HOME").map(PathBuf::from).as_deref())
}

/// [`expand`], with home passed in.
///
/// Apart so it can be tested: `HOME` is process-global and the test harness
/// runs in threads, so a test that set it would fight every other test that
/// reads it. `places::user_dirs` is split for the same reason.
fn under(text: &str, home: Option<&Path>) -> PathBuf {
    let text = text.trim();

    let Some(home) = home else {
        return PathBuf::from(text);
    };

    match text {
        "~" => home.to_path_buf(),
        rest => match rest.strip_prefix("~/") {
            Some(inside) => home.join(inside),
            None => PathBuf::from(rest),
        },
    }
}

fn buffer_of(app: &App, window: window::Id) -> Option<&Buffer> {
    app.windows
        .get(&window)
        .and_then(|tiles| app.buffers.get(tiles.buffer()))
}

/// Start reading a buffer's directory again, abandoning whatever was running.
fn relist(app: &mut App, index: usize) -> Task<Message> {
    let Some(buffer) = app.buffers.get_mut(index) else {
        return Task::none();
    };

    buffer.generation = buffer.generation.wrapping_add(1);
    let generation = buffer.generation;

    // The old listing stays on screen while the new one is read, and is
    // swapped for it when the last chunk lands. Clearing first blanked the
    // tile for as long as the read took -- unnoticeable on a small directory
    // and about a second on 100,000, which reads as the program losing the
    // files rather than as it working.
    //
    // A buffer with nothing showing yet has nothing to preserve, so it
    // streams straight in and paints as it goes. That is what makes a slow
    // mount bearable, and it is worth keeping.
    buffer.start_arriving();

    let path = buffer.path.clone();
    buffer::list(path).map(move |update| Message::Listed(index, generation, update))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Layout;
    use crate::entry::{Entry, Kind};

    /// Drive `update` and drop the task: what these are about is the state it
    /// leaves behind, not the work it asks for.
    fn tell(app: &mut App, message: Message) {
        let _ = update(app, message);
    }

    fn app() -> App {
        App {
            config: Config::default(),
            buffers: vec![Buffer::new(
                PathBuf::from("/tmp/one"),
                Config::default().list.view(),
            )],
            windows: HashMap::new(),
            notice: None,
            dialogue: None,
            typed: String::new(),
            filtering: false,
            typing_path: None,
            icon_font: None,
            menu: None,
            pointer: (0.0, 0.0),
            size: iced::Size::new(1100.0, 700.0),
            places: Vec::new(),
            tick: 0,
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
            // Derived, not hardcoded: a fixture that says `.hidden` is not
            // hidden makes a test about hidden files pass for the wrong
            // reason, or fail for one.
            hidden: name.starts_with('.'),
        }
    }

    /// A view belongs to one buffer. Two tiles side by side wanting different
    /// arrangements is the ordinary case -- a wide detail listing beside a
    /// grid of pictures -- and a switch that changed both at once made the
    /// second tile useless for the thing it was opened for.
    #[test]
    fn a_view_switch_reaches_only_its_own_buffer() {
        let mut app = app();
        let start = Config::default().list.view();
        app.buffers
            .push(Buffer::new(PathBuf::from("/tmp/two"), start));

        tell(
            &mut app,
            Message::Act(Action::Layout {
                buffer: 1,
                layout: Layout::Icons,
            }),
        );

        assert_eq!(
            app.buffers[0].view.layout,
            Layout::List,
            "the other tile moved"
        );
        assert_eq!(app.buffers[1].view.layout, Layout::Icons);

        // And the config it started from is untouched, so a new buffer still
        // opens the way the config says rather than the way the last click
        // left one.
        assert_eq!(app.config.list.layout, Layout::default());
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

    /// A right click puts the cursor on the row it landed on, so what the
    /// menu will act on is on screen before it opens.
    #[test]
    fn a_menu_selects_what_it_will_act_on() {
        let mut app = app();
        let list = Config::default().list;
        app.buffers[0].extend(
            vec![entry("a.txt", Kind::File), entry("b.txt", Kind::File)],
            &list,
        );
        app.buffers[0].finish(&list);

        tell(
            &mut app,
            Message::List(
                0,
                list::Action::Menu {
                    row: Some(1),
                    at: iced::Point::new(40.0, 90.0),
                },
            ),
        );

        assert!(app.menu.is_some(), "the menu should be open");
        let selected: Vec<&str> = app.buffers[0]
            .selected()
            .map(|found| found.name.as_str())
            .collect();
        assert_eq!(selected, ["b.txt"]);
    }

    /// A right click past the last row is about the directory, so it must not
    /// pick a file on the way. Selecting the nearest row instead is how a
    /// menu ends up acting on something the person never pointed at.
    #[test]
    fn a_menu_on_empty_space_selects_nothing() {
        let mut app = app();
        let list = Config::default().list;
        app.buffers[0].extend(
            vec![entry("a.txt", Kind::File), entry("b.txt", Kind::File)],
            &list,
        );
        app.buffers[0].finish(&list);

        tell(
            &mut app,
            Message::List(
                0,
                list::Action::Menu {
                    row: None,
                    at: iced::Point::new(40.0, 600.0),
                },
            ),
        );

        assert!(app.menu.is_some(), "the menu should still open");
        assert_eq!(
            app.menu.as_ref().map(|menu| menu.kind),
            Some(MenuKind::Context { row: None }),
            "and know it was empty space"
        );
        assert_eq!(app.buffers[0].selected().count(), 0);
    }

    /// A right click inside an existing selection must not throw it away.
    /// Somebody who picked five files and right-clicked one of them means all
    /// five, which is what every file manager does.
    #[test]
    fn a_menu_inside_a_selection_keeps_it() {
        let mut app = app();
        let list = Config::default().list;
        app.buffers[0].extend(
            vec![
                entry("a.txt", Kind::File),
                entry("b.txt", Kind::File),
                entry("c.txt", Kind::File),
            ],
            &list,
        );
        app.buffers[0].finish(&list);
        app.buffers[0].select_all();

        tell(
            &mut app,
            Message::Act(Action::Menu {
                kind: MenuKind::Context { row: Some(1) },
                buffer: 0,
                at: (0.0, 0.0),
            }),
        );

        assert_eq!(app.buffers[0].selected().count(), 3, "all three still");
    }

    /// A menu that outlives the thing it was about acts on the wrong file, so
    /// every other action closes it.
    #[test]
    fn any_other_action_closes_the_menu() {
        let mut app = app();
        app.menu = Some(Menu {
            kind: MenuKind::Context { row: Some(0) },
            buffer: 0,
            at: (0.0, 0.0),
        });

        tell(&mut app, Message::Act(Action::SelectAll { buffer: 0 }));
        assert!(app.menu.is_none());
    }

    /// Escape closes the menu before it closes anything else, because the
    /// menu is the thing in front.
    #[test]
    fn escape_closes_the_menu_first() {
        let mut app = app();
        app.filtering = true;
        app.menu = Some(Menu {
            kind: MenuKind::Context { row: Some(0) },
            buffer: 0,
            at: (0.0, 0.0),
        });

        tell(&mut app, Message::Act(Action::Escape));
        assert!(app.menu.is_none(), "the menu went");
        assert!(app.filtering, "and the filter box stayed");
    }

    /// Hiding hidden files must rebuild every buffer, not only the one in
    /// front, or a second tile keeps showing them.
    #[test]
    fn showing_hidden_files_reaches_only_its_own_buffer() {
        let mut app = app();
        let list = Config::default().list;
        app.buffers
            .push(Buffer::new(PathBuf::from("/tmp/two"), list.view()));

        for buffer in &mut app.buffers {
            buffer.extend(
                vec![entry("plain", Kind::File), entry(".hidden", Kind::File)],
                &list,
            );
            buffer.finish(&list);
        }
        assert_eq!(app.buffers[1].rows(), 1);

        tell(
            &mut app,
            Message::Act(Action::ShowHidden {
                buffer: 1,
                showing: true,
            }),
        );

        assert_eq!(app.buffers[1].rows(), 2);
        assert_eq!(app.buffers[0].rows(), 1, "the other tile filled up");
    }

    /// The path bar takes a path, not a shell line. `~` is the one expansion,
    /// because it is the one people type; a `$VAR` or a `*` left alone will
    /// simply not be a directory and be refused.
    #[test]
    fn a_typed_path_expands_only_a_tilde() {
        let home = Path::new("/home/somebody");
        let at = |text| under(text, Some(home));

        assert_eq!(at("~"), PathBuf::from("/home/somebody"));
        assert_eq!(at("~/src"), PathBuf::from("/home/somebody/src"));
        assert_eq!(at("  /tmp/x  "), PathBuf::from("/tmp/x"));
        assert_eq!(at("$HOME"), PathBuf::from("$HOME"), "not a shell");
        assert_eq!(at("/tmp/*"), PathBuf::from("/tmp/*"), "nor a glob");
        assert_eq!(at("~notme"), PathBuf::from("~notme"), "not a user");

        // No home at all is survivable: a `~` is then just a silly directory
        // name, which is exactly what it is on disk.
        assert_eq!(under("~/src", None), PathBuf::from("~/src"));
    }

    /// Tab completes to what every match agrees on, the way a shell does, so
    /// it never guesses between two directories.
    #[test]
    fn tab_completes_to_the_shared_prefix() {
        let root = std::env::temp_dir().join("ricedir-complete");
        let _ = std::fs::remove_dir_all(&root);
        for name in ["workspace", "workbench", "other"] {
            std::fs::create_dir_all(root.join(name)).expect("make the tree");
        }
        std::fs::write(root.join("workfile"), b"x").expect("and a file");

        let typed = format!("{}/wo", root.display());
        let done = complete(&typed).expect("two directories start `work`");
        assert_eq!(done, format!("{}/work", root.display()), "no slash yet");

        // One match completes fully and gets a slash, so the next Tab looks
        // inside it. The file starting `work` is not offered.
        let typed = format!("{}/works", root.display());
        let done = complete(&typed).expect("only workspace");
        assert_eq!(done, format!("{}/workspace/", root.display()));

        // Nothing matching changes nothing rather than emptying the box.
        let typed = format!("{}/zzz", root.display());
        assert!(complete(&typed).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A path that is not a directory says so and keeps what was typed. Going
    /// nowhere and clearing the box would look like the keystroke was lost.
    #[test]
    fn submitting_a_path_that_is_not_there_says_so() {
        let mut app = app();
        app.typing_path = Some(String::from("/tmp/definitely-not-a-directory-here"));

        tell(&mut app, Message::PathSubmitted(0));

        assert_eq!(
            app.typing_path.as_deref(),
            Some("/tmp/definitely-not-a-directory-here"),
            "what was typed should still be there"
        );
        assert!(
            app.notice
                .as_deref()
                .is_some_and(|say| say.contains("not a directory")),
            "and the notice should say why: {:?}",
            app.notice
        );
    }

    /// Escape leaves the text face. It arrives from the subscription rather
    /// than the list, because the list is not taking keys while the box is up.
    #[test]
    fn escape_leaves_the_path_text_face() {
        let mut app = app();
        app.typing_path = Some(String::from("/tmp/half-typed"));

        tell(&mut app, Message::EscapedPath);
        assert!(app.typing_path.is_none());
    }

    /// Pointing a tile at a buffer that is already open is the other half of
    /// the emacs model: this is how two tiles come to share one listing.
    #[test]
    fn showing_a_buffer_points_the_focused_tile_at_it() {
        let mut app = app();
        app.windows.insert(window::Id::unique(), Tiles::new(0));
        let start = Config::default().list.view();
        app.buffers
            .push(Buffer::new(PathBuf::from("/tmp/two"), start));

        tell(&mut app, Message::Act(Action::ShowBuffer { buffer: 1 }));

        let tiles = app.windows.values().next().expect("one window");
        assert_eq!(tiles.buffer(), 1);
        assert_eq!(app.buffers.len(), 2, "no buffer was made or lost");
    }

    /// A buffer a tile is showing must not be closed underneath it, and the
    /// last one must not go at all -- either leaves a tile pointing at an
    /// index that is not there.
    #[test]
    fn a_buffer_in_use_is_not_closed() {
        let mut app = app();
        app.windows.insert(window::Id::unique(), Tiles::new(0));
        let start = Config::default().list.view();
        app.buffers
            .push(Buffer::new(PathBuf::from("/tmp/two"), start));

        tell(&mut app, Message::Act(Action::CloseBuffer { buffer: 0 }));
        assert_eq!(app.buffers.len(), 2, "a tile is showing that one");

        app.buffers.truncate(1);
        tell(&mut app, Message::Act(Action::CloseBuffer { buffer: 0 }));
        assert_eq!(app.buffers.len(), 1, "and it is the only one");
    }

    /// Closing uses `swap_remove`, so the buffer that was last lands in the
    /// hole. Any tile pointing at the old last index has to be told, or it
    /// shows a directory nobody asked for -- or nothing at all.
    #[test]
    fn closing_a_buffer_moves_the_last_one_and_tells_the_tiles() {
        let mut app = app();
        app.windows.insert(window::Id::unique(), Tiles::new(0));
        let start = Config::default().list.view();
        app.buffers
            .push(Buffer::new(PathBuf::from("/tmp/spare"), start));
        app.buffers
            .push(Buffer::new(PathBuf::from("/tmp/last"), start));

        // The one tile shows the last buffer; buffer 1 is showing nowhere.
        if let Some(window) = app.windows.values_mut().next() {
            let focus = window.focus;
            if let Some(at) = window.panes.get_mut(focus) {
                *at = 2;
            }
        }

        tell(&mut app, Message::Act(Action::CloseBuffer { buffer: 1 }));

        assert_eq!(app.buffers.len(), 2);
        assert_eq!(
            app.buffers[1].path,
            PathBuf::from("/tmp/last"),
            "the last buffer moved into the hole"
        );

        let tiles = app.windows.values().next().expect("one window");
        assert_eq!(tiles.buffer(), 1, "and the tile followed it");
    }

    /// "Open in a new tile" splits and lands on the directory asked for, not
    /// on the one the old tile was showing.
    #[test]
    fn opening_beside_puts_the_new_tile_on_the_named_directory() {
        let mut app = app();
        app.windows.insert(window::Id::unique(), Tiles::new(0));

        tell(
            &mut app,
            Message::Act(Action::OpenBeside {
                path: PathBuf::from("/tmp/elsewhere"),
            }),
        );

        assert_eq!(app.buffers.len(), 2);
        assert_eq!(app.buffers[0].path, PathBuf::from("/tmp/one"), "unmoved");
        assert_eq!(app.buffers[1].path, PathBuf::from("/tmp/elsewhere"));

        let tiles = app.windows.values().next().expect("one window");
        assert_eq!(tiles.panes.len(), 2);
        assert_eq!(tiles.buffer(), 1, "the new tile has the keyboard");
    }

    /// Splitting gives the new tile a buffer of its own, so the two are
    /// independent. Sharing is what pointing a tile at an existing buffer
    /// does, and that is a different move.
    #[test]
    fn a_split_makes_a_second_buffer_on_the_same_directory() {
        let mut app = app();
        app.windows.insert(window::Id::unique(), Tiles::new(0));
        assert_eq!(app.buffers.len(), 1);

        // Set both halves of the view away from the config's, so a split that
        // reached for the config rather than the parent would show.
        app.buffers[0].view.layout = Layout::Icons;
        app.buffers[0].view.show_hidden = true;

        tell(
            &mut app,
            Message::Act(Action::Split(pane_grid::Axis::Vertical)),
        );

        assert_eq!(app.buffers.len(), 2, "a buffer of its own");
        assert_eq!(app.buffers[1].path, app.buffers[0].path, "same directory");
        assert_eq!(
            app.buffers[1].view, app.buffers[0].view,
            "the new tile should look like the one it split from"
        );

        let tiles = app.windows.values().next().expect("one window");
        assert_eq!(tiles.panes.len(), 2);
        assert_eq!(tiles.buffer(), 1, "the new tile has the keyboard");
    }

    /// Closing a tile keeps the buffer. That is what buffers are for: closing
    /// costs nothing and reopening the directory is instant.
    #[test]
    fn closing_a_tile_keeps_its_buffer() {
        let mut app = app();
        app.windows.insert(window::Id::unique(), Tiles::new(0));
        tell(
            &mut app,
            Message::Act(Action::Split(pane_grid::Axis::Vertical)),
        );
        assert_eq!(app.buffers.len(), 2);

        tell(&mut app, Message::Act(Action::CloseTile));

        let tiles = app.windows.values().next().expect("one window");
        assert_eq!(tiles.panes.len(), 1, "one tile left");
        assert_eq!(app.buffers.len(), 2, "both buffers still open");
    }

    /// The last tile stays. A window with none in it shows nothing and gives
    /// nobody a way back.
    #[test]
    fn the_last_tile_cannot_be_closed() {
        let mut app = app();
        app.windows.insert(window::Id::unique(), Tiles::new(0));

        tell(&mut app, Message::Act(Action::CloseTile));

        let tiles = app.windows.values().next().expect("one window");
        assert_eq!(tiles.panes.len(), 1);
        assert!(app.notice.is_some(), "and it says why");
    }

    /// Tab goes round the tiles and comes back, rather than stopping at the
    /// end and leaving somebody stuck.
    #[test]
    fn tab_wraps_round_the_tiles() {
        let mut app = app();
        app.windows.insert(window::Id::unique(), Tiles::new(0));
        tell(
            &mut app,
            Message::Act(Action::Split(pane_grid::Axis::Vertical)),
        );

        let first = app.windows.values().next().expect("one window").focus;
        tell(&mut app, Message::Act(Action::NextTile));
        let second = app.windows.values().next().expect("one window").focus;
        assert_ne!(first, second);

        tell(&mut app, Message::Act(Action::NextTile));
        let third = app.windows.values().next().expect("one window").focus;
        assert_eq!(first, third, "round again");
    }

    /// A message naming a buffer that is gone must not panic.
    #[test]
    fn a_message_for_a_missing_buffer_is_ignored() {
        let mut app = app();
        tell(&mut app, Message::List(9, list::Action::SelectAll));
        tell(&mut app, Message::Listed(9, 0, buffer::Update::Done));
    }
}
