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
    /// A right click on a row.
    Context { row: usize },
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
    let mut app = App {
        config,
        buffers: vec![Buffer::new(start.clone())],
        windows: HashMap::new(),
        notice,
        dialogue: None,
        typed: String::new(),
        filtering: false,
        icon_font: icon_font(&config_font),
        menu: None,
        pointer: (0.0, 0.0),
        size: iced::Size::new(config_width, config_height),
        places: places::list(),
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
            // Anything done to a tile focuses it first. The list widget
            // captures its own presses, so `pane_grid`'s `on_click` never
            // sees them and the keyboard would stay on whichever tile had it
            // -- you could select a row in one tile and then find the arrows
            // moving a cursor in the other.
            focus_showing(app, index);

            let Some(action) = translate(index, found, app.config.list.layout) else {
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

        Message::Pointer(x, y) => {
            app.pointer = (x, y);
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
fn translate(buffer: usize, found: list::Action, current: crate::config::Layout) -> Option<Action> {
    Some(match found {
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
        list::Action::Layout(None) => Action::Layout(current.next()),
        list::Action::Layout(Some(layout)) => Action::Layout(layout),
        list::Action::SplitRight => Action::Split(pane_grid::Axis::Vertical),
        list::Action::SplitDown => Action::Split(pane_grid::Axis::Horizontal),
        list::Action::CloseTile => Action::CloseTile,
        list::Action::NextTile => Action::NextTile,
        list::Action::PreviousTile => Action::PreviousTile,
        list::Action::Escape => Action::Escape,
        list::Action::Menu { row, at } => Action::Menu {
            kind: MenuKind::Context { row },
            buffer,
            at: (at.x, at.y),
        },
    })
}

/// Resolve the icon family, once.
fn icon_font(family: &Option<String>) -> Option<iced::Font> {
    family
        .clone()
        .map(|family| iced::Font::with_name(String::leak(family)))
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
pub fn close_menu(app: &mut App) {
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

        Action::Menu { kind, buffer, at } => {
            // A right click already moved the cursor. The menu acts on
            // whatever is selected, so what it will do is on screen before it
            // opens. A toolbar menu changes no selection at all.
            if let MenuKind::Context { row } = kind {
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

        Action::Bookmark { path } => {
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

        Action::ShowHidden(showing) => {
            app.config.list.show_hidden = showing;
            let list = app.config.list.clone();
            for found in &mut app.buffers {
                found.rebuild(&list);
            }
            Task::none()
        }

        Action::Layout(layout) => {
            app.config.list.layout = layout;
            app.notice = Some(format!("{} view", layout.name()));
            Task::none()
        }

        Action::Split(axis) => {
            // The new tile gets a buffer of its own on the same directory, so
            // the two are independent. Pointing a tile at a buffer another
            // one already shows is the other move, and that is what shares a
            // listing -- see `Tiles`.
            let Some(window) = app.windows.values_mut().next() else {
                return Task::none();
            };
            let Some(showing) = window.panes.get(window.focus).copied() else {
                return Task::none();
            };
            let Some(path) = app.buffers.get(showing).map(|found| found.path.clone()) else {
                return Task::none();
            };

            let fresh = app.buffers.len();
            app.buffers.push(Buffer::new(path));

            let Some(window) = app.windows.values_mut().next() else {
                return Task::none();
            };
            let Some((pane, _)) = window.panes.split(axis, window.focus, fresh) else {
                // `pane_grid` refuses a split it cannot fit. Saying so beats
                // a keystroke that does nothing.
                app.buffers.pop();
                return refuse(app, String::from("no room to split"));
            };
            window.focus = pane;

            relist(app, fresh)
        }

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
        focused && app.dialogue.is_none(),
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
                .map(|(index, ())| Message::Changed(index)),
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
            _ => None,
        }
    }

    iced::Subscription::batch(watches.chain([
        window::close_events().map(Message::Closed),
        window::resize_events().map(|(_, size)| Message::Resized(size)),
        iced::event::listen_with(moved),
    ]))
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

    // The crumbs take what is left, so the toolbar stays pinned to the right
    // however long the path is.
    row![container(crumbs).width(Length::Fill), toolbar(app, index)]
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
fn toolbar<'a>(app: &'a App, index: usize) -> Element<'a, Message> {
    let list = &app.config.list;

    // Every codepoint checked against the installed font before use, the way
    // `icon.rs` was built. A guessed one draws an empty box.
    let layout_glyph = match list.layout {
        crate::config::Layout::List => '\u{f0c9}',
        crate::config::Layout::Detail => '\u{f00b}',
        crate::config::Layout::Icons => '\u{f009}',
    };

    let sort_glyph = if list.sort_reversed {
        '\u{f0de}'
    } else {
        '\u{f0dd}'
    };

    let hidden_glyph = if list.show_hidden {
        '\u{f06e}'
    } else {
        '\u{f070}'
    };

    row![
        tool(
            app,
            layout_glyph,
            format!(
                "View: {} \u{2192} {}",
                list.layout.name(),
                list.layout.next().name()
            ),
            false,
            Message::Act(Action::Layout(list.layout.next())),
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
            list.show_hidden,
            Message::Act(Action::ShowHidden(!list.show_hidden)),
        ),
        tool(
            app,
            '\u{f021}',
            String::from("Relist"),
            false,
            Message::Act(Action::Relist { buffer: index }),
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
        tool(
            app,
            '\u{f142}',
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
fn tool<'a>(
    app: &'a App,
    glyph: char,
    says: String,
    lit: bool,
    message: Message,
) -> Element<'a, Message> {
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
        MenuKind::Context { row } => Some(row),
        MenuKind::Sort | MenuKind::Toolbar => None,
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

        items = items.push(item(
            format!("View: {}", list.layout.next().name()),
            Message::Act(Action::Layout(list.layout.next())),
        ));
        items = items.push(item(
            String::from(if list.show_hidden {
                "Hide hidden files"
            } else {
                "Show hidden files"
            }),
            Message::Act(Action::ShowHidden(!list.show_hidden)),
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
    } else {
        if let Some(row) = row
            && entry.is_some()
        {
            items = items.push(item(
                String::from("Open"),
                Message::Act(Action::Activate {
                    buffer: menu.buffer,
                    row,
                }),
            ));
        }

        items = items.push(item(
            String::from("Copy path"),
            Message::Act(Action::CopyPath {
                buffer: menu.buffer,
            }),
        ));

        // Only a directory can be a place, and the current one is offered
        // when the click landed on a file, since that is still useful.
        let bookmarkable = directory
            .map(|entry| entry.path.clone())
            .or_else(|| buffer.map(|found| found.path.clone()));

        if let Some(path) = bookmarkable {
            let label = match directory {
                Some(entry) => format!("Add {} to places", entry.name),
                None => String::from("Add this directory to places"),
            };
            items = items.push(item(label, Message::Act(Action::Bookmark { path })));
        }
    }

    // Kept inside the window. A menu opened near the right edge would
    // otherwise be clipped, and its longest line would wrap instead of the
    // menu simply moving left -- which is what a sort menu on the toolbar did,
    // since the toolbar lives at the right-hand end of the bar.
    const WIDE: f32 = 230.0;
    const TALL: f32 = 200.0;

    let at = (
        menu.at.0.min((app.size.width - WIDE).max(0.0)),
        menu.at.1.min((app.size.height - TALL).max(0.0)),
    );

    let panel = container(items)
        .width(Length::Fixed(WIDE))
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
    let mut last = None;

    for place in &app.places {
        // A rule between the four groups, so the panel reads as a short list
        // of short lists rather than one long one.
        if last.is_some_and(|kind| kind != place.kind) {
            list = list.push(iced::widget::Space::new().height(6));
        }
        last = Some(place.kind);

        list = list.push(
            button(text(place.label.clone()).size(13))
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
                })),
        );
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
    .width(Length::Fixed(180.0))
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
fn filter_id() -> iced::widget::Id {
    iced::widget::Id::new("ricedir-filter")
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
            icon_font: None,
            menu: None,
            pointer: (0.0, 0.0),
            size: iced::Size::new(1100.0, 700.0),
            places: Vec::new(),
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
                    row: 1,
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
                kind: MenuKind::Context { row: 1 },
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
            kind: MenuKind::Context { row: 0 },
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
            kind: MenuKind::Context { row: 0 },
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
    fn showing_hidden_files_reaches_every_buffer() {
        let mut app = app();
        let list = Config::default().list;
        app.buffers.push(Buffer::new(PathBuf::from("/tmp/two")));

        for buffer in &mut app.buffers {
            buffer.extend(
                vec![entry("plain", Kind::File), entry(".hidden", Kind::File)],
                &list,
            );
            buffer.finish(&list);
        }
        assert_eq!(app.buffers[1].rows(), 1);

        tell(&mut app, Message::Act(Action::ShowHidden(true)));
        assert_eq!(app.buffers[0].rows(), 2);
        assert_eq!(app.buffers[1].rows(), 2, "the buffer behind too");
    }

    /// Splitting gives the new tile a buffer of its own, so the two are
    /// independent. Sharing is what pointing a tile at an existing buffer
    /// does, and that is a different move.
    #[test]
    fn a_split_makes_a_second_buffer_on_the_same_directory() {
        let mut app = app();
        app.windows.insert(window::Id::unique(), Tiles::new(0));
        assert_eq!(app.buffers.len(), 1);

        tell(
            &mut app,
            Message::Act(Action::Split(pane_grid::Axis::Vertical)),
        );

        assert_eq!(app.buffers.len(), 2, "a buffer of its own");
        assert_eq!(app.buffers[1].path, app.buffers[0].path, "same directory");

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
