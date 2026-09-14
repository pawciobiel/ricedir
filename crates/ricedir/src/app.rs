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
use crate::jobs;
use crate::open::{self, Plan, scan};
use crate::places::{self, Place};
use crate::widget::flourish::{self, Flourish};
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

/// What is being dragged.
///
/// Where it would land is `App::over_place`, which the panel keeps up to date
/// whether or not anything is being dragged, because it is also what draws
/// the row under the pointer as hovered.
#[derive(Debug, Clone)]
pub struct Dragging {
    /// What was picked up. Anything at all when it came from a listing;
    /// directories only from the places panel, where a file cannot be a
    /// place and picking one up would promise a drop that is refused.
    pub paths: Vec<PathBuf>,
    pub source: Source,
    /// Which buffer it was picked up from, when it came from a listing.
    ///
    /// A drop back into the same directory would be a job with nothing to do,
    /// so the destination is checked against this before anything is made.
    pub from: Option<usize>,
    /// Which tile the pointer is over, and which row inside it.
    ///
    /// `Some((buffer, None))` is the tile's own directory, which is what
    /// empty space below the last row means.
    pub over: Option<(usize, Option<usize>)>,
}

/// Where a drag started, which is what decides what a drop means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A row in a listing. Dropping it on the panel adds a bookmark.
    List,
    /// A place already in the panel. Dropping it on another reorders them.
    Place,
}

/// A press on a place, held until it is known whether it was a click.
///
/// A press both opens a place and may begin a drag, so the two are told apart
/// the way the listing tells them apart: by the pointer moving [`DRAG`]
/// pixels. Until it does, nothing has happened yet.
#[derive(Debug, Clone)]
struct Pressed {
    /// Which place, as an index into `App::places`.
    place: usize,
    /// The tile that would go there, if this turns out to be a click.
    buffer: usize,
    /// Where the pointer was when the button went down.
    at: (f32, f32),
    /// Whether it has since moved far enough to be a drag.
    dragged: bool,
}

// How far the pointer moves before a press becomes a drag. The list widget's
// own threshold, because it is the same question, and two answers would be
// felt as the window behaving differently on each side of the divider.
use list::DRAG;

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
    /// Which key does what. A table nobody can read is one nobody edits.
    Keys,
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

/// The places panel, on show and hidden.
///
/// `cod-layout_sidebar_left` and `cod-layout_sidebar_left_off`, read out of
/// the font's own glyph names rather than remembered, and then rendered to
/// be sure. The same codicon family as the two above.
const SIDEBAR_ON: char = '\u{ebf3}';
const SIDEBAR_OFF: char = '\u{ec02}';

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
    // The filter box and the path bar's text face belong to the buffer, not
    // to the window. See `Buffer::filtering` and `Buffer::typing_path`.
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
    /// What is being dragged, if anything.
    ///
    /// Inside this window only. `window::Event::FileDropped` brings files
    /// *in* from other applications; winit cannot drag them out on Wayland.
    dragging: Option<Dragging>,
    /// A press on a place that is not yet a click or a drag.
    pressed: Option<Pressed>,
    /// Which modifiers are held, window-wide. A drop reads them as it lands.
    modifiers: iced::keyboard::Modifiers,
    /// What a drag looks like: the icon in hand, where it went, the burst.
    ///
    /// Drawn over everything and read by nothing else. `update` stays the
    /// only writer; the overlay widget takes no events at all.
    flourish: flourish::State,
    /// The place the pointer is over, as an index into `places`.
    ///
    /// `places.len()` means the strip under the last one, which is how
    /// something is dropped at the end. `None` means the pointer is not on
    /// the panel at all.
    ///
    /// Kept whether or not a drag is going on: it draws the hovered row as
    /// well as saying where a drop would land.
    over_place: Option<usize>,
    /// Whether the panel down the left is on show.
    ///
    /// One window's worth, not one per tile: it is one panel. Remembered
    /// between runs in `state.toml`.
    sidebar: bool,
    /// Every job, running, waiting or finished.
    ///
    /// Storage, not transport. A worker never touches this: it sends what it
    /// did down a channel and `update` writes it here, which keeps one
    /// writer. See `## Decided: how a job tells the window what changed`.
    jobs: jobs::Queue,
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
    /// Show or hide the filter box, in one buffer.
    Filtering(usize, bool),
    /// The path bar's text face: what has been typed, and going there or back.
    PathTyped(usize, String),
    TypingPath(usize, bool),
    /// The typed path was submitted.
    PathSubmitted(usize),
    /// Escape, from the subscription rather than the list. Used while a text
    /// box holds the keyboard; everything else Escape does still comes
    /// through the list widget.
    ///
    /// The window, because the subscription knows no tile. It reaches the
    /// buffer the focused tile of that window is showing, which is the one
    /// whose text face is on screen.
    Escaped(window::Id),
    /// Tab, likewise: complete the path being typed.
    CompletePath(window::Id),
    /// The pointer went down on a place. Not yet a click: it may be a drag.
    PlacePressed {
        place: usize,
        buffer: usize,
    },
    /// The pointer moved onto a place, or off one. The index is into
    /// `App::places`, and one past the end is the strip below them.
    PlaceEntered(usize),
    PlaceLeft(usize),
    /// The left button came up, anywhere. This finishes a drag and settles
    /// whether a press on a place was a click.
    Released,
    /// Which modifiers are held now.
    Modifiers(iced::keyboard::Modifiers),
    /// Enter, in the filter box.
    FilterSubmitted(usize),
    /// A frame, while something is being animated.
    Frame(std::time::Instant),
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
    /// A job said something: its plan arrived, or it got somewhere.
    Job(jobs::Message),
}

pub fn new(mut config: Config, start: PathBuf) -> (App, Task<Message>) {
    let (id, opened) = window::open(window::Settings {
        size: iced::Size::new(config.window.width, config.window.height),
        min_size: Some(iced::Size::new(480.0, 320.0)),
        ..window::Settings::default()
    });

    let notice = config.problem.clone();
    let config_font = config.list.icon_font.clone();
    let (config_width, config_height) = (config.window.width, config.window.height);
    // What was last switched to wins over the built-in default, but never
    // over a config that names a layout. Somebody who wrote `layout =
    // "detail"` in a file means it, and a stray keystroke should not
    // quietly overrule the file they edited.
    let remembered = crate::state::State::load();
    if let Some(layout) = remembered.layout
        && !config.list.layout_named
    {
        config.list.layout = layout;
    }

    let config_view = config.list.view();
    // At least one, or a config that said nought would take jobs on and
    // never start any of them.
    let workers = config.jobs.workers.max(1);
    let mut app = App {
        config,
        buffers: vec![Buffer::new(start, config_view)],
        windows: HashMap::new(),
        notice,
        dialogue: None,
        typed: String::new(),
        icon_font: config_font.as_deref().map(icon_font),
        menu: None,
        pointer: (0.0, 0.0),
        size: iced::Size::new(config_width, config_height),
        places: places::list(),
        sidebar: remembered.sidebar.unwrap_or(true),
        jobs: jobs::Queue::new(workers),
        tick: 0,
        dragging: None,
        modifiers: iced::keyboard::Modifiers::default(),
        flourish: flourish::State::default(),
        pressed: None,
        over_place: None,
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

        // Where a drop would land, while the pointer is still moving. Not an
        // action and not a focus change: hovering must not take the keyboard
        // off the tile a person dragged *from*, or letting go would act on
        // the wrong selection.
        Message::List(index, list::Action::Over(row)) => {
            if let Some(dragging) = &mut app.dragging {
                dragging.over = Some((index, row));
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
            let moved = focus_showing(app, index);

            let showing = app
                .buffers
                .get(index)
                .map_or_else(|| app.config.list.view(), |found| found.view);

            let acted = action::dispatch(
                app,
                translate(index, found, showing, app.pointer, app.sidebar),
            );

            if moved {
                Task::batch([acted, refocus(app, index)])
            } else {
                acted
            }
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

        Message::Job(jobs::Message::Planned(id, made)) => {
            // Say what it will do, or why it will not. A plan that found
            // nothing, or a destination that is not there, is the answer to
            // a question somebody asked and has to reach them.
            match &made {
                Ok(plan) => app.notice = Some(plan.describe()),
                Err(why) => app.notice = Some(why.clone()),
            }

            app.jobs.planned(id, made);

            // A plan that found something already at the other end stops and
            // asks. One dialogue for the whole job, and nothing runs until it
            // is answered.
            if let Some(job) = app.jobs.get(id)
                && job.state == jobs::State::Asking
                && let Some(plan) = &job.plan
            {
                app.dialogue = Some(Dialogue::Clashing {
                    job: id,
                    work: plan.work,
                    clashes: plan.clashes.clone(),
                });
            }

            app.jobs.start_ready().map(Message::Job)
        }

        Message::Job(jobs::Message::Said(id, said)) => {
            let ended = matches!(said, jobs::Update::Ended(_));
            let freed = app.jobs.update(id, said);

            // A job that ended says what it left behind. The listing looks
            // after itself: the destination is watched if anybody is looking
            // at it, so the new files arrive the way any other change does.
            if ended && let Some(job) = app.jobs.get(id) {
                app.notice = Some(job.ending());
            }

            if freed {
                app.jobs.start_ready().map(Message::Job)
            } else {
                Task::none()
            }
        }

        Message::Modifiers(modifiers) => {
            app.modifiers = modifiers;
            Task::none()
        }

        // A frame, while something is animating. Nothing is worked out here:
        // every effect reads the clock itself when it draws, so all this does
        // is throw away what has finished, which is what ends the
        // subscription.
        Message::Frame(now) => {
            app.flourish.tidy(now);
            Task::none()
        }

        Message::Pointer(x, y) => {
            app.pointer = (x, y);
            pick_up_place(app);

            // The icon goes where the hand goes.
            if let Some(held) = &mut app.flourish.held {
                held.at = iced::Point::new(x, y);
            }
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

        Message::TileClicked(pane) => match focus_tile(app, pane) {
            Some(buffer) => refocus(app, buffer),
            None => Task::none(),
        },

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

        Message::Filtering(index, showing) => {
            let list = app.config.list.clone();
            let Some(buffer) = app.buffers.get_mut(index) else {
                return Task::none();
            };
            buffer.filtering = showing;

            if showing {
                // A box that appears without focus is a box that swallows the
                // first thing typed into it -- `/` then `file1` put the box on
                // screen and the text nowhere. `focus` unfocuses everything
                // else in the same traversal, so nothing has to be told to let
                // go first.
                return iced::widget::operation::focus(filter_id());
            }

            // Leaving the box behind with text in it would leave the listing
            // narrowed and nothing on screen saying why. This buffer only:
            // clearing every filter in the window used to undo the narrowing
            // in the tile beside it, which nobody asked about.
            if !buffer.filter.is_empty() {
                buffer.filter.clear();
                buffer.rebuild(&list);
            }
            Task::none()
        }

        // Enter in the filter box means "this one", the way it does in every
        // launcher. `text_input` captures Enter, so the list never sees it
        // and cannot activate the row itself.
        //
        // The row is read before anything else happens: entering a directory
        // drops the filter, which re-arranges the listing and makes the same
        // number a different entry.
        Message::FilterSubmitted(index) => {
            let Some(row) = app.buffers.get(index).map(|buffer| buffer.cursor) else {
                return Task::none();
            };

            // Nothing matched, so there is nothing to open. Put the box away
            // rather than leave a listing narrowed to nothing. Done here
            // rather than through `Action::Filtering`, which answers with a
            // `Task`: this arm already holds `&mut App`.
            if app
                .buffers
                .get(index)
                .is_none_or(|buffer| buffer.rows() == 0)
            {
                let list = app.config.list.clone();
                if let Some(buffer) = app.buffers.get_mut(index) {
                    buffer.leaving(&list);
                }
                return Task::none();
            }

            action::dispatch(app, Action::Activate { buffer: index, row })
        }

        Message::TypingPath(index, typing) => {
            let Some(buffer) = app.buffers.get_mut(index) else {
                return Task::none();
            };

            if !typing {
                buffer.typing_path = None;
                return Task::none();
            }

            // Starts as the path it is showing, so the common thing -- take a
            // piece of this path -- needs no typing at all.
            buffer.typing_path = Some(buffer.path.to_string_lossy().into_owned());

            // Focused *and* selected: turning the bar over to copy half a
            // path should not need a drag from one end first.
            iced::widget::operation::focus(path_id())
                .chain(iced::widget::operation::select_all(path_id()))
        }

        Message::PathTyped(index, text) => {
            if let Some(buffer) = app.buffers.get_mut(index) {
                buffer.typing_path = Some(text);
            }
            Task::none()
        }

        Message::PlacePressed { place, buffer } => {
            app.pressed = Some(Pressed {
                place,
                buffer,
                at: app.pointer,
                dragged: false,
            });
            Task::none()
        }

        Message::PlaceEntered(at) => {
            app.over_place = Some(at);
            Task::none()
        }

        Message::PlaceLeft(at) => {
            // Only if it is still the one being left. Leaving one row and
            // entering the next arrive in whichever order the widgets were
            // built in, and clearing unconditionally would lose the new one.
            if app.over_place == Some(at) {
                app.over_place = None;
            }
            Task::none()
        }

        Message::Released => {
            let dropped = finish_drag(app);

            // A press that never moved far enough is a click, and a click on
            // a place opens it. On release rather than on press, because
            // until the button comes up it may still become a drag.
            match app.pressed.take() {
                Some(pressed) if !pressed.dragged => {
                    let Some(place) = app.places.get(pressed.place) else {
                        return dropped;
                    };
                    let path = place.path.clone();
                    Task::batch([
                        dropped,
                        action::dispatch(
                            app,
                            Action::Go {
                                buffer: pressed.buffer,
                                path,
                            },
                        ),
                    ])
                }
                _ => dropped,
            }
        }

        Message::Escaped(window) => {
            let Some(index) = focused_buffer(app, window) else {
                return Task::none();
            };

            // Only while a text box has the keyboard. `text_input` captures
            // Escape and unfocuses itself, so the list never sees it and
            // cannot report it. With no box up the list reports it as usual,
            // and acting here as well would spend two steps on one press.
            let boxed = app
                .buffers
                .get(index)
                .is_some_and(|buffer| buffer.typing_path.is_some() || buffer.filtering);

            if !boxed {
                return Task::none();
            }

            action::dispatch(app, Action::Escape { buffer: index })
        }

        Message::CompletePath(window) => {
            let Some(index) = focused_buffer(app, window) else {
                return Task::none();
            };
            let Some(longer) = app
                .buffers
                .get(index)
                .and_then(|buffer| buffer.typing_path.as_deref())
                .and_then(complete)
            else {
                return Task::none();
            };

            if let Some(buffer) = app.buffers.get_mut(index) {
                buffer.typing_path = Some(longer);
            }
            // The caret goes to the end, or the next keystroke lands in the
            // middle of what was just filled in.
            iced::widget::operation::move_cursor_to_end(path_id())
        }

        Message::PathSubmitted(index) => {
            let Some(text) = app
                .buffers
                .get_mut(index)
                .and_then(|buffer| buffer.typing_path.take())
            else {
                return Task::none();
            };

            let path = expand(&text);

            // A path that is not a directory says so and stays on screen with
            // what was typed still in it. Going nowhere and clearing the box
            // would look like the keystroke was lost.
            if !path.is_dir() {
                if let Some(buffer) = app.buffers.get_mut(index) {
                    buffer.typing_path = Some(text);
                }
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
    view: crate::config::View,
    pointer: (f32, f32),
    sidebar: bool,
) -> Action {
    let crate::config::View {
        layout: current,
        show_hidden: showing_hidden,
    } = view;

    match found {
        // Handled before this, in `Message::List`, because it must not focus
        // the tile it passes over. It cannot reach here.
        list::Action::Over(_) => Action::Cursor,
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
        list::Action::Back => Action::Back { buffer },
        list::Action::Forward => Action::Forward { buffer },
        list::Action::CopyPath => Action::CopyPath { buffer },
        // The widget cannot read the flag it is toggling, so `translate`
        // does: it has the buffer's view to hand and the widget does not.
        list::Action::ShowHidden => Action::ShowHidden {
            buffer,
            showing: !showing_hidden,
        },
        list::Action::Filter => Action::Filtering {
            buffer,
            showing: true,
        },
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
        list::Action::Escape => Action::Escape { buffer },
        list::Action::Delete => Action::Delete { buffer },
        list::Action::Sidebar => Action::Sidebar { showing: !sidebar },
        list::Action::Bookmark => Action::Bookmark { buffer, path: None },
        list::Action::Buffers => Action::Menu {
            kind: MenuKind::Buffers,
            buffer,
            at: pointer,
        },
        list::Action::Keys => Action::Menu {
            kind: MenuKind::Keys,
            buffer,
            at: pointer,
        },
        list::Action::DragRow(row) => Action::Drag { buffer, row },
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

/// Turn a held press on a place into a drag, once the pointer has moved.
///
/// Past the threshold the press is a drag whatever comes of it, so a release
/// after this no longer opens the place. That holds even when nothing is
/// picked up: a pointer dragged off the home directory did not mean to go
/// there.
fn pick_up_place(app: &mut App) {
    let Some(pressed) = app.pressed.as_ref() else {
        return;
    };
    if pressed.dragged {
        return;
    }

    let (place, at) = (pressed.place, pressed.at);
    if (app.pointer.0 - at.0).hypot(app.pointer.1 - at.1) < DRAG {
        return;
    }

    if let Some(pressed) = app.pressed.as_mut() {
        pressed.dragged = true;
    }

    // Only a bookmark moves. Home, the user directories and a mounted disk
    // are in the panel because the system says they are, and ricedir has no
    // order of its own to move one into.
    let Some(place) = app.places.get(place) else {
        return;
    };
    if place.kind != places::Kind::Bookmark {
        return;
    }

    app.dragging = Some(Dragging {
        paths: vec![place.path.clone()],
        source: Source::Place,
        from: None,
        over: None,
    });
}

/// Where a dragged place would land.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Lands {
    /// In front of this bookmark.
    Before(PathBuf),
    /// After all of them, which is what the strip below the panel is for.
    AtTheEnd,
    /// Nowhere: the row under the pointer is not one ricedir puts in order.
    Nowhere,
}

/// Where a place dragged onto row `over` would land.
fn lands(places: &[Place], over: usize, from: &Path) -> Lands {
    // Past the last place, which is the strip that means "the end".
    let Some(target) = places.get(over) else {
        return Lands::AtTheEnd;
    };

    // Home, the user directories and a mounted disk are in the panel because
    // the system says so. There is no order of ours to drop one into.
    if target.kind != places::Kind::Bookmark || target.path == from {
        return Lands::Nowhere;
    }

    Lands::Before(target.path.clone())
}

/// Whether the marker belongs above this row.
fn lands_here(app: &App, at: usize) -> bool {
    let Some(dragging) = &app.dragging else {
        return false;
    };

    dragging.source == Source::Place
        && app.over_place == Some(at)
        && dragging
            .paths
            .first()
            .is_some_and(|from| lands(&app.places, at, from) != Lands::Nowhere)
}

/// A drop on a listing: work out where it lands, and ask what it means.
///
/// A row that is a directory takes the drop; anything else means the
/// directory the tile is showing, which is also what empty space below the
/// last row means.
///
/// The modifier is read at the drop rather than at the press, because people
/// reach for it after they have started dragging. Without one the question is
/// asked, because copy and move are not the same mistake.
fn drop_into(
    app: &mut App,
    dragging: &Dragging,
    buffer: usize,
    row: Option<usize>,
) -> (Task<Message>, bool) {
    let Some(found) = app.buffers.get(buffer) else {
        return (Task::none(), false);
    };

    let into = row
        .and_then(|row| found.at(row))
        .filter(|entry| entry.kind.is_directory())
        .map_or_else(|| found.path.clone(), |entry| entry.path.clone());

    // Onto itself, or into where it already is. Both are a job that would do
    // nothing, and saying so beats a row in the panel that reports success.
    let sources: Vec<PathBuf> = dragging
        .paths
        .iter()
        .filter(|path| path.parent() != Some(into.as_path()) && *path != &into)
        .cloned()
        .collect();

    if sources.is_empty() {
        return (Task::none(), false);
    }

    // The job is made here rather than through `Action::Copy`, because that
    // one reads the tile's selection and a drag carries its own paths: a row
    // that was not selected drags only itself. The same shape as
    // `Choice::Delete`, where the dialogue answer makes the job and the
    // action was only what asked.
    let held = app.modifiers;
    if held.control() || held.shift() {
        let work = if held.shift() {
            jobs::Work::Move
        } else {
            jobs::Work::Copy
        };
        let (_, planning) = app.jobs.add(work, sources, into);
        return (planning.map(Message::Job), true);
    }

    app.dialogue = Some(Dialogue::Dropping {
        sources,
        into,
        buffer,
    });
    (Task::none(), true)
}

/// End a drag, and do whatever it turned out to be.
///
/// Both ends report a release -- the list widget captures its own, and the
/// subscription sees every one -- so this takes the drag rather than reading
/// it, and whichever arrives second finds nothing left to do.
fn finish_drag(app: &mut App) -> Task<Message> {
    let Some(dragging) = app.dragging.take() else {
        return Task::none();
    };

    let now = std::time::Instant::now();

    // Dropped on a listing. The panel is checked after, so a pointer over a
    // tile is never also read as a drop on the places panel.
    if dragging.source == Source::List
        && let Some((buffer, row)) = dragging.over
    {
        let (started, took) = drop_into(app, &dragging, buffer, row);

        // The burst goes where the pointer is, which is where the person was
        // looking. A drop the tile would not take springs back instead: a
        // burst would say something landed when nothing did.
        let at = iced::Point::new(app.pointer.0, app.pointer.1);
        if took {
            app.flourish.landed(at, Some(buffer), now);
        } else {
            app.flourish.refused(now);
        }

        return started;
    }

    // Everything else: onto the panel, or onto nothing at all. The icon goes
    // back where it came from rather than vanishing, so a drag that missed
    // says so.
    app.flourish.refused(now);

    // Dropped somewhere that is not the panel. Nothing happens, quietly:
    // a drag that lands nowhere is how a drag is called off.
    let Some(over) = app.over_place else {
        return Task::none();
    };

    match dragging.source {
        Source::List => {
            let mut added = 0;
            for path in &dragging.paths {
                if places::bookmark(path).is_ok() {
                    added += 1;
                }
            }

            app.places = places::list();
            app.notice = Some(match added {
                0 => String::from("nothing was added to your places"),
                1 => format!("{} is in your places", dragging.paths[0].display()),
                many => format!("{many} directories are in your places"),
            });
        }

        Source::Place => {
            let Some(from) = dragging.paths.first() else {
                return Task::none();
            };

            let moved = match lands(&app.places, over, from) {
                Lands::Nowhere => return Task::none(),
                Lands::AtTheEnd => places::move_bookmark(from, None),
                Lands::Before(before) => places::move_bookmark(from, Some(&before)),
            };

            match moved {
                Ok(()) => app.places = places::list(),
                Err(error) => {
                    app.notice = Some(format!("could not reorder the places: {error}"));
                }
            }
        }
    }

    Task::none()
}

/// Write what should survive the next start.
///
/// Everything at once, because the file holds one table: writing only the
/// field that changed would drop the other one. `None` for the layout means
/// keep whatever was last remembered, which is what a config that names a
/// layout leaves in there.
fn remember_view(app: &App, layout: Option<crate::config::Layout>) {
    crate::state::State {
        layout: layout.or_else(|| crate::state::State::load().layout),
        sidebar: Some(app.sidebar),
    }
    .save();
}

/// Which buffer the focused tile of one window is showing.
///
/// The subscription reports a key with a window and no tile, so this is how a
/// keystroke that belongs to the path bar finds the directory it is about.
fn focused_buffer(app: &App, window: window::Id) -> Option<usize> {
    let tiles = app.windows.get(&window)?;
    tiles.panes.get(tiles.focus).copied()
}

/// Give one tile the keyboard, and say which buffer it landed on.
///
/// `None` when the keyboard was already there, so a caller can tell a move
/// from a click on the tile that already had it.
///
/// Every window is asked, because a `Pane` belongs to exactly one of them and
/// there is no cheaper way to say which from a click alone.
fn focus_tile(app: &mut App, pane: pane_grid::Pane) -> Option<usize> {
    for tiles in app.windows.values_mut() {
        let Some(buffer) = tiles.panes.get(pane).copied() else {
            continue;
        };
        if tiles.focus == pane {
            return None;
        }
        tiles.focus = pane;
        return Some(buffer);
    }
    None
}

/// Give the keyboard to whichever tile is showing this buffer.
///
/// The first one found: two tiles can show one buffer, and then either will
/// do -- they are the same listing and the same cursor.
fn focus_showing(app: &mut App, buffer: usize) -> bool {
    for tiles in app.windows.values_mut() {
        let found = tiles
            .panes
            .iter()
            .find(|(_, index)| **index == buffer)
            .map(|(pane, _)| *pane);

        if let Some(pane) = found {
            let moved = tiles.focus != pane;
            tiles.focus = pane;
            return moved;
        }
    }
    false
}

/// Give the keyboard back to whatever box the tile now in front has up.
///
/// A tile showing the path bar's text face draws a box the list deliberately
/// will not compete with, so if nothing holds iced's text focus that tile is
/// dead: the box takes no keys, and neither does the list behind it. Moving
/// the keyboard away and back used to leave it exactly like that.
fn refocus(app: &App, buffer: usize) -> Task<Message> {
    let Some(found) = app.buffers.get(buffer) else {
        return Task::none();
    };

    if found.typing_path.is_some() {
        return iced::widget::operation::focus(path_id());
    }
    if found.filtering {
        return iced::widget::operation::focus(filter_id());
    }
    Task::none()
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

        Action::Filtering { buffer, showing } => Task::done(Message::Filtering(buffer, showing)),

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

        Action::Drag { buffer, row } => {
            let Some(found) = app.buffers.get(buffer) else {
                return Task::none();
            };

            // A row inside the selection drags the whole selection; a row
            // outside it drags only that one. Every file manager does this,
            // and the alternative -- dragging whatever was selected an hour
            // ago -- is how people move the wrong files.
            let paths: Vec<PathBuf> = if found.is_selected(row) {
                found.selected().map(|entry| entry.path.clone()).collect()
            } else {
                found
                    .at(row)
                    .map(|entry| entry.path.clone())
                    .into_iter()
                    .collect()
            };

            // Anything at all. The places panel takes only directories and
            // says so when a file lands on it; another tile takes whatever
            // it is given, which is the ordinary reason to drag a file.
            if !paths.is_empty() {
                // The icon in hand. Worked out here, where the entry is:
                // `flourish` never reads a buffer.
                let picked = found.at(row);
                app.flourish.held = Some(flourish::Held::new(
                    picked.map(crate::icon::of),
                    picked.map_or_else(
                        || format!("{} things", paths.len()),
                        |entry| entry.name.clone(),
                    ),
                    paths.len(),
                    iced::Point::new(app.pointer.0, app.pointer.1),
                    std::time::Instant::now(),
                ));

                app.dragging = Some(Dragging {
                    paths,
                    source: Source::List,
                    from: Some(buffer),
                    over: None,
                });
            }
            Task::none()
        }

        Action::MovePlace { from, before } => {
            match crate::places::move_bookmark(&from, before.as_deref()) {
                Ok(()) => app.places = places::list(),
                Err(error) => {
                    app.notice = Some(format!("could not reorder the places: {error}"));
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

            // Remembered for the next run, in ricedir's own state file --
            // never written back into the config, which is a file a person
            // edits and comments.
            remember_view(app, Some(layout));

            Task::none()
        }

        Action::Sidebar { showing } => {
            app.sidebar = showing;
            remember_view(app, None);
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
            let now = window.buffer();

            // The buffer stays open. That is the point of buffers: closing a
            // tile costs nothing and reopening the directory is instant.
            refocus(app, now)
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
            let now = window.buffer();
            refocus(app, now)
        }

        Action::Relist { buffer } => relist(app, buffer),

        Action::Copy { buffer, into } => {
            let Some(sources) = chosen(app, buffer) else {
                return refuse(app, String::from("nothing is selected"));
            };

            let (_, planning) = app.jobs.add(jobs::Work::Copy, sources, into);
            planning.map(Message::Job)
        }

        Action::Move { buffer, into } => {
            let Some(sources) = chosen(app, buffer) else {
                return refuse(app, String::from("nothing is selected"));
            };

            let (_, planning) = app.jobs.add(jobs::Work::Move, sources, into);
            planning.map(Message::Job)
        }

        Action::Resolve { job, how } => {
            app.jobs.resolve(job, how);
            app.dialogue = None;
            app.jobs.start_ready().map(Message::Job)
        }

        Action::Delete { buffer } => {
            let Some(paths) = chosen(app, buffer) else {
                return refuse(app, String::from("nothing is selected"));
            };

            // Asked, never assumed. This is the only thing between a
            // keystroke and work nobody can get back.
            app.dialogue = Some(Dialogue::Deleting { paths, buffer });
            Task::none()
        }

        Action::PauseJob { job } => {
            app.jobs.pause(job);
            Task::none()
        }
        Action::ResumeJob { job } => {
            app.jobs.resume(job);
            Task::none()
        }
        Action::CancelJob { job } => {
            app.jobs.cancel(job);
            // A job cancelled before it started frees a worker at once. One
            // already running frees it when its thread notices.
            app.jobs.start_ready().map(Message::Job)
        }
        Action::DismissJob { job } => {
            app.jobs.dismiss(job);
            Task::none()
        }

        Action::Escape { buffer } => {
            // One key for "put away whatever is in front of me", in the order
            // things are stacked. The menu and the dialogue are window-wide;
            // the last two belong to the tile that asked.
            if app.menu.take().is_some() {
                return Task::none();
            }
            if app.dialogue.is_some() {
                return Task::done(Message::Dialogue(Choice::Dismiss));
            }
            if let Some(found) = app.buffers.get_mut(buffer)
                && found.typing_path.take().is_some()
            {
                return Task::none();
            }

            // The filter box goes in two steps. A filter typed by mistake is
            // the common case, and one Escape that took the box away with it
            // would mean opening the box again to carry on. An empty box has
            // nothing left to lose, so it goes on the first press.
            //
            // The box is focused again by hand: `text_input` captures Escape
            // and unfocuses itself on the way past, so without this the box
            // would stay on screen and take nothing typed into it.
            let list = app.config.list.clone();
            if let Some(found) = app.buffers.get_mut(buffer)
                && found.filtering
                && !found.filter.is_empty()
            {
                found.filter.clear();
                found.rebuild(&list);
                return iced::widget::operation::focus(filter_id());
            }

            Task::done(Message::Filtering(buffer, false))
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
            found.leaving(&list);

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
            found.leaving(&list);
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
            found.leaving(&list);
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
            found.leaving(&list);
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
            found.leaving(&list);
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
///
/// The answer arrives late on purpose: `open::spawn` watches the child for a
/// moment, so a handler that starts and gives up straight away is reported
/// rather than lost.
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

        // Cancelling the question cancels the job. A job left in `Asking`
        // would sit in the panel for ever, holding a plan nobody answered.
        (Choice::Dismiss, Dialogue::Clashing { job, .. }) => {
            app.typed.clear();
            app.jobs.cancel(job);
            Task::none()
        }

        (Choice::Dismiss, _) => {
            app.typed.clear();
            Task::none()
        }

        (Choice::Resolve(how), Dialogue::Clashing { job, .. }) => {
            Task::done(Message::Act(Action::Resolve { job, how }))
        }

        (Choice::CopyHere, Dialogue::Dropping { sources, into, .. }) => {
            let (_, planning) = app.jobs.add(jobs::Work::Copy, sources, into);
            planning.map(Message::Job)
        }

        (Choice::MoveHere, Dialogue::Dropping { sources, into, .. }) => {
            let (_, planning) = app.jobs.add(jobs::Work::Move, sources, into);
            planning.map(Message::Job)
        }

        (Choice::Delete, Dialogue::Deleting { paths, buffer }) => {
            // The destination is nowhere. A delete job carries the directory
            // it came from so the panel row has somewhere to point at.
            let from = app
                .buffers
                .get(buffer)
                .map_or_else(PathBuf::new, |found| found.path.clone());

            let (_, planning) = app.jobs.add(jobs::Work::Delete, paths, from);
            planning.map(Message::Job)
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
    //
    // The panel goes altogether or not at all, divider included. Leaving the
    // line behind would be a rule down the side of the window with nothing
    // on the other side of it.
    let mut page = row![];

    if app.sidebar {
        page = page.push(sidebar(app, focused));
        // One pixel of `muted`, full height. A `Space` with no height makes
        // the container collapse to nothing, which is a divider you cannot
        // see -- it was written that way once.
        page = page.push(
            container(iced::widget::Space::new())
                .width(1)
                .height(Length::Fill)
                .style(move |_: &iced::Theme| container::Style {
                    background: Some(app.config.theme.muted.color().into()),
                    ..container::Style::default()
                }),
        );
    }

    let page = page.push(
        // `muted` behind the grid, so the gaps `pane_grid` leaves between
        // tiles are lines rather than nothing.
        container(grid)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(app.config.theme.muted.color().into()),
                ..container::Style::default()
            }),
    );

    let page = page.width(Length::Fill).height(Length::Fill);

    // A menu sits over the page, and under a dialogue.
    let page: Element<'_, Message> = match &app.menu {
        Some(menu) => iced::widget::stack![page, context_menu(app, menu)].into(),
        None => page.into(),
    };

    // The drag overlay goes over the tiles and under the dialogue: the icon
    // has to cross tiles and the panel, and a dialogue that opens because of
    // a drop should not have an icon flying across it. It takes no events, so
    // stacking it over the page costs the page nothing.
    let page: Element<'_, Message> = if app.flourish.busy(std::time::Instant::now()) {
        iced::widget::stack![
            page,
            Flourish::new(&app.flourish, &app.config, app.icon_font),
        ]
        .into()
    } else {
        page
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
        &app.config,
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
        focused && app.dialogue.is_none() && buffer.typing_path.is_none(),
        move |action| Message::List(index, action),
    );

    // Only the tile the pointer is over wears the marker, and only a row
    // that could actually take a drop: a file cannot, and the marker would
    // then promise something that does not happen.
    let dragging = app
        .dragging
        .as_ref()
        .filter(|dragging| dragging.source == Source::List);
    let over = dragging.and_then(|dragging| match dragging.over {
        Some((tile, row)) if tile == index => row.filter(|row| {
            buffer
                .at(*row)
                .is_some_and(|entry| entry.kind.is_directory())
        }),
        _ => None,
    });
    let list = list
        .dropping(dragging.is_some(), over)
        .shaken(app.flourish.shake(index, std::time::Instant::now()));

    let mut page = column![path_bar(app, index, buffer, focused)];

    if buffer.filtering && focused {
        page = page.push(
            container(
                iced::widget::text_input("filter, or :glob", &buffer.filter)
                    .id(filter_id())
                    .on_input(move |text| Message::Filter(index, text))
                    .on_submit(Message::FilterSubmitted(index))
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
        window: window::Id,
    ) -> Option<Message> {
        match event {
            iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                Some(Message::Pointer(position.x, position.y))
            }
            // Which modifiers are held, window-wide. A drop reads them at the
            // moment it lands, and by then the pointer may be over a tile
            // that never saw the key go down.
            iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
                Some(Message::Modifiers(modifiers))
            }
            // Every release, wherever it lands. A drag that ends outside the
            // panel has to be called off, and the panel never hears about
            // one: `mouse_area` reports a release only over itself.
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                Some(Message::Released)
            }
            // The path bar's text face takes the keyboard away from the list,
            // so nothing else is left to report Escape while it is open.
            // `update` ignores this unless the box is actually up.
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                ..
            }) => Some(Message::Escaped(window)),
            // Tab, for the same reason. `text_input` does not capture it
            // either, so without this it would reach nothing at all.
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
                ..
            }) => Some(Message::CompletePath(window)),
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

    // A frame per redraw, and only while a drag or a burst is playing. Held
    // open for ever this is a file manager that keeps a laptop's GPU awake
    // for nothing; `State::busy` is what closes it again.
    let animating = app
        .flourish
        .busy(std::time::Instant::now())
        .then(|| window::frames().map(Message::Frame));

    iced::Subscription::batch(
        watches.chain(
            [
                window::close_events().map(Message::Closed),
                window::resize_events().map(|(_, size)| Message::Resized(size)),
                iced::event::listen_with(moved),
            ]
            .into_iter()
            .chain(ticking)
            .chain(animating),
        ),
    )
}

/// Breadcrumbs, and the two arrows.
///
/// Each component is a button rather than one long string, because the thing
/// people want from a path bar is to jump three levels up without typing.
fn path_bar<'a>(
    app: &'a App,
    index: usize,
    buffer: &'a Buffer,
    focused: bool,
) -> Element<'a, Message> {
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
    //
    // Only the focused tile draws the box, because `path_id` is one id and
    // two widgets wearing it would fight over the keyboard. `focused` comes
    // from the pane this is drawn in, not from which buffer the window
    // points at: two tiles can show one buffer, and both would have claimed
    // it.
    let middle: Element<'a, Message> = match &buffer.typing_path {
        Some(typed) if focused => iced::widget::text_input("path", typed)
            .id(path_id())
            .on_input(move |text| Message::PathTyped(index, text))
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
            buffer.typing_path.is_some() && focused,
            Message::Act(Action::TypingPath {
                buffer: index,
                typing: buffer.typing_path.is_none(),
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
            if app.sidebar { SIDEBAR_ON } else { SIDEBAR_OFF },
            String::from("Places panel  (F9)"),
            app.sidebar,
            Message::Act(Action::Sidebar {
                showing: !app.sidebar,
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
        MenuKind::Place { .. }
        | MenuKind::Buffers
        | MenuKind::Keys
        | MenuKind::Sort
        | MenuKind::Toolbar => None,
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
            Message::Act(Action::Filtering {
                buffer: menu.buffer,
                showing: true,
            }),
        ));
        items = items.push(item(
            String::from("Open directories\u{2026}  (Ctrl+B)"),
            Message::Act(Action::Menu {
                kind: MenuKind::Buffers,
                buffer: menu.buffer,
                at: app.pointer,
            }),
        ));
        items = items.push(item(
            String::from("Keys\u{2026}  (?)"),
            Message::Act(Action::Menu {
                kind: MenuKind::Keys,
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
    } else if menu.kind == MenuKind::Keys {
        // Read-only. Editing a binding means editing the config, and a
        // half-built editor here would be worse than the file.
        for (chord, action) in app.config.keys.listed() {
            items = items.push(item(
                format!("{chord}    {action}"),
                Message::Act(Action::Escape {
                    buffer: menu.buffer,
                }),
            ));
        }
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
            // Reordering lives here rather than only in a drag. The file is
            // an order and nothing else respected it; a menu item respects
            // it from the keyboard too, and a drag can be added later
            // without changing what it does.
            let marks: Vec<&Place> = app
                .places
                .iter()
                .filter(|one| one.kind == places::Kind::Bookmark)
                .collect();
            let at = marks.iter().position(|one| one.path == place.path);

            if let Some(at) = at
                && at > 0
            {
                items = items.push(item(
                    String::from("Move up"),
                    Message::Act(Action::MovePlace {
                        from: place.path.clone(),
                        before: Some(marks[at - 1].path.clone()),
                    }),
                ));
            }

            if let Some(at) = at
                && at + 1 < marks.len()
            {
                // In front of the one after next, or at the end when there
                // is no such place.
                items = items.push(item(
                    String::from("Move down"),
                    Message::Act(Action::MovePlace {
                        from: place.path.clone(),
                        before: marks.get(at + 2).map(|one| one.path.clone()),
                    }),
                ));
            }

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

        // Into the tile beside this one, which is what a two-panel file
        // manager has always meant by "copy". Offered only when there is
        // another tile, because otherwise there is nowhere to name.
        if let Some((into, name)) = beside(app, menu.buffer) {
            items = items.push(item(
                format!("Copy to {name}"),
                Message::Act(Action::Copy {
                    buffer: menu.buffer,
                    into: into.clone(),
                }),
            ));
            items = items.push(item(
                format!("Move to {name}"),
                Message::Act(Action::Move {
                    buffer: menu.buffer,
                    into,
                }),
            ));
        }

        // Last, and with the ellipsis that says it asks first. A destructive
        // item beside "Open" is how a menu costs somebody a file.
        items = items.push(item(
            String::from("Delete for good\u{2026}  (Shift+Delete)"),
            Message::Act(Action::Delete {
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
    let wide: f32 = if matches!(menu.kind, MenuKind::Buffers | MenuKind::Keys) {
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
    let asked_at = if matches!(menu.kind, MenuKind::Buffers | MenuKind::Keys) {
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
    .on_press(Message::Act(Action::Escape {
        buffer: menu.buffer,
    }))
    .on_right_press(Message::Act(Action::Escape {
        buffer: menu.buffer,
    }))
    .into()
}

/// The panel down the left: places at the top, jobs at the bottom.
fn sidebar<'a>(app: &'a App, index: usize) -> Element<'a, Message> {
    use iced::widget::{mouse_area, scrollable};

    let theme = &app.config.theme;
    let dim = theme.dim.color();
    let foreground = theme.foreground.color();
    let accent = theme.accent.color();

    let heading = |what: &'a str| {
        container(text(what).size(11).color(dim))
            .padding([8, 10])
            .width(Length::Fill)
    };

    // The line that says where a dragged place would land.
    //
    // The two pixels are always there and only the colour changes. Pushing a
    // line in mid-drag moved the row out from under the pointer, which made
    // the pointer leave it, which took the line away again, which moved the
    // row back: a drop target that flickered and could not be hit.
    let marker = move |lit: bool| {
        container(iced::widget::Space::new().width(Length::Fill).height(2)).style(
            move |_: &iced::Theme| container::Style {
                background: lit.then(|| accent.into()),
                ..container::Style::default()
            },
        )
    };

    let mut list = column![].width(Length::Fill);
    let mut previous = None;

    for (at, place) in app.places.iter().enumerate() {
        // Bookmarks get a heading of their own. The ones above are what the
        // system says is here; these are what a person put there, and one
        // list of both made it look as though home could be removed too.
        if place.kind == places::Kind::Bookmark && previous != Some(places::Kind::Bookmark) {
            list = list.push(iced::widget::Space::new().height(10));
            list = list.push(heading("BOOKMARKS"));
        } else if previous.is_some_and(|kind| kind != place.kind) {
            // A gap between the other groups, so the panel reads as a short
            // list of short lists rather than one long one.
            list = list.push(iced::widget::Space::new().height(6));
        }
        previous = Some(place.kind);

        list = list.push(marker(lands_here(app, at)));

        // Drawn, not a `button`. `mouse_area` gives its content the event
        // first and gives up if it was captured, and `button` captures a
        // press -- so with a button in here the `on_press` below would never
        // fire and a place could not be picked up. Everything the button did
        // is done by hand: the hover colour, the hand cursor, and opening the
        // place, which now happens on release because until the button comes
        // up this may still turn into a drag.
        let hovered = app.over_place == Some(at);
        let row = container(text(place.label.clone()).size(13).color(if hovered {
            iced::Color::WHITE
        } else {
            foreground
        }))
        .width(Length::Fill)
        .padding([3, 10]);

        // `mouse_area`'s right press does not say where it happened. The
        // tracked pointer answers that: it is the same position, one event
        // earlier.
        list = list.push(
            mouse_area(row)
                .interaction(iced::mouse::Interaction::Pointer)
                .on_press(Message::PlacePressed {
                    place: at,
                    buffer: index,
                })
                .on_enter(Message::PlaceEntered(at))
                .on_exit(Message::PlaceLeft(at))
                .on_right_press(Message::Act(Action::Menu {
                    kind: MenuKind::Place { index: at },
                    buffer: index,
                    at: app.pointer,
                })),
        );
    }

    // The strip under the last place. It is how a bookmark is moved to the
    // end, which "drop in front of the one you are over" cannot express, and
    // it is always there rather than appearing mid-drag: a target that shows
    // up only once you are dragging is a target nobody finds.
    let end = app.places.len();
    let tail = column![
        marker(lands_here(app, end)),
        iced::widget::Space::new().width(Length::Fill).height(40),
    ]
    .width(Length::Fill);

    list = list.push(
        mouse_area(tail)
            .on_enter(Message::PlaceEntered(end))
            .on_exit(Message::PlaceLeft(end)),
    );

    let mut jobs = column![heading("JOBS")].width(Length::Fill);

    if app.jobs.is_empty() {
        jobs = jobs.push(container(text("nothing running").size(12).color(dim)).padding([0, 10]));
    }

    for job in app.jobs.iter() {
        // A bar behind the words rather than beside them: 180 pixels is not
        // enough for both, and the words are the part that says what failed.
        let filled = job.fraction().clamp(0.0, 1.0);
        let bar = container(
            iced::widget::Space::new()
                .width(Length::FillPortion((filled * 1000.0) as u16 + 1))
                .height(2),
        )
        .style(move |_: &iced::Theme| container::Style {
            background: Some(accent.into()),
            ..container::Style::default()
        });
        let rest = iced::widget::Space::new()
            .width(Length::FillPortion(((1.0 - filled) * 1000.0) as u16 + 1))
            .height(2);

        let mut buttons = row![].spacing(4);
        if job.state.running() {
            let held = job.state == jobs::State::Paused;
            buttons = buttons.push(small(
                theme,
                if held { "go" } else { "hold" },
                if held {
                    Message::Act(Action::ResumeJob { job: job.id })
                } else {
                    Message::Act(Action::PauseJob { job: job.id })
                },
            ));
        }
        buttons = buttons.push(small(
            theme,
            if job.state.over() { "clear" } else { "stop" },
            if job.state.over() {
                Message::Act(Action::DismissJob { job: job.id })
            } else {
                Message::Act(Action::CancelJob { job: job.id })
            },
        ));

        jobs = jobs.push(
            container(column![
                row![
                    text(job.describe())
                        .size(12)
                        .color(foreground)
                        .width(Length::Fill),
                    buttons,
                ]
                .align_y(iced::Alignment::Center),
                row![bar, rest],
            ])
            .padding([2, 10]),
        );
    }

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
    // A gutter down the left edge. Without it the first letter of every
    // place sits against the side of the window, which reads as a fault
    // rather than as a margin.
    .padding(iced::Padding {
        left: 6.0,
        ..iced::Padding::ZERO
    })
    .style(move |_: &iced::Theme| container::Style {
        background: Some(theme.background.color().into()),
        ..container::Style::default()
    })
    .into()
}

/// What a job in this buffer would act on.
///
/// What is selected, or the row the cursor is on when nothing is -- which is
/// what every file manager does, and it is why pressing a key with nothing
/// selected still works. `None` means there is nothing at all, which is worth
/// saying rather than doing quietly.
fn chosen(app: &App, buffer: usize) -> Option<Vec<PathBuf>> {
    let found = app.buffers.get(buffer)?;

    let mut paths: Vec<PathBuf> = found.selected().map(|entry| entry.path.clone()).collect();
    if paths.is_empty() {
        paths.extend(found.at(found.cursor).map(|entry| entry.path.clone()));
    }

    (!paths.is_empty()).then_some(paths)
}

/// The directory the next tile is showing, and a short name for it.
///
/// `None` when there is only one tile, or when the next one is showing the
/// same directory: copying a thing on top of itself is not an offer.
fn beside(app: &App, buffer: usize) -> Option<(PathBuf, String)> {
    let here = app.buffers.get(buffer)?;

    // One window today, and the next tile round from this one. When a second
    // window arrives, "beside" has to mean the same window as the menu.
    let tiles = app.windows.values().next()?;
    let order: Vec<usize> = tiles.panes.iter().map(|(_, index)| *index).collect();
    let at = order.iter().position(|index| *index == buffer)?;
    let next = *order.get((at + 1) % order.len())?;

    let there = app.buffers.get(next)?;
    if there.path == here.path {
        return None;
    }

    let name = there.path.file_name().map_or_else(
        || there.path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Some((there.path.clone(), name))
}

/// A small word that acts, for the job rows.
///
/// A word rather than a glyph: the panel is 180 pixels wide and "hold" is
/// unambiguous where a pause symbol beside a stop symbol is two guesses.
fn small<'a>(
    theme: &crate::config::Theme,
    what: &'a str,
    message: Message,
) -> Element<'a, Message> {
    use iced::widget::button;

    let dim = theme.dim.color();
    button(text(what).size(11))
        .padding([0, 4])
        .style(move |_: &iced::Theme, status| button::Style {
            background: None,
            text_color: if matches!(status, button::Status::Hovered) {
                iced::Color::WHITE
            } else {
                dim
            },
            ..button::Style::default()
        })
        .on_press(message)
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
            icon_font: None,
            menu: None,
            pointer: (0.0, 0.0),
            size: iced::Size::new(1100.0, 700.0),
            places: Vec::new(),
            sidebar: true,
            jobs: jobs::Queue::new(2),
            tick: 0,
            dragging: None,
            modifiers: iced::keyboard::Modifiers::default(),
            flourish: flourish::State::default(),
            pressed: None,
            over_place: None,
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

    /// A drag carrying two tiles, ready to be dropped.
    ///
    /// Buffer 0 shows `/tmp/one` and holds a directory called `pictures`;
    /// buffer 1 shows `/tmp/two`, which is where the drag comes from.
    fn dragging(over: Option<(usize, Option<usize>)>) -> App {
        let mut app = app();
        let start = Config::default().list.view();
        app.buffers
            .push(Buffer::new(PathBuf::from("/tmp/two"), start));
        // Directories sort first, so `pictures` is row 0 and `notes.txt` is
        // row 1 -- which is what the drop tests below name.
        let list = Config::default().list;
        app.buffers[0].extend(
            vec![
                entry("pictures", Kind::Directory),
                entry("notes.txt", Kind::File),
            ],
            &list,
        );
        app.buffers[0].finish(&list);

        app.dragging = Some(Dragging {
            paths: vec![PathBuf::from("/tmp/two/holiday.jpg")],
            source: Source::List,
            from: Some(1),
            over,
        });
        app
    }

    /// Dropped on a directory, the drop goes inside it. Dropped anywhere
    /// else in the tile it goes into the directory the tile is showing.
    #[test]
    fn where_a_drop_lands_is_the_row_or_the_tile() {
        // On `pictures`, which is a directory.
        let mut app = dragging(Some((0, Some(0))));
        tell(&mut app, Message::Released);
        let Some(Dialogue::Dropping { into, sources, .. }) = app.dialogue.clone() else {
            panic!("a drop with no modifier asks");
        };
        assert_eq!(into, PathBuf::from("/tmp/one/pictures"));
        assert_eq!(sources, [PathBuf::from("/tmp/two/holiday.jpg")]);

        // On `notes.txt`, which is not. A file cannot take a drop, so this
        // means the directory the tile is showing.
        let mut app = dragging(Some((0, Some(1))));
        tell(&mut app, Message::Released);
        let Some(Dialogue::Dropping { into, .. }) = app.dialogue.clone() else {
            panic!("a drop on a file still lands somewhere");
        };
        assert_eq!(into, PathBuf::from("/tmp/one"));

        // Past the last row.
        let mut app = dragging(Some((0, None)));
        tell(&mut app, Message::Released);
        let Some(Dialogue::Dropping { into, .. }) = app.dialogue else {
            panic!("empty space is the tile's own directory");
        };
        assert_eq!(into, PathBuf::from("/tmp/one"));
    }

    /// A modifier answers the question without it being asked. Read at the
    /// drop, because people reach for it after they start dragging.
    #[test]
    fn a_held_modifier_skips_the_question() {
        let mut app = dragging(Some((0, Some(0))));
        app.modifiers = iced::keyboard::Modifiers::CTRL;
        tell(&mut app, Message::Released);
        assert!(app.dialogue.is_none(), "Ctrl means copy, and asks nothing");
        assert_eq!(app.jobs.iter().count(), 1, "and a job was made");

        let mut app = dragging(Some((0, Some(0))));
        app.modifiers = iced::keyboard::Modifiers::SHIFT;
        tell(&mut app, Message::Released);
        assert!(app.dialogue.is_none(), "Shift means move");
        assert_eq!(app.jobs.iter().count(), 1);
    }

    /// Dropping something back where it already is would make a job that
    /// does nothing, and a panel row that reports success for it.
    #[test]
    fn a_drop_where_it_already_is_does_nothing() {
        let mut app = dragging(Some((0, None)));
        app.dragging.as_mut().expect("dragging").paths = vec![PathBuf::from("/tmp/one/notes.txt")];

        tell(&mut app, Message::Released);

        assert!(app.dialogue.is_none(), "nothing is asked");
        assert_eq!(app.jobs.iter().count(), 0, "and no job is made");
    }

    /// Reporting a drop target is not an action on the tile it passes over.
    ///
    /// Every other message from a list focuses its tile and goes through the
    /// registry. This one must not: hovering would take the keyboard off the
    /// tile the drag came from, and letting go would act on the wrong
    /// tile's selection.
    #[test]
    fn a_drop_target_is_not_an_action_on_the_tile() {
        let mut app = dragging(None);
        let was = app.buffers[0].cursor;

        tell(&mut app, Message::List(0, list::Action::Over(Some(1))));

        assert_eq!(app.buffers[0].cursor, was, "the cursor did not move");
        assert!(
            app.buffers[0].selected().next().is_none(),
            "and nothing was selected"
        );
        assert_eq!(
            app.dragging.as_ref().and_then(|dragging| dragging.over),
            Some((0, Some(1))),
            "but the target was noted"
        );
    }

    /// Enter in the filter box opens what is highlighted, the way it does in
    /// every launcher. `text_input` captures Enter, so the list never sees it
    /// and cannot activate the row itself.
    #[test]
    fn enter_in_the_filter_box_opens_the_highlighted_row() {
        let mut app = app();
        let list = Config::default().list;
        app.buffers[0].extend(
            vec![
                entry("Documents", Kind::Directory),
                entry("Downloads", Kind::Directory),
            ],
            &list,
        );
        app.buffers[0].finish(&list);

        // Narrowed to `Downloads`, which is then the only row and the cursor.
        app.buffers[0].filtering = true;
        app.buffers[0].filter = String::from("Down");
        app.buffers[0].rebuild(&list);
        assert_eq!(app.buffers[0].rows(), 1);

        tell(&mut app, Message::FilterSubmitted(0));

        assert_eq!(
            app.buffers[0].path,
            PathBuf::from("/tmp/one/Downloads"),
            "the highlighted row, not the one that was first before filtering"
        );
        assert!(app.buffers[0].filter.is_empty(), "and the filter went");
        assert!(!app.buffers[0].filtering);
    }

    /// A filter that matches nothing has nothing to open, so Enter puts the
    /// box away rather than leaving a listing narrowed to nothing.
    #[test]
    fn enter_on_a_filter_that_matches_nothing_just_closes_it() {
        let mut app = app();
        let list = Config::default().list;
        app.buffers[0].extend(vec![entry("Documents", Kind::Directory)], &list);
        app.buffers[0].finish(&list);

        app.buffers[0].filtering = true;
        app.buffers[0].filter = String::from("nothing matches this");
        app.buffers[0].rebuild(&list);
        assert_eq!(app.buffers[0].rows(), 0);

        tell(&mut app, Message::FilterSubmitted(0));

        assert_eq!(app.buffers[0].path, PathBuf::from("/tmp/one"), "stayed put");
        assert!(!app.buffers[0].filtering);
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

        // Switched to something that is *not* the default, or the two tiles
        // would look the same and the test could not tell them apart.
        let was = app.buffers[0].view.layout;
        assert_ne!(was, Layout::Detail, "pick a layout that is not the default");

        tell(
            &mut app,
            Message::Act(Action::Layout {
                buffer: 1,
                layout: Layout::Detail,
            }),
        );

        assert_eq!(app.buffers[0].view.layout, was, "the other tile moved");
        assert_eq!(app.buffers[1].view.layout, Layout::Detail);

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
        app.buffers[0].filtering = true;
        app.menu = Some(Menu {
            kind: MenuKind::Context { row: Some(0) },
            buffer: 0,
            at: (0.0, 0.0),
        });

        tell(&mut app, Message::Act(Action::Escape { buffer: 0 }));
        assert!(app.menu.is_none(), "the menu went");
        assert!(app.buffers[0].filtering, "and the filter box stayed");
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
        let root = crate::testing::scratch("complete");
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
        app.buffers[0].typing_path = Some(String::from("/tmp/definitely-not-a-directory-here"));

        tell(&mut app, Message::PathSubmitted(0));

        assert_eq!(
            app.buffers[0].typing_path.as_deref(),
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

    /// Escape clears the filter before it closes the box.
    ///
    /// A filter typed by mistake is the common case, and one Escape that
    /// took the box away as well would mean opening it again to carry on.
    #[test]
    fn escape_clears_the_filter_then_closes_the_box() {
        let mut app = app();
        let list = Config::default().list;
        app.buffers[0].extend(
            vec![entry("alpha.txt", Kind::File), entry("beta.md", Kind::File)],
            &list,
        );
        app.buffers[0].finish(&list);

        app.buffers[0].filtering = true;
        app.buffers[0].filter = String::from("alpha");
        app.buffers[0].rebuild(&list);
        assert_eq!(app.buffers[0].rows(), 1);

        // First: the text goes, the box stays.
        tell(&mut app, Message::Act(Action::Escape { buffer: 0 }));
        assert!(app.buffers[0].filter.is_empty(), "the filter went");
        assert!(app.buffers[0].filtering, "but the box is still up");
        assert_eq!(app.buffers[0].rows(), 2, "and everything is showing");

        // Second: the box goes.
        tell(&mut app, Message::Act(Action::Escape { buffer: 0 }));
        tell(&mut app, Message::Filtering(0, false));
        assert!(!app.buffers[0].filtering);
    }

    /// An empty box has nothing left to lose, so one press takes it away.
    #[test]
    fn escape_closes_an_empty_filter_box_at_once() {
        let mut app = app();
        app.buffers[0].filtering = true;

        // `Action::Escape` answers with the message that closes it, so the
        // step being tested is that it did not stop to clear anything first.
        tell(&mut app, Message::Act(Action::Escape { buffer: 0 }));
        tell(&mut app, Message::Filtering(0, false));

        assert!(!app.buffers[0].filtering);
    }

    /// Escape leaves the text face. It arrives from the subscription rather
    /// than the list, because the list is not taking keys while the box is up.
    #[test]
    fn escape_leaves_the_path_text_face() {
        let mut app = app();
        let window = window::Id::unique();
        app.windows.insert(window, Tiles::new(0));
        app.buffers[0].typing_path = Some(String::from("/tmp/half-typed"));

        tell(&mut app, Message::Escaped(window));
        assert!(app.buffers[0].typing_path.is_none());
    }

    /// F9 turns the panel off and on again. A toggle that only worked one
    /// way would take the panel away and keep it.
    #[test]
    fn the_panel_goes_and_comes_back() {
        let mut app = app();
        assert!(app.sidebar, "it starts on show");

        tell(&mut app, Message::List(0, list::Action::Sidebar));
        assert!(!app.sidebar);

        tell(&mut app, Message::List(0, list::Action::Sidebar));
        assert!(app.sidebar);
    }

    /// Two buffers, so the per-tile state has somewhere to leak to.
    fn two_buffers() -> App {
        let mut app = app();
        app.buffers.push(Buffer::new(
            PathBuf::from("/tmp/two"),
            Config::default().list.view(),
        ));
        app
    }

    /// A half-typed path belongs to the directory, not to the window.
    ///
    /// It was one field on `App`. Splitting a tile while a path was half
    /// typed then handed the draft to whichever tile the keyboard moved to
    /// next, and coming back found the box gone.
    #[test]
    fn a_half_typed_path_stays_with_its_own_tile() {
        let mut app = two_buffers();

        tell(
            &mut app,
            Message::Act(Action::TypingPath {
                buffer: 0,
                typing: true,
            }),
        );
        tell(&mut app, Message::PathTyped(0, String::from("/tmp/half")));

        assert_eq!(app.buffers[0].typing_path.as_deref(), Some("/tmp/half"));
        assert!(
            app.buffers[1].typing_path.is_none(),
            "the tile beside it is not typing anything"
        );
    }

    /// And Escape puts away the box in front of the person rather than the
    /// one in the tile beside it.
    #[test]
    fn escape_leaves_only_the_tile_that_asked() {
        let mut app = two_buffers();
        app.buffers[0].typing_path = Some(String::from("/tmp/one"));
        app.buffers[1].typing_path = Some(String::from("/tmp/two"));

        tell(&mut app, Message::Act(Action::Escape { buffer: 1 }));

        assert!(app.buffers[1].typing_path.is_none());
        assert_eq!(app.buffers[0].typing_path.as_deref(), Some("/tmp/one"));
    }

    /// The filter box is the same shape, and closing one used to clear every
    /// filter in the window -- which un-narrowed a listing nobody had asked
    /// about.
    #[test]
    fn closing_the_filter_box_clears_only_its_own_buffer() {
        let mut app = two_buffers();
        app.buffers[0].filtering = true;
        app.buffers[0].filter = String::from("one");
        app.buffers[1].filtering = true;
        app.buffers[1].filter = String::from("two");

        tell(&mut app, Message::Filtering(0, false));

        assert!(!app.buffers[0].filtering);
        assert!(app.buffers[0].filter.is_empty());
        assert!(app.buffers[1].filtering, "the other box stayed");
        assert_eq!(app.buffers[1].filter, "two", "and so did its text");
    }

    /// A tile split while a path is half typed gives the new tile a clean
    /// path bar. It shows a new directory; somebody else's draft is not what
    /// it should open with.
    #[test]
    fn a_new_tile_starts_with_breadcrumbs() {
        let mut app = app();
        app.windows.insert(window::Id::unique(), Tiles::new(0));
        app.buffers[0].typing_path = Some(String::from("/tmp/half"));

        tell(
            &mut app,
            Message::Act(Action::Split(pane_grid::Axis::Vertical)),
        );

        let made = app.buffers.last().expect("the split made a buffer");
        assert!(made.typing_path.is_none());
        assert!(!made.filtering);
        assert_eq!(
            app.buffers[0].typing_path.as_deref(),
            Some("/tmp/half"),
            "and the tile that was typing kept its draft"
        );
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

    /// Home, then two bookmarks. Enough to say what may be reordered and
    /// what may not.
    fn with_places() -> App {
        let mut app = app();
        app.places = vec![
            Place {
                label: String::from("Home"),
                path: PathBuf::from("/home/someone"),
                kind: places::Kind::Home,
            },
            Place {
                label: String::from("alpha"),
                path: PathBuf::from("/tmp/alpha"),
                kind: places::Kind::Bookmark,
            },
            Place {
                label: String::from("beta"),
                path: PathBuf::from("/tmp/beta"),
                kind: places::Kind::Bookmark,
            },
        ];
        app
    }

    /// The button going down on a place, in the one tile these tests have.
    const fn press(place: usize) -> Message {
        Message::PlacePressed { place, buffer: 0 }
    }

    /// A place opens on release, not on press. Until the button comes up the
    /// press may still turn into a drag, and opening it at the start of a
    /// drag would move the listing out from under the person dragging.
    #[test]
    fn a_place_opens_when_the_button_comes_up() {
        let mut app = with_places();
        let was = app.buffers[0].path.clone();

        tell(&mut app, press(1));
        assert_eq!(app.buffers[0].path, was, "not yet");

        tell(&mut app, Message::Released);
        assert_eq!(app.buffers[0].path, PathBuf::from("/tmp/alpha"));
    }

    /// A hand that shakes is still a click. Anything under the threshold
    /// opens the place, as it always did.
    #[test]
    fn a_small_wobble_is_still_a_click() {
        let mut app = with_places();

        tell(&mut app, press(1));
        tell(&mut app, Message::Pointer(DRAG - 1.0, 0.0));
        assert!(app.dragging.is_none(), "not far enough to be a drag");

        tell(&mut app, Message::Released);
        assert_eq!(app.buffers[0].path, PathBuf::from("/tmp/alpha"));
    }

    /// And past the threshold it is a drag, so the release must not also
    /// open the place. Doing both is how a reorder ends somewhere else.
    #[test]
    fn a_dragged_place_is_not_also_opened() {
        let mut app = with_places();
        let was = app.buffers[0].path.clone();

        tell(&mut app, press(1));
        tell(&mut app, Message::Pointer(0.0, DRAG + 1.0));

        let dragging = app.dragging.clone().expect("a drag started");
        assert_eq!(dragging.source, Source::Place);
        assert_eq!(dragging.paths, [PathBuf::from("/tmp/alpha")]);

        tell(&mut app, Message::Released);
        assert_eq!(app.buffers[0].path, was, "it was dragged, not clicked");
    }

    /// Home cannot be dragged anywhere -- but the press is still spent, so
    /// dragging off it does not open it either.
    #[test]
    fn a_place_that_is_not_a_bookmark_cannot_be_picked_up() {
        let mut app = with_places();
        let was = app.buffers[0].path.clone();

        tell(&mut app, press(0));
        tell(&mut app, Message::Pointer(0.0, DRAG + 1.0));
        assert!(app.dragging.is_none());

        tell(&mut app, Message::Released);
        assert_eq!(app.buffers[0].path, was);
    }

    /// Leaving one row and entering the next arrive in whichever order the
    /// widgets were built in. A clear that did not check which row it was
    /// about would lose the row just entered, and the marker with it.
    #[test]
    fn leaving_the_row_behind_does_not_clear_the_new_one() {
        let mut app = with_places();

        tell(&mut app, Message::PlaceEntered(2));
        tell(&mut app, Message::PlaceLeft(1));
        assert_eq!(app.over_place, Some(2));

        tell(&mut app, Message::PlaceLeft(2));
        assert_eq!(app.over_place, None);
    }

    /// Where a drop would land, over each kind of row.
    #[test]
    fn a_drop_lands_only_where_ricedir_keeps_an_order() {
        let places = with_places().places;
        let alpha = Path::new("/tmp/alpha");

        assert_eq!(
            lands(&places, 2, alpha),
            Lands::Before(PathBuf::from("/tmp/beta"))
        );

        // The strip below the last place, which is the only way to say "after
        // all of them".
        assert_eq!(lands(&places, places.len(), alpha), Lands::AtTheEnd);

        // Home is in the panel because the system says so.
        assert_eq!(lands(&places, 0, alpha), Lands::Nowhere);

        // And onto itself is not a move.
        assert_eq!(lands(&places, 1, alpha), Lands::Nowhere);
    }

    /// Resident memory, in bytes.
    ///
    /// Field two of `/proc/self/statm` is the resident page count. Only the
    /// difference between two readings is used, so the page size being taken
    /// as 4 KiB changes nothing but the units.
    fn resident() -> usize {
        std::fs::read_to_string("/proc/self/statm")
            .ok()
            .and_then(|statm| statm.split_whitespace().nth(1)?.parse::<usize>().ok())
            .unwrap_or(0)
            * 4096
    }

    /// Nothing `view` builds may be leaked, because `view` runs every frame.
    ///
    /// The breadcrumbs were: the first version made each path component a
    /// `&'static str` with `Box::leak`, which in a function that runs sixty
    /// times a second grows for as long as the window is open. That was
    /// found by reading the code back, which is not a way of finding
    /// anything. This measures instead.
    ///
    /// Two readings, both after the allocator has settled, so what is being
    /// compared is growth and not the first thousand frames warming up.
    #[test]
    fn building_the_window_does_not_leak() {
        const WARM: usize = 2_000;
        const FRAMES: usize = 40_000;
        /// One leaked path component per frame is about 30 bytes, so this
        /// catches a leak of a hundredth of that and still leaves room for
        /// the allocator to keep a page or two back.
        const ALLOWED: usize = 1 << 20;

        let mut app = with_places();
        let window = window::Id::unique();
        app.windows.insert(window, Tiles::new(0));

        // A deep path, because the breadcrumbs are one widget per component
        // and a leak there is proportional to how many there are.
        app.buffers[0].path = PathBuf::from("/home/someone/one/two/three/four/five");

        for _ in 0..WARM {
            drop(view(&app, window));
        }
        let before = resident();

        for _ in 0..FRAMES {
            drop(view(&app, window));
        }
        let grew = resident().saturating_sub(before);

        assert!(
            grew < ALLOWED,
            "{grew} bytes over {FRAMES} frames, which is {} a frame",
            grew / FRAMES
        );
    }

    /// A drag released away from the panel is called off, and nothing is
    /// written. This is how a drag is cancelled, so it has to be silent.
    #[test]
    fn a_drag_released_off_the_panel_does_nothing() {
        let mut app = with_places();

        tell(&mut app, press(1));
        tell(&mut app, Message::Pointer(300.0, 300.0));
        assert!(app.dragging.is_some());

        // Nowhere near a place: `over_place` is what the panel sets, and the
        // pointer never reached it.
        tell(&mut app, Message::Released);
        assert!(app.dragging.is_none());
        assert!(app.notice.is_none(), "cancelling is not a complaint");
    }
}
